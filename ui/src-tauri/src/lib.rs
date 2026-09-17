//! Verio Tauri bridge library.
//!
//! Phase 0 proves the frontend ↔ Rust bridge in both directions:
//! - `ping` command: frontend → Rust → frontend (with error path on empty name),
//! - `emit_test_event` command: Rust emits a Tauri event the frontend listens for.
//!
//! Real commands (`create_room`, `join_room`, `leave_room`, `toggle_mute`,
//! `toggle_deafen`, `set_input_device`, `set_output_device`, `set_per_peer_volume`,
//! `set_hotkey`, `update_settings`) and real events (`peer_joined`, `peer_left`,
//! `speaking_changed`, `connection_state_changed`, `rtt_updated`, `error`) land in
//! later phases.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use uuid::Uuid;

use verio_app::hotkeys::{HotkeyAction, TxState};
use verio_app::pipeline::{LiveControls, PipelineConfig, PipelineEvent, PipelineHandle};
use verio_app::room::{RoomEvent, RoomManager, RoomState};
use verio_app::settings::{Settings, SettingsStore};
use verio_app::AppState;
use verio_discovery::{DirectListener, Identity, MdnsAdvertiser};

/// Response payload for the `ping` round-trip proof.
#[derive(Debug, Serialize)]
pub struct PingResponse {
    message: String,
    rust_timestamp_ms: u64,
    app: verio_app::AppInfo,
}

/// Tauri command: echo a greeting back to the frontend, proving the
/// frontend → Rust → frontend command path (arguments in, Result out).
#[tauri::command]
fn ping(name: String) -> Result<PingResponse, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Name must not be empty".to_string());
    }

    let rust_timestamp_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    Ok(PingResponse {
        message: format!("Hello {trimmed}! The Rust ↔ frontend bridge is working."),
        rust_timestamp_ms,
        app: verio_app::app_info(),
    })
}

/// Event payload emitted from Rust to the frontend.
#[derive(Debug, Serialize, Clone)]
struct TestEvent {
    payload: String,
    at_ms: u64,
}

/// Tauri command: trigger a Rust → frontend event, proving the event path.
#[tauri::command]
fn emit_test_event(app: AppHandle, payload: String) -> Result<(), String> {
    let at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    app.emit("test_event", TestEvent { payload, at_ms })
        .map_err(|e| format!("failed to emit test_event: {e}"))
}

/// TEMP debug: surface webview-side diagnostics in the terminal log so
/// `npx tauri dev` shows them (devtools console is not visible there).
/// Remove once the hotkey-capture issue is resolved.
#[tauri::command]
fn debug_log(msg: String) {
    tracing::info!(target: "ui-debug", "{msg}");
}

// ---------------------------------------------------------------------------
// Phase 1: managed state
// ---------------------------------------------------------------------------

/// Managed bridge state. Everything the commands touch lives here; the audio
/// paths only ever share lock-free atomics ([`TxState`], [`LiveControls`]).
struct Bridge {
    settings: SettingsStore,
    tx: std::sync::Arc<TxState>,
    controls: std::sync::Arc<LiveControls>,
    pipeline: Mutex<Option<PipelineHandle>>,
    log_dir: PathBuf,
    /// Last speaking state received from the pipeline (for get_audio_state).
    speaking: std::sync::Arc<AtomicBool>,
    /// Phase 2: room session (single remote peer) + signaling listener + mDNS.
    room: Arc<Mutex<RoomManager>>,
    listener: Option<Arc<DirectListener>>,
    /// Keeps the mDNS daemon alive for the app lifetime (best-effort).
    _mdns: Option<MdnsAdvertiser>,
    /// Keeps the tracing file-log worker alive for the whole app lifetime.
    _log_guard: WorkerGuard,
    /// Session 13: music bus (shared with the audio pipeline).
    music: std::sync::Arc<verio_app::pipeline::MusicSource>,
}

/// Snapshot of mute/deafen/PTT/speaking state for the UI.
#[derive(Debug, Serialize, Clone)]
struct AudioState {
    muted: bool,
    deafened: bool,
    ptt_mode: bool,
    ptt_held: bool,
    speaking: bool,
    loopback: bool,
}

/// A device entry for the pickers.
#[derive(Debug, Serialize, Clone)]
struct DeviceEntry {
    name: String,
    is_default: bool,
}

#[derive(Debug, Serialize, Clone)]
struct DeviceList {
    inputs: Vec<DeviceEntry>,
    outputs: Vec<DeviceEntry>,
}

fn emit_audio_state(app: &AppHandle, bridge: &Bridge) {
    let s = bridge.settings.get();
    let state = AudioState {
        muted: bridge.tx.is_muted(),
        deafened: bridge.tx.is_deafened(),
        ptt_mode: bridge.tx.is_ptt_mode(),
        ptt_held: bridge.tx.is_ptt_held(),
        speaking: bridge.speaking.load(Ordering::Relaxed),
        loopback: s.loopback,
    };
    if let Err(e) = app.emit("audio_state_changed", state) {
        tracing::error!("emit audio_state_changed failed: {e}");
    }
}

/// Start (or restart) the audio pipeline from current settings.
fn restart_pipeline(app: &AppHandle, bridge: &Bridge) {
    let mut guard = bridge.pipeline.lock().expect("pipeline lock");
    if let Some(handle) = guard.take() {
        handle.shutdown();
    }

    let s = bridge.settings.get();
    bridge.tx.set_ptt_mode(!s.hotkeys.ptt.is_empty());
    // Phase 2 wiring: incoming network audio gets a fresh channel per
    // pipeline start (the RX thread owns the receiver); outgoing frames go
    // through the room manager's permanent sender.
    let (net_in_tx, net_in_rx) = std::sync::mpsc::channel();
    {
        let room = bridge.room.lock().expect("room lock");
        room.set_net_in(Some(net_in_tx));
        let net_out_tx = room.pipeline_endpoints();
        let cfg = PipelineConfig {
            input_device: s.input_device.clone(),
            output_device: s.output_device.clone(),
            input_gain_db: s.input_gain_db,
            noise_suppression: s.noise_suppression,
            loopback: s.loopback,
            debug_wavs: s.debug_wavs,
            log_dir: bridge.log_dir.clone(),
            net_out_tx: Some(net_out_tx),
            net_in_rx: Some(net_in_rx),
            music: std::sync::Arc::clone(&bridge.music),
        };

        match verio_app::pipeline::start(cfg, bridge.tx.clone(), bridge.controls.clone()) {
            Ok((event_rx, handle)) => {
                let forwarder_app = app.clone();
                let forwarder_speaking = bridge.speaking.clone();
                let forwarder_tx = Arc::clone(&bridge.tx);
                let forwarder_room = Arc::clone(&bridge.room);
                let spawned = std::thread::Builder::new()
                    .name("verio-event-forwarder".into())
                    .spawn(move || {
                        forward_events(
                            forwarder_app,
                            event_rx,
                            forwarder_speaking,
                            forwarder_tx,
                            forwarder_room,
                        );
                    });
                if let Err(e) = spawned {
                    tracing::error!("spawn event forwarder failed: {e}");
                }
                *guard = Some(handle);
            }
            Err(e) => {
                tracing::error!("pipeline start failed: {e}");
                let _ = app.emit("audio_error", e);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Settings / audio commands
// ---------------------------------------------------------------------------

#[tauri::command]
fn get_settings(bridge: State<Bridge>) -> Settings {
    bridge.settings.get()
}

#[tauri::command]
fn list_devices() -> Result<DeviceList, String> {
    let to_entry =
        |d: verio_audio::devices::DeviceInfo| DeviceEntry { name: d.name, is_default: d.is_default };
    let inputs = verio_audio::devices::list_input_devices()?
        .into_iter()
        .map(to_entry)
        .collect();
    let outputs = verio_audio::devices::list_output_devices()?
        .into_iter()
        .map(to_entry)
        .collect();
    Ok(DeviceList { inputs, outputs })
}

#[tauri::command]
fn get_audio_state(bridge: State<Bridge>) -> AudioState {
    let s = bridge.settings.get();
    AudioState {
        muted: bridge.tx.is_muted(),
        deafened: bridge.tx.is_deafened(),
        ptt_mode: bridge.tx.is_ptt_mode(),
        ptt_held: bridge.tx.is_ptt_held(),
        speaking: bridge.speaking.load(Ordering::Relaxed),
        loopback: s.loopback,
    }
}

#[tauri::command]
fn set_input_device(
    name: Option<String>,
    app: AppHandle,
    bridge: State<Bridge>,
) -> Result<(), String> {
    bridge.settings.update(|s| s.input_device = name.clone());
    tracing::info!(device = ?name, "input device changed — restarting pipeline");
    restart_pipeline(&app, &bridge);
    Ok(())
}

#[tauri::command]
fn set_output_device(
    name: Option<String>,
    app: AppHandle,
    bridge: State<Bridge>,
) -> Result<(), String> {
    bridge.settings.update(|s| s.output_device = name.clone());
    tracing::info!(device = ?name, "output device changed — restarting pipeline");
    restart_pipeline(&app, &bridge);
    Ok(())
}

#[tauri::command]
// Session 14: the UI sends `gainDb`, so the parameter must be named `gain_db`
// (Tauri maps camelCase JS keys onto snake_case Rust arguments). It used to be
// `db`, which made every invoke fail and the slider snap back.
fn set_input_gain(gain_db: f64, bridge: State<Bridge>) -> Result<(), String> {
    let gain_db = gain_db.clamp(0.0, 30.0) as f32;
    bridge.settings.update(|s| s.input_gain_db = gain_db);
    bridge.controls.set_gain_db(gain_db);
    Ok(())
}

/// Session 14: voice encoder bitrate (kbps) for this client.
#[tauri::command]
fn set_voice_quality(kbps: u32, bridge: State<Bridge>) -> Result<(), String> {
    bridge.settings.update(|s| s.voice_bitrate_kbps = kbps);
    bridge.controls.set_voice_kbps(kbps);
    Ok(())
}

/// Session 14: encoder bitrate (kbps) used while music is playing.
#[tauri::command]
fn set_music_quality(kbps: u32, bridge: State<Bridge>) -> Result<(), String> {
    bridge.settings.update(|s| s.music_bitrate_kbps = kbps);
    bridge.controls.set_music_kbps(kbps);
    Ok(())
}

#[tauri::command]
fn set_noise_suppression(on: bool, bridge: State<Bridge>) -> Result<(), String> {
    bridge.settings.update(|s| s.noise_suppression = on);
    bridge.controls.set_noise_suppression(on);
    Ok(())
}

#[tauri::command]
fn set_loopback(on: bool, app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    bridge.settings.update(|s| s.loopback = on);
    tracing::info!(loopback = on, "loopback toggled — restarting pipeline");
    restart_pipeline(&app, &bridge);
    emit_audio_state(&app, &bridge);
    Ok(())
}

#[tauri::command]
fn set_debug_wavs(on: bool, app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    bridge.settings.update(|s| s.debug_wavs = on);
    tracing::info!(debug_wavs = on, "debug WAVs toggled — restarting pipeline");
    restart_pipeline(&app, &bridge);
    Ok(())
}

#[tauri::command]
fn set_nickname(
    name: Option<String>,
    nickname: Option<String>,
    bridge: State<Bridge>,
) -> Result<(), String> {
    let raw = name.or(nickname).unwrap_or_default();
    let trimmed = raw.trim().to_string();
    tracing::info!(nickname = %trimmed, "updating nickname");
    bridge.settings.update(|s| s.nickname = trimmed);
    // Session 14: tell the room right away so the new name shows live everywhere.
    bridge.room.lock().expect("room lock").send_hello();
    Ok(())
}

/// Called when the UI enters hotkey-capture mode: unregisters the action's
/// current global binding so pressing the old combo mid-capture cannot fire
/// the action. Returns the previous binding so Escape can restore it.
#[tauri::command]
fn prepare_hotkey_capture(
    action: String,
    app: AppHandle,
    bridge: State<Bridge>,
) -> Result<String, String> {
    let hotkeys = bridge.settings.get().hotkeys;
    let old = match action.as_str() {
        "mute" => hotkeys.mute,
        "deafen" => hotkeys.deafen,
        "ptt" => hotkeys.ptt,
        other => return Err(format!("unknown hotkey action: {other}")),
    };
    if let Ok(sc) = parse_binding(&old) {
        app.global_shortcut().unregister(sc).ok();
        tracing::info!(action = %action, binding = %old, "hotkey unregistered for capture");
    }
    Ok(old)
}

#[tauri::command]
fn set_hotkey(
    action: Option<String>,
    field: Option<String>,
    combo: String,
    app: AppHandle,
    bridge: State<Bridge>,
) -> Result<(), String> {
    let target = action.or(field).unwrap_or_default();
    let combo = normalize_combo(&combo);
    tracing::info!(target = %target, combo = %combo, "set_hotkey requested");
    let field = match target.as_str() {
        "mute" => "mute",
        "deafen" => "deafen",
        "ptt" => "ptt",
        other => return Err(format!("unknown hotkey action: {other}")),
    };
    // Validate BEFORE saving so a bad combo never sticks.
    if !combo.is_empty() {
        parse_binding(&combo)?;
    }
    let hotkeys = bridge.settings.get().hotkeys;
    // The current binding of `field` plus the two bindings it must not collide
    // with (a combo may only ever be bound to one action).
    let (old_combo, other_bindings): (&str, [(&str, &str); 2]) = match field {
        "mute" => (&hotkeys.mute, [("deafen", &hotkeys.deafen), ("ptt", &hotkeys.ptt)]),
        "deafen" => (&hotkeys.deafen, [("mute", &hotkeys.mute), ("ptt", &hotkeys.ptt)]),
        _ => (&hotkeys.ptt, [("mute", &hotkeys.mute), ("deafen", &hotkeys.deafen)]),
    };
    if !combo.is_empty() {
        for (other, bound) in other_bindings {
            if bound == combo {
                return Err(format!(
                    "hotkey {combo:?} is already bound to {other:?} — pick another"
                ));
            }
        }
    }

    let gs = app.global_shortcut();
    // Unregister the old binding first (idempotent if it was already gone).
    if let Ok(sc) = parse_binding(old_combo) {
        gs.unregister(sc).ok();
        tracing::info!(binding = %old_combo, "hotkey unregistered");
    }
    if !combo.is_empty() {
        let sc = parse_binding(&combo)?;
        gs.unregister(sc).ok(); // stale registration from a previous run, if any
        let action_enum = match field {
            "mute" => HotkeyAction::Mute,
            "deafen" => HotkeyAction::Deafen,
            _ => HotkeyAction::Ptt,
        };
        gs.on_shortcut(sc, move |app, _s, event| {
            handle_shortcut_event(app, action_enum, event.state());
        })
        .map_err(|e| format!("register hotkey {combo:?}: {e}"))?;
        // Verify with the plugin so a silently-failed registration is an
        // error instead of a binding that exists only in settings.
        if !gs.is_registered(sc) {
            return Err(format!("hotkey {combo:?} did not register (taken by another app?)"));
        }
        tracing::info!(binding = %combo, action = %field, "hotkey registered");
    }

    // Persist only after the registration path succeeded.
    bridge.settings.update(|s| match field {
        "mute" => s.hotkeys.mute = combo.clone(),
        "deafen" => s.hotkeys.deafen = combo.clone(),
        _ => s.hotkeys.ptt = combo.clone(),
    });

    // PTT binding cleared ⇔ voice-activity mode.
    let updated = bridge.settings.get();
    bridge.tx.set_ptt_mode(!updated.hotkeys.ptt.is_empty());
    emit_audio_state(&app, &bridge);
    Ok(())
}

// ---------------------------------------------------------------------------
// Session 13: music player commands
// ---------------------------------------------------------------------------

/// Push the current music status to the UI right away.
fn emit_music_status(app: &AppHandle, bridge: &Bridge) {
    let _ = app.emit("music_status_changed", bridge.music.status());
}

#[tauri::command]
fn music_status(bridge: State<Bridge>) -> verio_app::pipeline::MusicStatus {
    bridge.music.status()
}

#[tauri::command]
fn music_open(
    path: String,
    app: AppHandle,
    bridge: State<Bridge>,
) -> Result<verio_app::pipeline::MusicInfo, String> {
    let info = bridge.music.open(std::path::Path::new(&path))?;
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
    Ok(info)
}

#[tauri::command]
fn music_play(app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    bridge.music.play()?;
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
    Ok(())
}

#[tauri::command]
fn music_pause(app: AppHandle, bridge: State<Bridge>) {
    bridge.music.pause();
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
}

#[tauri::command]
fn music_stop(app: AppHandle, bridge: State<Bridge>) {
    bridge.music.stop();
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
}

#[tauri::command]
fn music_seek(ms: u64, app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    bridge.music.seek(ms)?;
    emit_music_status(&app, &bridge);
    Ok(())
}

#[tauri::command]
fn music_set_volume(v: f32, app: AppHandle, bridge: State<Bridge>) {
    bridge.music.set_volume(v);
    bridge.settings.update(|s| s.music_volume = v.clamp(0.0, 2.0));
    emit_music_status(&app, &bridge);
}

#[tauri::command]
fn music_set_loop(on: bool, app: AppHandle, bridge: State<Bridge>) {
    bridge.music.set_loop(on);
    bridge.settings.update(|s| s.music_loop = on);
    emit_music_status(&app, &bridge);
}

#[tauri::command]
fn music_set_monitor(on: bool, app: AppHandle, bridge: State<Bridge>) {
    bridge.music.set_monitor(on);
    bridge.settings.update(|s| s.music_monitor = on);
    emit_music_status(&app, &bridge);
}

/// Native picker, starting in the folder used last time.
#[tauri::command]
fn music_pick_file(bridge: State<Bridge>) -> Option<String> {
    let mut dialog = rfd::FileDialog::new()
        .add_filter("Audio", &["mp3", "flac", "wav", "ogg"])
        .set_title("Open audio file");
    if let Some(dir) = bridge.settings.get().music_last_dir {
        dialog = dialog.set_directory(dir);
    }
    let picked = dialog.pick_file()?;
    if let Some(parent) = picked.parent() {
        let dir = parent.display().to_string();
        bridge.settings.update(|s| s.music_last_dir = Some(dir));
    }
    Some(picked.display().to_string())
}

/// Play an earcon through the live pipeline (no-op when the pipeline is stopped).
fn play_cue(app: &tauri::AppHandle, kind: verio_app::EarconKind) {
    let bridge = tauri::Manager::state::<Bridge>(app);
    let pipeline = bridge.pipeline.lock().expect("pipeline lock");
    if let Some(pipe) = pipeline.as_ref() {
        pipe.play_earcon(kind);
    }
}

/// Push local speaking/mute state to the connected peer (no-op when idle).
/// Called on every mute/deafen/PTT transition and speaking change.
fn send_room_state(bridge: &Bridge) {
    let speaking = bridge.speaking.load(Ordering::Relaxed);
    let muted = bridge.tx.is_muted();
    let deafened = bridge.tx.is_deafened();
    // Session 14: carry music presence so peers see who is playing what.
    let status = bridge.music.status();
    let music = status.active;
    let title = if music {
        status.title.clone().unwrap_or_default()
    } else {
        String::new()
    };
    bridge
        .room
        .lock()
        .expect("room lock")
        .send_state(speaking, muted, deafened, music, title);
}

#[tauri::command]
fn toggle_mute(app: AppHandle, bridge: State<Bridge>) -> AudioState {
    bridge.tx.toggle_mute();
    let is_muted = bridge.tx.is_muted();
    if let Some(pipe) = bridge.pipeline.lock().expect("pipeline lock").as_ref() {
        pipe.play_earcon(if is_muted {
            verio_app::EarconKind::MuteOn
        } else {
            verio_app::EarconKind::MuteOff
        });
    }
    send_room_state(&bridge);
    emit_audio_state(&app, &bridge);
    get_audio_state(bridge)
}

#[tauri::command]
fn toggle_deafen(app: AppHandle, bridge: State<Bridge>) -> AudioState {
    bridge.tx.toggle_deafen();
    let is_deafened = bridge.tx.is_deafened();
    if let Some(pipe) = bridge.pipeline.lock().expect("pipeline lock").as_ref() {
        if is_deafened {
            pipe.play_earcon(verio_app::EarconKind::Deafen);
        } else {
            pipe.play_earcon(verio_app::EarconKind::MuteOff);
        }
    }
    send_room_state(&bridge);
    emit_audio_state(&app, &bridge);
    get_audio_state(bridge)
}

// ---------------------------------------------------------------------------
// Phase 2 / Phase 4: room / networking commands
// ---------------------------------------------------------------------------

/// Snapshot for the Connect panel (direct code, room state, peer name, 4-digit code).
#[derive(Debug, Serialize, Clone)]
struct RoomSnapshot {
    state: RoomState,
    peer_name: Option<String>,
    peers: Vec<verio_app::room::PeerInfo>,
    room_code: Option<String>,
    direct_codes: Vec<String>,
    signaling_port: u16,
    vps_address: String,
    transport_mode: Option<String>,
    diagnostics: Option<verio_transport::TransportDiagnostics>,
}

#[tauri::command]
fn get_room_state(bridge: State<Bridge>) -> RoomSnapshot {
    let room = bridge.room.lock().expect("room lock");
    let settings = bridge.settings.get();
    RoomSnapshot {
        state: room.state(),
        peer_name: room.peer_name().map(str::to_string),
        peers: room.peers(),
        room_code: room.room_code().map(str::to_string),
        direct_codes: bridge
            .listener
            .as_ref()
            .map(|l| l.local_codes())
            .unwrap_or_default(),
        signaling_port: settings.signaling_port,
        vps_address: settings.vps_address,
        transport_mode: room.transport_mode().map(str::to_string),
        diagnostics: room.diagnostics(),
    }
}

#[tauri::command]
fn get_transport_diagnostics(bridge: State<Bridge>) -> Option<verio_transport::TransportDiagnostics> {
    bridge.room.lock().expect("room lock").diagnostics()
}

#[tauri::command]
async fn create_room(bridge: State<'_, Bridge>) -> Result<String, String> {
    let room = Arc::clone(&bridge.room);
    let s = bridge.settings.get();
    let vps_addr = s.vps_address;
    let transport_mode = s.transport_mode;
    let host_port = s.host_port;
    tauri::async_runtime::spawn_blocking(move || {
        RoomManager::run_create_room(&room, vps_addr, transport_mode, host_port)
    })
    .await
    .map_err(|e| format!("create room task: {e}"))?
}

#[tauri::command]
async fn join_room(code: String, bridge: State<'_, Bridge>) -> Result<(), String> {
    let room = Arc::clone(&bridge.room);
    let s = bridge.settings.get();
    let vps_addr = s.vps_address;
    let transport_mode = s.transport_mode;
    tauri::async_runtime::spawn_blocking(move || {
        RoomManager::run_join_room(&room, vps_addr, code, transport_mode)
    })
    .await
    .map_err(|e| format!("join room task: {e}"))?
}

#[tauri::command]
fn leave_room(bridge: State<Bridge>) -> Result<(), String> {
    bridge.room.lock().expect("room lock").disconnect();
    Ok(())
}

#[tauri::command]
fn set_peer_volume(peer_id: String, volume: f32, bridge: State<Bridge>) -> Result<(), String> {
    if let Some(pipe) = bridge.pipeline.lock().expect("pipeline lock").as_ref() {
        pipe.set_peer_volume(peer_id, volume);
    }
    Ok(())
}

#[tauri::command]
fn set_vps_address(address: String, bridge: State<Bridge>) -> Result<(), String> {
    let address = address.trim().to_string();
    if address.is_empty() {
        return Err("VPS address cannot be empty".into());
    }
    bridge.settings.update(|s| s.vps_address = address);
    Ok(())
}

#[tauri::command]
fn connect_direct(addr: String, bridge: State<Bridge>) -> Result<(), String> {
    let addr = addr.trim().to_string();
    if addr.is_empty() {
        return Err("direct code is empty".into());
    }
    bridge.settings.update(|s| s.remember_direct_addr(&addr));
    let room = Arc::clone(&bridge.room);
    std::thread::Builder::new()
        .name("verio-connect".into())
        .spawn(move || RoomManager::run_connect_direct(&room, addr))
        .map_err(|e| format!("spawn connect worker: {e}"))?;
    Ok(())
}

#[tauri::command]
async fn create_invite(bridge: State<'_, Bridge>) -> Result<String, String> {
    let room = Arc::clone(&bridge.room);
    tauri::async_runtime::spawn_blocking(move || RoomManager::run_create_invite(&room))
        .await
        .map_err(|e| format!("invite task: {e}"))?
}

#[tauri::command]
async fn accept_invite(blob: String, bridge: State<'_, Bridge>) -> Result<String, String> {
    let room = Arc::clone(&bridge.room);
    tauri::async_runtime::spawn_blocking(move || RoomManager::run_accept_invite(&room, blob))
        .await
        .map_err(|e| format!("invite task: {e}"))?
}

#[tauri::command]
async fn complete_invite(blob: String, bridge: State<'_, Bridge>) -> Result<(), String> {
    let room = Arc::clone(&bridge.room);
    tauri::async_runtime::spawn_blocking(move || RoomManager::run_complete_invite(&room, blob))
        .await
        .map_err(|e| format!("invite task: {e}"))?
}

#[tauri::command]
fn disconnect_peer(bridge: State<Bridge>) -> Result<(), String> {
    bridge.room.lock().expect("room lock").disconnect();
    Ok(())
}

#[tauri::command]
fn set_signaling_port(port: u16, bridge: State<Bridge>) -> Result<(), String> {
    if port == 0 {
        return Err("port must be 1–65535".into());
    }
    bridge.settings.update(|s| s.signaling_port = port);
    tracing::warn!(port, "signaling port changed — takes effect at next launch");
    Ok(())
}

#[tauri::command]
fn set_transport_mode(mode: String, bridge: State<Bridge>) -> Result<(), String> {
    use verio_app::settings::TransportModeSetting;
    let setting = match mode.to_ascii_lowercase().as_str() {
        "auto" => TransportModeSetting::Auto,
        "cloud_relay" | "cloudrelay" | "cloud" => TransportModeSetting::CloudRelay,
        "peer_host" | "peerhost" | "host" => TransportModeSetting::PeerHost,
        "direct_p2p" | "directp2p" | "direct" | "p2p" => TransportModeSetting::DirectP2P,
        other => return Err(format!("unknown transport mode: {other}")),
    };
    bridge.settings.update(|s| s.transport_mode = setting);
    tracing::info!(?setting, "transport mode updated in settings");
    Ok(())
}

#[tauri::command]
fn set_host_port(port: u16, bridge: State<Bridge>) -> Result<(), String> {
    if port == 0 {
        return Err("host port must be 1–65535".into());
    }
    bridge.settings.update(|s| s.host_port = port);
    tracing::info!(port, "host port updated in settings");
    Ok(())
}

// ---------------------------------------------------------------------------
// App bootstrap
// ---------------------------------------------------------------------------

/// CLI launch arguments (parsed manually — no clap): `--profile <name>` for
/// multi-instance (separate config/log dirs, window title suffix, independent
/// peer uuid) and the dev/test hook `--connect <ip:port>` to dial a direct
/// code at startup (documented deviation, used for automated two-instance
/// verification; equally usable manually).
#[derive(Debug, Default)]
pub struct LaunchArgs {
    pub profile: Option<String>,
    pub connect: Option<String>,
}

fn parse_launch_args() -> LaunchArgs {
    let mut args = LaunchArgs::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--profile" => args.profile = it.next().filter(|v| !v.trim().is_empty()),
            "--connect" => args.connect = it.next().filter(|v| !v.trim().is_empty()),
            other => tracing::warn!(arg = other, "unknown CLI argument ignored"),
        }
    }
    args
}

/// ONE-TIME migration for the identifier change (app.verio.desktop →
/// ir.verio.app). ALL config/log paths derive from the identifier, which is now
/// PERMANENT — it must never change again (it defines install/update identity).
fn migrate_legacy_identifier(new_config_dir: &Path, new_log_dir: &Path) {
    let legacy_config = std::env::var("APPDATA")
        .map(|d| PathBuf::from(d).join("app.verio.desktop"))
        .ok();
    let legacy_logs = std::env::var("LOCALAPPDATA")
        .map(|d| PathBuf::from(d).join("app.verio.desktop").join("logs"))
        .ok();

    if let Some(legacy) = legacy_config {
        let legacy_settings = legacy.join("settings.json");
        let settings_file = new_config_dir.join("settings.json");
        if !settings_file.exists() && legacy_settings.exists() {
            match std::fs::copy(&legacy_settings, &settings_file) {
                Ok(_) => tracing::info!(
                    from = %legacy_settings.display(),
                    to = %settings_file.display(),
                    "ONE-TIME migration: settings.json copied from legacy identifier app.verio.desktop"
                ),
                Err(e) => tracing::error!("settings migration failed: {e}"),
            }
        }
    }

    if let Some(legacy_logs) = legacy_logs {
        if legacy_logs.exists() && new_log_dir.exists() {
            let has_own_logs = std::fs::read_dir(new_log_dir)
                .map(|mut rd| {
                    rd.any(|e| {
                        e.map(|f| f.file_name().to_string_lossy().starts_with("verio.log"))
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false);
            if !has_own_logs {
                let mut copied = 0usize;
                if let Ok(entries) = std::fs::read_dir(&legacy_logs) {
                    for entry in entries.flatten() {
                        let Ok(meta) = entry.metadata() else { continue };
                        // Non-trivial only: skip empty files.
                        if meta.len() == 0 {
                            continue;
                        }
                        let name = entry.file_name();
                        if !name.to_string_lossy().starts_with("verio.log") {
                            continue;
                        }
                        let to = new_log_dir.join(&name);
                        if std::fs::copy(entry.path(), &to).is_ok() {
                            copied += 1;
                        }
                    }
                }
                if copied > 0 {
                    tracing::info!(
                        from = %legacy_logs.display(),
                        files = copied,
                        "ONE-TIME migration: legacy logs copied from app.verio.desktop"
                    );
                }
            }
        }
    }
}

// __SETUP__

/// Build and run the Tauri application.
pub fn run() {
    let launch = parse_launch_args();
    tauri::Builder::default()

        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            ping,
            emit_test_event,
            debug_log,
            get_settings,
            set_nickname,
            list_devices,
            get_audio_state,
            set_input_device,
            set_output_device,
            set_input_gain,
            set_noise_suppression,
            set_loopback,
            set_debug_wavs,
            set_hotkey,
            prepare_hotkey_capture,
            toggle_mute,
            toggle_deafen,
            get_room_state,
            create_room,
            join_room,
            leave_room,
            set_peer_volume,
            set_vps_address,
            connect_direct,
            create_invite,
            accept_invite,
            complete_invite,
            disconnect_peer,
            set_signaling_port,
            get_transport_diagnostics,
            set_transport_mode,
            set_host_port,
            music_status,
            music_open,
            music_play,
            music_pause,
            music_stop,
            music_seek,
            music_set_volume,
            music_set_loop,
            music_set_monitor,
            music_pick_file,
            set_voice_quality,
            set_music_quality,
        ])
        .setup(move |app| {
            let launch: LaunchArgs = launch;
            // --- dirs (profile-aware: --profile <name> → subdirectories) ---
            let log_dir = app
                .path()
                .app_log_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("verio-logs"));
            let config_dir = app
                .path()
                .app_config_dir()
                .unwrap_or_else(|_| std::env::temp_dir().join("verio-config"));
            // ONE-TIME legacy identifier migration (default profile only).
            migrate_legacy_identifier(&config_dir, &log_dir);
            let (log_dir, config_dir) = match &launch.profile {
                Some(p) => {
                    let ld = log_dir.join("profiles").join(p);
                    let cd = config_dir.join("profiles").join(p);
                    std::fs::create_dir_all(&ld)
                        .map_err(|e| format!("create log dir {}: {e}", ld.display()))?;
                    std::fs::create_dir_all(&cd)
                        .map_err(|e| format!("create config dir {}: {e}", cd.display()))?;
                    (ld, cd)
                }
                None => (log_dir, config_dir),
            };
            if let Some(p) = &launch.profile {
                tracing::info!(profile = %p, "multi-instance profile active");
            }
            let file_appender = tracing_appender::rolling::daily(&log_dir, "verio.log");
            let (writer, guard) = tracing_appender::non_blocking(file_appender);
            tracing_subscriber::registry()
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_writer(std::io::stdout)
                        .with_target(false),
                )
                .with(tracing_subscriber::fmt::layer().with_writer(writer))
                .with(if std::env::var("RUST_LOG")
                    .map(|v| {
                        let v = v.to_ascii_lowercase();
                        v.contains("debug") || v.contains("trace")
                    })
                    .unwrap_or(false)
                {
                    tracing_subscriber::filter::LevelFilter::DEBUG
                } else {
                    tracing_subscriber::filter::LevelFilter::INFO
                })
                .init();
            tracing::info!(log_dir = %log_dir.display(), "logging initialised");

            // --- settings (corrupt file → defaults + warning, inside load) ---
            let settings = SettingsStore::load(&config_dir);
            let settings_snapshot = settings.get();
            tracing::info!(settings = ?settings_snapshot, "settings loaded");
            tracing::info!(
                "relay packet log is {} (launch with RUST_LOG=debug to capture per-packet relay lines)",
                if std::env::var("RUST_LOG")
                    .map(|v| {
                        let v = v.to_ascii_lowercase();
                        v.contains("debug") || v.contains("trace")
                    })
                    .unwrap_or(false)
                {
                    "ON"
                } else {
                    "OFF"
                }
            );

            // Peer identity: generate a fresh UUID v4 for each running process
            // so that multiple instances launched on the same PC (e.g. for testing)
            // never collide with each other in signaling rooms or UDP relays.
            let peer_uuid = Uuid::new_v4();
            let identity = Identity {
                uuid: peer_uuid,
                name: if settings_snapshot.nickname.is_empty() {
                    "Player".to_string()
                } else {
                    settings_snapshot.nickname.clone()
                },
                version: verio_app::APP_VERSION.to_string(),
            };
            tracing::info!(peer_uuid = %peer_uuid, name = %identity.name, "peer identity");
            let (room_tx, room_rx) = std::sync::mpsc::channel::<RoomEvent>();
            let room = Arc::new(Mutex::new(RoomManager::new(identity.clone(), room_tx)));
            room.lock().expect("room lock").spawn_audio_pump();

            // Signaling listener. Best-effort: on one PC the SECOND instance
            // loses the port race and acts as connector only.
            let listener = match DirectListener::bind(settings_snapshot.signaling_port) {
                Ok(l) => {
                    let l = Arc::new(l);
                    let accept_listener = Arc::clone(&l);
                    let listen_room = Arc::clone(&room);
                    let spawned = std::thread::Builder::new()
                        .name("verio-signaling-listen".into())
                        .spawn(move || loop {
                            match accept_listener.accept() {
                                Ok(session) => {
                                    let accept_room = Arc::clone(&listen_room);
                                    let worker = std::thread::Builder::new()
                                        .name("verio-direct-accept".into())
                                        .spawn(move || {
                                            RoomManager::run_accept_direct(&accept_room, session)
                                        });
                                    if worker.is_err() {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("signaling accept failed: {e}");
                                    break;
                                }
                            }
                        });
                    if let Err(e) = spawned {
                        tracing::error!("spawn signaling listener thread failed: {e}");
                    }
                    Some(l)
                }
                Err(e) => {
                    tracing::warn!(
                        port = settings_snapshot.signaling_port,
                        "signaling port unavailable ({e}) — this instance can only connect outward (KNOWN ISSUE: same-host mDNS/5353 analog; direct-connect covers testing)"
                    );
                    None
                }
            };

            // mDNS — DISCOVERY ONLY, best-effort. If the second instance cannot
            // bind UDP 5353, log it and continue (cross-machine mDNS is
            // validated when a 2nd PC exists).
            let mdns = match MdnsAdvertiser::start(&identity, settings_snapshot.signaling_port) {
                Ok(a) => Some(a),
                Err(e) => {
                    tracing::warn!("mDNS advertisement unavailable: {e} — continuing (direct-connect covers testing)");
                    None
                }
            };

            // --- bridge state (pipeline is started/restarted via helper) ---
            let bridge = Bridge {
                settings,
                tx: std::sync::Arc::new(TxState::new()),
                controls: std::sync::Arc::new(LiveControls::new(
                    settings_snapshot.input_gain_db,
                    settings_snapshot.noise_suppression,
                    settings_snapshot.voice_bitrate_kbps,
                    settings_snapshot.music_bitrate_kbps,
                )),
                pipeline: Mutex::new(None),
                log_dir: log_dir.clone(),
                speaking: std::sync::Arc::new(AtomicBool::new(false)),
                music: std::sync::Arc::new(verio_app::pipeline::MusicSource::new(
                    settings_snapshot.music_volume,
                    settings_snapshot.music_loop,
                    settings_snapshot.music_monitor,
                )),
                room,
                listener,
                _mdns: mdns,
                _log_guard: guard,
            };

            // Session 13: push music status to the UI (~4 Hz while playing, and
            // immediately on any state transition).
            {
                let tick_app = app.handle().clone();
                let tick_music = std::sync::Arc::clone(&bridge.music);
                let spawned = std::thread::Builder::new()
                    .name("verio-music-status".into())
                    .spawn(move || {
                        let mut last = String::new();
                        loop {
                            std::thread::sleep(std::time::Duration::from_millis(250));
                            let status = tick_music.status();
                            let json = serde_json::to_string(&status).unwrap_or_default();
                            if status.playing || json != last {
                                last = json;
                                let _ = tick_app.emit("music_status_changed", status.clone());
                            }
                        }
                    });
                if let Err(e) = spawned {
                    tracing::error!("spawn music status thread failed: {e}");
                }
            }
            app.manage(bridge);
            let bridge: State<Bridge> = app.state();

            // --- room events → UI events ---
            {
                let fwd_app = app.handle().clone();
                let spawned = std::thread::Builder::new()
                    .name("verio-room-forwarder".into())
                    .spawn(move || {
                        while let Ok(ev) = room_rx.recv() {
                            match ev {
                                RoomEvent::StateChanged { state } => {
                                    let _ = fwd_app.emit("room_state", state);
                                }
                                RoomEvent::RoomCreated { code } => {
                                    let _ = fwd_app.emit("room_created", serde_json::json!({ "code": code }));
                                }
                                RoomEvent::RoomJoined { code } => {
                                    let _ = fwd_app.emit("room_joined", serde_json::json!({ "code": code }));
                                }
                                RoomEvent::PeerConnected { name, uuid } => {
                                    play_cue(&fwd_app, verio_app::EarconKind::PeerJoin);
                                    let _ = fwd_app.emit(
                                        "peer_connected",
                                        serde_json::json!({ "name": name, "uuid": uuid }),
                                    );
                                }
                                RoomEvent::PeerDisconnected { reason, peer_id } => {
                                    play_cue(&fwd_app, verio_app::EarconKind::PeerLeave);
                                    let _ = fwd_app.emit(
                                        "peer_disconnected",
                                        serde_json::json!({ "reason": reason, "peer_id": peer_id }),
                                    );
                                }
                                RoomEvent::PeerState {
                                    peer_id,
                                    speaking,
                                    mute,
                                    deafen,
                                    music,
                                    music_title,
                                } => {
                                    let _ = fwd_app.emit(
                                        "peer_state",
                                        serde_json::json!({
                                            "peer_id": peer_id,
                                            "speaking": speaking,
                                            "mute": mute,
                                            "deafen": deafen,
                                            "music": music,
                                            "music_title": music_title,
                                        }),
                                    );
                                }
                                RoomEvent::TransportModeChanged { mode } => {
                                    let _ = fwd_app.emit(
                                        "transport_mode_changed",
                                        serde_json::json!({ "mode": mode }),
                                    );
                                }
                                RoomEvent::Rtt { ms } => {
                                    let _ = fwd_app.emit(
                                        "rtt_updated",
                                        serde_json::json!({ "ms": ms }),
                                    );
                                }
                                RoomEvent::Error { message } => {
                                    let _ = fwd_app.emit("room_error", message);
                                }
                            }
                        }
                    });
                if let Err(e) = spawned {
                    tracing::error!("spawn room forwarder failed: {e}");
                }
            }

            restart_pipeline(app.handle(), &bridge);
            if let Err(e) = register_hotkeys(app.handle()) {
                // Non-fatal: the app still runs; UI shows hotkeys as unbound.
                tracing::error!("hotkey registration failed at startup: {e}");
                let _ = app.emit("audio_error", e);
            }

            // Multi-instance: window title suffix " — <profile>".
            if let Some(p) = &launch.profile {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.set_title(&format!("Verio — {p}"));
                }
            }

            // Dev/test hook: dial a direct code at startup.
            if let Some(addr) = launch.connect.clone() {
                let room = Arc::clone(&bridge.room);
                tracing::info!(%addr, "--connect: auto-connecting (dev/test hook)");
                std::thread::Builder::new()
                    .name("verio-connect".into())
                    .spawn(move || RoomManager::run_connect_direct(&room, addr))
                    .ok();
            }

            Ok(())
        })

        .run(tauri::generate_context!())
        .expect("Verio failed to start");
}

/// Drain pipeline events → UI events. Runs on its own (non-audio) thread.
fn forward_events(
    app: AppHandle,
    event_rx: std::sync::mpsc::Receiver<PipelineEvent>,
    speaking: std::sync::Arc<AtomicBool>,
    tx: std::sync::Arc<TxState>,
    room: Arc<Mutex<RoomManager>>,
) {
    use std::sync::mpsc::RecvTimeoutError;
    loop {
        match event_rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(event) => match event {
                PipelineEvent::InputLevel(db) => {
                    let _ = app.emit("input_level", f64::from(db));
                }
                PipelineEvent::SpeakingChanged(is_speaking) => {
                    let effective_speaking = is_speaking && tx.effective_transmission();
                    speaking.store(effective_speaking, Ordering::Relaxed);
                    let _ = app.emit("speaking_changed", effective_speaking);
                    // Spec: state{speaking,mute} JSON on the control channel
                    // drives the remote speaking ring.
                    // Session 14: music presence rides along with the speaking state.
                    let music_status = tauri::Manager::state::<Bridge>(&app).music.status();
                    let music = music_status.active;
                    let music_title = if music {
                        music_status.title.unwrap_or_default()
                    } else {
                        String::new()
                    };
                    room.lock().expect("room lock").send_state(
                        effective_speaking,
                        tx.is_muted(),
                        tx.is_deafened(),
                        music,
                        music_title,
                    );
                }
                PipelineEvent::WavFinished { path, is_loopback } => {
                    let _ = app.emit(
                        "wav_finished",
                        serde_json::json!({
                            "path": path.display().to_string(),
                            "is_loopback": is_loopback
                        }),
                    );
                }
                PipelineEvent::Failed(e) => {
                    tracing::error!("pipeline event: {e}");
                    let _ = app.emit("audio_error", e);
                }
            },
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

/// Map a global-shortcut press/release to its TxState action.
fn handle_shortcut_event(app: &AppHandle, action: HotkeyAction, state: ShortcutState) {
    let bridge: State<Bridge> = app.state();
    let pressed = state == ShortcutState::Pressed;
    match action {
        HotkeyAction::Mute => {
            if pressed {
                let muted = bridge.tx.toggle_mute();
                tracing::info!(muted, "hotkey: mute toggle");
                send_room_state(&bridge);
                emit_audio_state(app, &bridge);
            }
        }
        HotkeyAction::Deafen => {
            if pressed {
                let (deafened, muted) = bridge.tx.toggle_deafen();
                tracing::info!(deafened, muted, "hotkey: deafen toggle");
                send_room_state(&bridge);
                emit_audio_state(app, &bridge);
            }
        }
        HotkeyAction::Ptt => {
            bridge.tx.set_ptt_held(pressed);
            send_room_state(&bridge);
            emit_audio_state(app, &bridge);
        }
    }
}

fn normalize_combo(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = raw.split('+').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
    let mut normalized_parts = Vec::new();
    for part in parts {
        let lower = part.to_ascii_lowercase();
        match lower.as_str() {
            "ctrl" | "control" => normalized_parts.push("Ctrl".to_string()),
            "shift" => normalized_parts.push("Shift".to_string()),
            "alt" => normalized_parts.push("Alt".to_string()),
            "super" | "win" | "cmd" | "command" => normalized_parts.push("Super".to_string()),
            "space" => normalized_parts.push("Space".to_string()),
            s if s.len() == 1 => normalized_parts.push(s.to_ascii_uppercase()),
            other => {
                let mut c = other.chars();
                match c.next() {
                    None => {}
                    Some(f) => normalized_parts.push(f.to_uppercase().collect::<String>() + c.as_str()),
                }
            }
        }
    }
    normalized_parts.join("+")
}

/// Parse a stored binding string ("Ctrl+Shift+M", "B", "Shift+B", etc.) into a shortcut.
fn parse_binding(combo: &str) -> Result<Shortcut, String> {
    let norm = normalize_combo(combo);
    if norm.is_empty() {
        return Err("empty hotkey binding".to_string());
    }
    norm
        .parse::<Shortcut>()
        .map_err(|e| format!("invalid hotkey binding {combo:?} (normalized: {norm:?}): {e}"))
}

/// (Re)register all hotkeys from settings.
fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
    let bridge: State<Bridge> = app.state();
    let hotkeys = bridge.settings.get().hotkeys;
    let gs = app.global_shortcut();

    let bindings = [
        (HotkeyAction::Mute, hotkeys.mute),
        (HotkeyAction::Deafen, hotkeys.deafen),
        (HotkeyAction::Ptt, hotkeys.ptt),
    ];
    for (action, combo) in bindings {
        if combo.is_empty() {
            tracing::info!(action = ?action, "hotkey disabled (empty binding)");
            continue;
        }
        let shortcut = parse_binding(&combo)?;
        gs.unregister(shortcut).ok(); // idempotent if not registered
        gs.on_shortcut(shortcut, move |app, _sc, event| {
            handle_shortcut_event(app, action, event.state());
        })
        .map_err(|e| format!("register hotkey {combo:?}: {e}"))?;
        tracing::info!(action = ?action, binding = %combo, "hotkey registered");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_rejects_empty_name() {
        assert!(ping("   ".to_string()).is_err());
    }

    #[test]
    fn test_shortcut_parsing() {
        assert!(parse_binding("Ctrl+Shift+M").is_ok(), "Ctrl+Shift+M failed");
        assert!(parse_binding("Shift+B").is_ok(), "Shift+B failed");
        assert!(parse_binding("Ctrl+B").is_ok(), "Ctrl+B failed");
        assert!(parse_binding("Space").is_ok(), "Space failed");
        assert!(parse_binding("B").is_ok(), "B failed");
    }
}
