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
use tauri_plugin_global_shortcut::ShortcutState;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use verio_app::db::Db;
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
    /// Session 29b: the running music-bot session, if this client is hosting.
    ///
    /// `None` is IDLE and `Some` is HOSTING — the whole host state machine, with no
    /// second flag to fall out of step with it.
    bot: std::sync::Arc<Mutex<Option<verio_app::music_bot::MusicBot>>>,
    /// Session 29b: where the bot pushes the host's own copy of the music.
    ///
    /// A slot rather than a sender because a pipeline restart replaces the mixer's
    /// monitor channel — the same reason the room takes its mute gate as a sink.
    /// Without this, changing the output device while hosting would silently end
    /// the host's local monitor.
    bot_monitor: std::sync::Arc<Mutex<Option<verio_app::pipeline::MonitorSink>>>,
    /// Guards concurrent `music_bot_start` calls so two bots cannot spawn.
    bot_starting: AtomicBool,
    /// Session 31: non-blocking hardware hotkey state.
    hotkeys: Arc<Mutex<HotkeyPollerState>>,
}

/// Session 31: hardware hotkey binding for non-blocking polling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyBinding {
    pub vk: i32,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
}

#[derive(Debug, Default)]
pub struct HotkeyPollerState {
    pub mute: Option<HotkeyBinding>,
    pub deafen: Option<HotkeyBinding>,
    pub ptt: Option<HotkeyBinding>,
    pub paused: bool,
    pub seed_edges: bool,
}

/// Session 29b: whether this client is hosting the music bot.
#[derive(Debug, Serialize, Clone)]
struct BotState {
    hosting: bool,
    /// The bot session's id. `None` when idle.
    session_id: Option<String>,
    /// The person this bot belongs to. Equal to our own nickname when we host.
    host_name: Option<String>,
}

/// Session 29b: the state the UI needs to draw Play/Stop and the host badge.
fn bot_state(bridge: &Bridge) -> BotState {
    let guard = bridge.bot.lock().expect("bot lock");
    match guard.as_ref() {
        Some(bot) => BotState {
            hosting: true,
            session_id: Some(bot.session_id().to_string()),
            host_name: Some(bridge.settings.get().nickname),
        },
        None => BotState {
            hosting: false,
            session_id: None,
            host_name: None,
        },
    }
}

fn emit_bot_state(app: &AppHandle, bridge: &Bridge) {
    if let Err(e) = app.emit("music_bot_changed", bot_state(bridge)) {
        tracing::error!("emit music_bot_changed failed: {e}");
    }
}

/// Session 29b: end the bot session and give the music bus back to the pipeline.
///
/// Idempotent, because four different things end a session: the host's Stop, the
/// queue running out, another peer's stop request, and the host leaving the room.
fn stop_bot(app: &AppHandle, bridge: &Bridge) {
    let bot = bridge.bot.lock().expect("bot lock").take();
    if let Some(bot) = bot {
        tracing::info!(session = %bot.session_id(), "music bot: stopping");
        bot.stop();
        // Session 29c fix: stop the player *with* the session, and before the bus
        // goes back. Handing the bus back while the player was still `playing` let
        // the voice pipeline take it and mix the track into the outgoing voice
        // frame and the local monitor — so a bot that ended left the music audibly
        // running with no tile in the room to stop it, and leaving the room left the
        // host hearing their own music with no call in progress.
        //
        // Only when a bot was actually running: a listener who never hosted keeps
        // their own local player across a leave.
        bridge.music.stop();
    }
    // Handed back unconditionally: leaving the bus owned by a bot that no longer
    // exists would leave the voice session deaf to its own music player, and the
    // player would look broken with no error anywhere.
    bridge.music.set_bot_owned(false);
    emit_bot_state(app, bridge);
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
        let mut room = bridge.room.lock().expect("room lock");
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
                // Session 29: hand the room the new mixer's mute gate. This is the
                // only moment the room and a pipeline exist at once, and it is why
                // the room takes a sink rather than holding a `PipelineHandle`: the
                // room is built first, before any pipeline does.
                room.set_mute_sink(handle.peer_mutes_handle());
                // Session 29b: a restart builds a new mixer, and therefore a new
                // monitor channel. Re-point the bot's slot at it, or changing the
                // output device while hosting would silently end the host's own
                // copy of the music — audible, but with nothing anywhere saying why.
                *bridge.bot_monitor.lock().expect("bot monitor lock") =
                    Some(handle.music_monitor_handle());
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
    let gain_db = gain_db.clamp(-12.0, 12.0) as f32;
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
    bridge.settings.update(|s| s.nickname = trimmed.clone());
    // Session 14 + 32A: update RoomManager's live identity and re-announce immediately.
    bridge
        .room
        .lock()
        .expect("room lock")
        .set_local_nickname(trimmed);
    Ok(())
}

fn hotkey_binding_for_action<'a>(
    hotkeys: &'a verio_app::settings::HotkeySettings,
    action: &str,
) -> Result<&'a str, String> {
    match action {
        "mute" => Ok(hotkeys.mute.as_str()),
        "deafen" => Ok(hotkeys.deafen.as_str()),
        "ptt" => Ok(hotkeys.ptt.as_str()),
        other => Err(format!("unknown hotkey action: {other}")),
    }
}

/// Called when the UI enters hotkey-capture mode: pauses the hotkey poller
/// so pressing keys mid-capture cannot fire actions.
#[tauri::command]
fn prepare_hotkey_capture(
    action: String,
    bridge: State<Bridge>,
) -> Result<String, String> {
    let hotkeys = bridge.settings.get().hotkeys;
    // Validate action BEFORE pausing the poller so invalid input cannot wedge hotkeys.
    let old = hotkey_binding_for_action(&hotkeys, action.as_str())?.to_string();
    bridge.hotkeys.lock().expect("hotkeys lock").paused = true;
    tracing::info!(action = %action, binding = %old, "hotkey poller paused for capture");
    Ok(old)
}

/// Called when the user cancels hotkey capture: unpauses the poller and seeds edges.
#[tauri::command]
fn cancel_hotkey_capture(bridge: State<Bridge>) {
    let mut hp = bridge.hotkeys.lock().expect("hotkeys lock");
    hp.paused = false;
    hp.seed_edges = true;
    tracing::info!("hotkey poller unpaused");
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
    let new_binding = if !combo.is_empty() {
        Some(parse_binding(&combo)?)
    } else {
        None
    };
    let hotkeys = bridge.settings.get().hotkeys;
    // The current binding of `field` plus the two bindings it must not collide
    // with (a combo may only ever be bound to one action).
    let other_bindings: [(&str, &str); 2] = match field {
        "mute" => [("deafen", &hotkeys.deafen), ("ptt", &hotkeys.ptt)],
        "deafen" => [("mute", &hotkeys.mute), ("ptt", &hotkeys.ptt)],
        _ => [("mute", &hotkeys.mute), ("deafen", &hotkeys.deafen)],
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

    // Update poller bindings, unpause, and seed live key edges
    {
        let mut hp = bridge.hotkeys.lock().expect("hotkeys lock");
        hp.paused = false;
        hp.seed_edges = true;
        match field {
            "mute" => hp.mute = new_binding,
            "deafen" => hp.deafen = new_binding,
            _ => hp.ptt = new_binding,
        }
    }

    // Persist only after validation
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

/// Session 31: send an ephemeral in-call chat message.
#[tauri::command]
fn send_chat_message(text: String, bridge: State<Bridge>) -> Result<(), String> {
    bridge.room.lock().expect("room lock").send_chat(text)
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
    let in_room = matches!(
        bridge.room.lock().expect("room lock").state(),
        verio_app::room::RoomState::Connected
    );
    if in_room {
        // Reserve bot ownership before unpausing so the voice processing thread
        // never leaks pre-bot music audio into the mic stream while the bot starts.
        bridge.music.set_bot_owned(true);
    }
    if let Err(e) = bridge.music.play() {
        let has_bot = bridge.bot.lock().expect("bot lock").is_some();
        if !has_bot {
            bridge.music.set_bot_owned(false);
        }
        return Err(e);
    }
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
    if bridge.bot.lock().expect("bot lock").is_none() {
        bridge.music.set_bot_owned(false);
    }
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

/// Session 29b Force Stop: may other peers stop the music this user hosts?
///
/// A *setting*, not a live control. It is read at the moment the bot is spawned —
/// the plan's "applied each time the user becomes host" — so changing it while
/// hosting affects the next session, never the running one. That is deliberate: the
/// value is announced to the room when the bot joins, so a bot that changed its mind
/// mid-session would be refusing a stop it had told every peer it would accept.
///
/// Nothing is emitted. The switch that drives this belongs to the same client, so it
/// already knows; and the room is deliberately not told until the next bot joins.
#[tauri::command]
fn set_music_force_stop(enabled: bool, bridge: State<Bridge>) {
    bridge.settings.update(|s| s.music_force_stop = enabled);
}

/// Session 29b Force Stop: ask this room's music host to stop.
///
/// A listener's control. Whether the request is honoured is deliberately *not*
/// decided here: the host's bot refuses the frame outright when the host disabled
/// Force Stop, and the host is the only client that owns a bot. So this reports only
/// that the request left, which is all this client can honestly claim — the music
/// stopping is something the roster will show, not something this call can promise.
#[tauri::command]
fn request_music_stop(bridge: State<Bridge>) -> Result<(), String> {
    bridge.room.lock().expect("room lock").request_music_stop()
}

// ---------------------------------------------------------------------------
// Session 29b: the music bot session
// ---------------------------------------------------------------------------

/// A peer already hosting music in this room, if any.
///
/// Read from the roster rather than remembered locally, because the host may be
/// somebody else entirely: this client only knows that a bot is in the room, which
/// is exactly what it needs to refuse a second one.
fn room_bot_peer(room: &verio_app::room::RoomManager) -> Option<verio_app::room::PeerInfo> {
    room.peers().into_iter().find(|p| p.is_bot)
}

#[tauri::command]
fn music_bot_state(bridge: State<Bridge>) -> BotState {
    bot_state(&bridge)
}

struct BotStartGuard<'a>(&'a AtomicBool);

impl Drop for BotStartGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[must_use]
fn session_id_matches(current_id: Option<&str>, expected_session_id: &str) -> bool {
    current_id.is_some_and(|id| id == expected_session_id)
}

#[must_use]
fn watcher_owns_bot(
    current: Option<&verio_app::music_bot::MusicBot>,
    expected_session_id: &str,
) -> bool {
    session_id_matches(
        current.map(verio_app::music_bot::MusicBot::session_id),
        expected_session_id,
    )
}

/// Start hosting: open a second session, join the room as "Music", and route the
/// local music bus through it.
#[tauri::command]
fn music_bot_start(app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    if bridge
        .bot_starting
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err("Music bot is already starting.".to_string());
    }
    let _start_guard = BotStartGuard(&bridge.bot_starting);

    // Stop any previous local bot instance before starting a new one so no
    // duplicate bot threads remain, without rewinding active music playback.
    let prior_bot = bridge.bot.lock().expect("bot lock").take();
    if let Some(old) = prior_bot {
        old.stop();
        emit_bot_state(&app, &bridge);
    }

    // Everything read from the room in one lock, so the room cannot change between
    // the checks and the start.
    let (room_code, host_user_id, host_name, existing_bot) = {
        let room = bridge.room.lock().expect("room lock");
        if !matches!(room.state(), verio_app::room::RoomState::Connected) {
            return Err("Join a room before playing music into it.".into());
        }
        let code = room
            .room_code()
            .ok_or("This room has no code yet.")?
            .to_string();
        let identity = room.identity();
        (
            code,
            identity.user_id.clone(),
            identity.name.clone(),
            room_bot_peer(&room),
        )
    };

    // One bot per room. The distinction matters to the message: if the bot in the
    // roster is *ours*, the user needs telling they are already hosting; if it is
    // somebody else's, telling them to press play again would be wrong.
    if let Some(bot) = existing_bot {
        let mine = !host_user_id.is_empty() && bot.user_id == host_user_id;
        return Err(if mine {
            "You are already hosting the music for this room.".into()
        } else {
            "Music is already playing in this room.".to_string()
        });
    }

    if !bridge.music.is_active() {
        return Err("Press play before sending music to the room.".into());
    }

    // Hand the bus over *before* spawning `music_bot::start` so the voice pipeline
    // cannot pull chunks and mix them into mic frames during signaling/STUN setup.
    bridge.music.set_bot_owned(true);

    let (events_tx, events_rx) = std::sync::mpsc::channel();
    let settings = bridge.settings.get();
    // Session 29b Force Stop: read here, at the moment the user becomes host, and
    // fixed for the session. It travels inside `BotConfig` to both the announcement
    // and the enforcement, so the room is told exactly what will be enforced — and
    // toggling the setting later changes the *next* bot, never this one.
    let force_stop = settings.music_force_stop;
    let bot = match verio_app::music_bot::start(verio_app::music_bot::BotConfig {
        vps_addr: settings.vps_address,
        room_code,
        host_user_id,
        host_name,
        force_stop,
        music: Arc::clone(&bridge.music),
        controls: Arc::clone(&bridge.controls),
        tx_state: Arc::clone(&bridge.tx),
        monitor_tx: Arc::clone(&bridge.bot_monitor),
        events: events_tx,
    }) {
        Ok(bot) => bot,
        Err(e) => {
            bridge.music.set_bot_owned(false);
            return Err(e);
        }
    };

    let expected_session_id = bot.session_id().to_string();
    *bridge.bot.lock().expect("bot lock") = Some(bot);
    emit_bot_state(&app, &bridge);

    // Watch for the session ending by itself. Without this the app would keep
    // believing it was hosting after the queue ran out, and the UI would show a
    // Stop button for a session that no longer exists.
    let bot_slot = Arc::clone(&bridge.bot);
    let music = Arc::clone(&bridge.music);
    let watcher_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("verio-bot-watch".into())
        .spawn(move || {
            // A wait, not a loop. `BotEvent` has one variant and acting on it ends
            // the session, so every path through the body left — clippy's
            // `never_loop` caught the `while` for claiming otherwise.
            let Ok(ev) = events_rx.recv() else {
                tracing::debug!("music bot: watcher exiting with no event");
                return;
            };
            let verio_app::music_bot::BotEvent::Stopped { reason } = ev;
            let taken = {
                let mut guard = bot_slot.lock().expect("bot lock");
                if watcher_owns_bot(guard.as_ref(), &expected_session_id) {
                    guard.take()
                } else {
                    None
                }
            };
            if let Some(bot) = taken {
                bot.stop();
                // Session 29c fix: the player is stopped with the session so the
                // voice pipeline does not pick up playing audio when the bus returns.
                music.stop();
                music.set_bot_owned(false);
                let bridge = tauri::Manager::state::<Bridge>(&watcher_app);
                emit_bot_state(&watcher_app, &bridge);
                let _ = watcher_app.emit(
                    "music_bot_ended",
                    serde_json::json!({ "reason": reason }),
                );
            }
        });
    if let Err(e) = spawned {
        tracing::error!("spawn music bot watcher failed: {e}");
    }
    Ok(())
}

/// Stop hosting. Safe to call when not hosting.
#[tauri::command]
fn music_bot_stop(app: AppHandle, bridge: State<Bridge>) {
    stop_bot(&app, &bridge);
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

// ---------------------------------------------------------------------------
// Session 20: music queue commands
// ---------------------------------------------------------------------------

/// Payload of `music_queue_changed`.
#[derive(Clone, Serialize)]
struct QueueEvent {
    items: Vec<verio_app::pipeline::QueueItem>,
    index: Option<usize>,
}

/// Result of adding files, so the UI can report partial success honestly instead
/// of a bare "ok" that hides unreadable files.
#[derive(Clone, Serialize)]
struct QueueAddResult {
    added: usize,
    skipped: Vec<String>,
}

/// Push the queue contents to the UI. Emitted only when the queue actually
/// changes — the list can be long, and `music_status_changed` fires on every
/// fader movement.
fn emit_music_queue(app: &AppHandle, bridge: &Bridge) {
    let (items, index) = bridge.music.queue_snapshot();
    let _ = app.emit("music_queue_changed", QueueEvent { items, index });
}

#[tauri::command]
fn music_queue_list(bridge: State<Bridge>) -> Vec<verio_app::pipeline::QueueItem> {
    bridge.music.queue_snapshot().0
}

/// Resolve each path into a queue entry and append it. Container parsing happens
/// here, on the command thread; the audio thread never opens a file.
#[tauri::command]
fn music_queue_add(
    paths: Vec<String>,
    app: AppHandle,
    bridge: State<Bridge>,
) -> QueueAddResult {
    let mut items = Vec::with_capacity(paths.len());
    let mut skipped = Vec::new();
    for p in &paths {
        match verio_app::pipeline::MusicSource::describe_track(std::path::Path::new(p)) {
            Some(item) => items.push(item),
            None => skipped.push(p.clone()),
        }
    }
    let added = items.len();
    bridge.music.queue_append(items);
    emit_music_queue(&app, &bridge);
    emit_music_status(&app, &bridge);
    QueueAddResult { added, skipped }
}

#[tauri::command]
fn music_queue_remove(index: usize, app: AppHandle, bridge: State<Bridge>) {
    bridge.music.queue_remove(index);
    emit_music_queue(&app, &bridge);
    emit_music_status(&app, &bridge);
}

#[tauri::command]
fn music_queue_clear(app: AppHandle, bridge: State<Bridge>) {
    bridge.music.queue_clear();
    emit_music_queue(&app, &bridge);
    emit_music_status(&app, &bridge);
}

#[tauri::command]
fn music_queue_move(from: usize, to: usize, app: AppHandle, bridge: State<Bridge>) {
    bridge.music.queue_move(from, to);
    emit_music_queue(&app, &bridge);
}

#[tauri::command]
fn music_queue_set_loop(on: bool, app: AppHandle, bridge: State<Bridge>) {
    bridge.music.set_queue_looping(on);
    emit_music_status(&app, &bridge);
}

/// Load a queue entry. `play` is explicit so that building a queue can load its
/// first track paused — otherwise adding files would start broadcasting music
/// into the room before the user pressed anything.
#[tauri::command]
fn music_queue_jump(
    index: usize,
    play: bool,
    app: AppHandle,
    bridge: State<Bridge>,
) -> Result<(), String> {
    bridge.music.jump_to(index, play)?;
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
    Ok(())
}

/// Advance immediately. `request_next` only names the target; servicing it here
/// keeps the button instant, while the background task covers end-of-track.
#[tauri::command]
fn music_next(app: AppHandle, bridge: State<Bridge>) {
    bridge.music.request_next();
    bridge.music.service_advance();
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
}

#[tauri::command]
fn music_prev(app: AppHandle, bridge: State<Bridge>) {
    bridge.music.request_prev();
    bridge.music.service_advance();
    emit_music_status(&app, &bridge);
    send_room_state(&bridge);
}

/// Multi-select picker, starting in the folder used last time.
#[tauri::command]
fn music_pick_files(bridge: State<Bridge>) -> Option<Vec<String>> {
    let mut dialog = rfd::FileDialog::new()
        .add_filter("Audio", &["mp3", "flac", "wav", "ogg"])
        .set_title("Add audio files");
    if let Some(dir) = bridge.settings.get().music_last_dir {
        dialog = dialog.set_directory(dir);
    }
    let picked = dialog.pick_files()?;
    if let Some(first) = picked.first() {
        if let Some(parent) = first.parent() {
            let dir = parent.display().to_string();
            bridge.settings.update(|s| s.music_last_dir = Some(dir));
        }
    }
    Some(picked.iter().map(|p| p.display().to_string()).collect())
}

/// Play an earcon through the live pipeline (no-op when the pipeline is stopped).
fn play_cue(app: &tauri::AppHandle, kind: verio_app::EarconKind) {
    let bridge = tauri::Manager::state::<Bridge>(app);
    let pipeline = bridge.pipeline.lock().expect("pipeline lock");
    if let Some(pipe) = pipeline.as_ref() {
        pipe.play_earcon(kind);
    }
}

/// Session 29b: the music presence the **voice** peer should announce.
///
/// While a bot session owns the bus, the voice peer is no longer the one injecting
/// music — the Music peer is, and it announces its own. Reporting `true` from both
/// would put a music chip on both of one person's tiles and claim two streams where
/// there is one.
fn voice_music_presence(music: &verio_app::pipeline::MusicSource) -> (bool, String) {
    if music.is_bot_owned() {
        return (false, String::new());
    }
    let status = music.status();
    if status.active {
        (true, status.title.unwrap_or_default())
    } else {
        (false, String::new())
    }
}

/// Push local speaking/mute state to the connected peer (no-op when idle).
/// Called on every mute/deafen/PTT transition and speaking change.
fn send_room_state(bridge: &Bridge) {
    let speaking = bridge.speaking.load(Ordering::Relaxed);
    let muted = bridge.tx.is_muted();
    let deafened = bridge.tx.is_deafened();
    // Session 14: carry music presence so peers see who is playing what.
    // Session 29b: except while a bot session owns the music, which announces its
    // own presence as a peer in its own right.
    let (music, title) = voice_music_presence(&bridge.music);
    bridge
        .room
        .lock()
        .expect("room lock")
        .send_state(speaking, muted, deafened, music, title);
}

/// Play the cue for a mute/deafen transition.
///
/// Session 29c fix. The Tauri commands and the global shortcuts flip the *same*
/// state, so they have to make the same sound — and while this lived inline in the
/// two commands, a global shortcut toggled mute in silence while the button played
/// the cue. Two copies of a rule is how they drift; one function is how they cannot.
fn play_toggle_earcon(bridge: &Bridge, kind: verio_app::EarconKind) {
    if let Some(pipe) = bridge.pipeline.lock().expect("pipeline lock").as_ref() {
        pipe.play_earcon(kind);
    }
}

#[tauri::command]
fn toggle_mute(app: AppHandle, bridge: State<Bridge>) -> AudioState {
    bridge.tx.toggle_mute();
    let is_muted = bridge.tx.is_muted();
    play_toggle_earcon(
        &bridge,
        if is_muted {
            verio_app::EarconKind::MuteOn
        } else {
            verio_app::EarconKind::Unmute
        },
    );
    send_room_state(&bridge);
    emit_audio_state(&app, &bridge);
    get_audio_state(bridge)
}

#[tauri::command]
fn toggle_deafen(app: AppHandle, bridge: State<Bridge>) -> AudioState {
    bridge.tx.toggle_deafen();
    let is_deafened = bridge.tx.is_deafened();
    // `Undeafen`, not `Unmute`. Until Session 27 the deafen-off branch played the
    // mute-off cue, so deafen and undeafen were not the distinguishable pair they
    // read as — you could not tell by ear whether you had unmuted or undeafened.
    play_toggle_earcon(
        &bridge,
        if is_deafened {
            verio_app::EarconKind::Deafen
        } else {
            verio_app::EarconKind::Undeafen
        },
    );
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
    /// Session 29: the room-wide mutes, so a webview reload does not lose them.
    /// Kept beside `peers` rather than inside it — see [`MuteInfo`].
    mutes: Vec<verio_app::room::MuteInfo>,
    /// Session 29: this user's own persistent id, so the UI can tell whether one
    /// of `mutes` names them — which is the only case where it owes an
    /// explanation for being silent.
    self_user_id: String,
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
        mutes: room.mutes(),
        // The room holds the identity it was built with, so this needs no second
        // copy of the id anywhere in the bridge.
        self_user_id: room.identity().user_id.clone(),
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
fn leave_room(app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    // Session 29b: the bot session dies with the host's, by design — "if the host
    // disconnects, the bot session dies with them". Stopping it here rather than
    // letting it notice means the room returns to IDLE before we have even finished
    // leaving, so nobody is left looking at a Music tile for a host who has gone.
    // Session 32A: wait briefly for signaling/roster threads to send Leave and close
    // the WebSocket before a subsequent rejoin can race ahead.
    stop_bot(&app, &bridge);
    RoomManager::disconnect_and_wait(&bridge.room);
    Ok(())
}

#[tauri::command]
fn set_peer_volume(peer_id: String, volume: f32, bridge: State<Bridge>) -> Result<(), String> {
    // Logged because this is the only place a per-peer level is written and the
    // key has to match what the mixer calls that peer. When a tile's slider looks
    // right but the audio does not change, this id is the first thing to compare
    // against the mixer's own `peer_id`.
    tracing::info!(peer = %peer_id, volume = %volume, "set_peer_volume");
    if let Some(pipe) = bridge.pipeline.lock().expect("pipeline lock").as_ref() {
        pipe.set_peer_volume(peer_id, volume);
    } else {
        // Was a silent no-op: the tile's slider would move, the audio would not
        // change, and nothing anywhere said why. There is no pipeline to hold a
        // level when the app is not in a call, so this is a real possibility
        // rather than an impossible branch.
        tracing::warn!(peer = %peer_id, "set_peer_volume dropped: no pipeline");
    }
    Ok(())
}

/// Session 29: silence a peer for everyone in the room, or lift it.
///
/// Not the same thing as `set_peer_volume(.., 0.0)`, which is this user's own
/// listening level and nobody else's business. This one is room state: it is
/// broadcast, and it silences that peer for every other listener too.
///
/// `target_user_id` is the peer's *persistent* id, because the mute has to still
/// name them after they relaunch.
#[tauri::command]
fn set_peer_muted(
    target_user_id: String,
    muted: bool,
    bridge: State<Bridge>,
) -> Result<(), String> {
    bridge
        .room
        .lock()
        .expect("room lock")
        .set_room_mute(&target_user_id, muted)
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
fn disconnect_peer(app: AppHandle, bridge: State<Bridge>) -> Result<(), String> {
    stop_bot(&app, &bridge);
    RoomManager::disconnect_and_wait(&bridge.room);
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

#[tauri::command]
fn window_minimize(window: tauri::Window) -> Result<(), String> {
    window.minimize().map_err(|e| format!("minimize failed: {e}"))
}

#[tauri::command]
fn window_toggle_maximize(window: tauri::Window) -> Result<(), String> {
    let maximized = window
        .is_maximized()
        .map_err(|e| format!("is_maximized failed: {e}"))?;
    if maximized {
        window.unmaximize().map_err(|e| format!("unmaximize failed: {e}"))
    } else {
        window.maximize().map_err(|e| format!("maximize failed: {e}"))
    }
}

#[tauri::command]
fn window_close(window: tauri::Window) -> Result<(), String> {
    window.close().map_err(|e| format!("close failed: {e}"))
}

#[tauri::command]
fn window_start_drag(window: tauri::Window) -> Result<(), String> {
    window
        .start_dragging()
        .map_err(|e| format!("start_dragging failed: {e}"))
}

#[cfg(target_os = "windows")]
fn enable_windows_acrylic(hwnd: *mut core::ffi::c_void) {
    #[link(name = "dwmapi")]
    extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: *mut core::ffi::c_void,
            dw_attribute: u32,
            pv_attribute: *const core::ffi::c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
    const DWMWA_WINDOW_CORNER_PREFERENCE: u32 = 33;
    const DWMWCP_ROUND: i32 = 2;

    unsafe {
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        );
        let round: i32 = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&round as *const i32).cast(),
            std::mem::size_of::<i32>() as u32,
        );
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateAvailable {
    pub version: String,
    pub current_version: String,
    pub date: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[tauri::command]
fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateAvailable>, String> {
    use tauri_plugin_updater::UpdaterExt;
    let update = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?;
    Ok(update.map(|u| UpdateAvailable {
        version: u.version,
        current_version: env!("CARGO_PKG_VERSION").to_string(),
        date: u.date.map(|d| d.to_string()),
        body: u.body,
    }))
}

#[tauri::command]
async fn install_update(app: tauri::AppHandle) -> Result<(), String> {
    use tauri_plugin_updater::UpdaterExt;
    let Some(update) = app
        .updater()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| e.to_string())?
    else {
        return Err("No update is currently available.".to_string());
    };

    let mut downloaded: u64 = 0;
    let progress_app = app.clone();
    update
        .download_and_install(
            move |chunk_len, content_len| {
                downloaded = downloaded.saturating_add(chunk_len as u64);
                let _ = progress_app.emit(
                    "update_progress",
                    UpdateProgress {
                        downloaded,
                        total: content_len,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    tauri::Manager::state::<Bridge>(&app).settings.flush();
    app.restart();
}

/// Build and run the Tauri application.
pub fn run() {
    #[cfg(target_os = "windows")]
    std::env::set_var("WEBVIEW2_DEFAULT_BACKGROUND_COLOR", "0");

    let launch = parse_launch_args();
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            ping,
            emit_test_event,
            debug_log,
            window_minimize,
            window_toggle_maximize,
            window_close,
            window_start_drag,
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
            cancel_hotkey_capture,
            send_chat_message,
            toggle_mute,
            toggle_deafen,
            get_room_state,
            create_room,
            join_room,
            leave_room,
            set_peer_volume,
            set_peer_muted,
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
            // Session 29b Force Stop: the host's decision, persisted as a setting.
            set_music_force_stop,
            // And a listener's request to end somebody else's music.
            request_music_stop,
            music_pick_file,
            // Session 29b: the music bot session.
            music_bot_state,
            music_bot_start,
            music_bot_stop,
            music_queue_list,
            music_queue_add,
            music_queue_remove,
            music_queue_clear,
            music_queue_move,
            music_queue_set_loop,
            music_queue_jump,
            music_next,
            music_prev,
            music_pick_files,
            set_voice_quality,
            set_music_quality,
            get_app_version,
            check_for_update,
            install_update,
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

            // --- local store (Session 28) ---
            // SQLite at <config_dir>/verio.db, profile-aware because config_dir
            // already is. The legacy settings.json is imported once and renamed
            // inside SettingsStore::load, so this is the only place that knows
            // about either file.
            let db = Db::open(&config_dir)?;
            let settings = SettingsStore::load(db.clone(), &config_dir);
            let settings_snapshot = settings.get();
            // The two halves of the identity, named at the one place they exist.
            // `user_id` is persistent and belongs to this profile; `session_id`
            // is minted per process so two instances on one PC never collide.
            // Order matters: `SettingsStore::load` above adopts the legacy
            // `peer_id` into `identity`, so reading the id before that call would
            // generate a fresh one and silently discard the user's existing id.
            let user_id = db
                .user_id()
                .map_err(|e| format!("read persistent user id: {e}"))?;
            tracing::info!(
                user_id = %user_id,
                session_id = %db.session_id(),
                "persistent identity and session identity"
            );
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

            // Wire identity is the *session* half of the identity: fresh per
            // process, so multiple instances launched on the same PC (e.g. for
            // testing) never collide with each other in signaling rooms or UDP
            // relays. It stays the transport prefix, because the relay has to see
            // two instances as two peers.
            //
            // Session 29 also announces `user_id` here, and this is the one place
            // the persistent id goes on the wire. It has to: a room-wide mute is
            // keyed on the person, so a peer must be able to name the person
            // rather than this particular run of the app.
            let peer_uuid = db.session_id();
            let identity = Identity {
                uuid: peer_uuid,
                user_id: user_id.clone(),
                // The voice session is a person. The music bot is a *second*
                // session built in `music_bot.rs`, and that is the only place
                // this is ever `true`.
                is_bot: false,
                // Session 29b Force Stop: meaningless on a person's identity —
                // nothing reads it — and `false` is the honest value, since a voice
                // session is not a thing anybody can ask to stop.
                force_stop: false,
                name: if settings_snapshot.nickname.is_empty() {
                    "Player".to_string()
                } else {
                    settings_snapshot.nickname.clone()
                },
                version: verio_app::APP_VERSION.to_string(),
            };
            tracing::info!(peer_uuid = %peer_uuid, name = %identity.name, "peer identity");
            let (room_tx, room_rx) = std::sync::mpsc::channel::<RoomEvent>();
            let room = Arc::new(Mutex::new(RoomManager::new(
                identity.clone(),
                room_tx,
                db.clone(),
            )));

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
            let hotkey_state = Arc::new(Mutex::new(HotkeyPollerState::default()));
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
                // Session 29b: IDLE until somebody presses play for the room.
                bot: std::sync::Arc::new(Mutex::new(None)),
                bot_monitor: std::sync::Arc::new(Mutex::new(None)),
                bot_starting: AtomicBool::new(false),
                hotkeys: Arc::clone(&hotkey_state),
            };

            // Session 13: push music status to the UI (~4 Hz while playing, and
            // immediately on any state transition).
            //
            // Session 20: this loop also services queue advances. It is the right
            // place because it already holds an Arc<MusicSource> and it is an
            // ordinary OS thread — opening the next file here keeps that work off
            // the audio thread, which only ever *names* the next index. The poll
            // runs at 50 ms so the gap between tracks stays short; status is still
            // computed and emitted on the original 250 ms cadence (every 5th
            // iteration), so the event rate the UI sees is unchanged.
            {
                let tick_app = app.handle().clone();
                let tick_music = std::sync::Arc::clone(&bridge.music);
                let spawned = std::thread::Builder::new()
                    .name("verio-music-status".into())
                    .spawn(move || {
                        let mut last = String::new();
                        let mut tick: u32 = 0;
                        loop {
                            // Cheap when nothing is pending: one uncontended lock
                            // and an Option::take.
                            tick_music.service_advance();

                            tick = tick.wrapping_add(1);
                            if tick == 1 || tick.is_multiple_of(5) {
                                let status = tick_music.status();
                                let json = serde_json::to_string(&status).unwrap_or_default();
                                if status.playing || json != last {
                                    last = json;
                                    let _ = tick_app.emit("music_status_changed", status.clone());
                                }
                            }
                            std::thread::sleep(std::time::Duration::from_millis(50));
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
                                    if matches!(
                                        state,
                                        verio_app::room::RoomState::Idle
                                            | verio_app::room::RoomState::Closed
                                    ) {
                                        let bridge = tauri::Manager::state::<Bridge>(&fwd_app);
                                        stop_bot(&fwd_app, &bridge);
                                    }
                                    let _ = fwd_app.emit("room_state", state);
                                }
                                RoomEvent::RoomCreated { code } => {
                                    let _ = fwd_app.emit("room_created", serde_json::json!({ "code": code }));
                                }
                                RoomEvent::RoomJoined { code } => {
                                    let _ = fwd_app.emit("room_joined", serde_json::json!({ "code": code }));
                                }
                                RoomEvent::PeerConnected {
                                    name,
                                    uuid,
                                    user_id,
                                    is_bot,
                                    force_stop,
                                    arrived,
                                } => {
                                    // Only a genuine arrival is audible. The
                                    // roster adopted when we join a populated room
                                    // and a peer renaming themselves both arrive
                                    // here with `arrived: false` — the roster
                                    // still needs them, the cue does not.
                                    //
                                    // Session 29b: nor is a bot. The cue exists so
                                    // people notice when somebody joins or leaves
                                    // the call; a bot is not a participant, and
                                    // sounding one would make pressing Play on your
                                    // own music announce a person arriving — twice
                                    // over, once for the bot and once for its
                                    // departure when the queue ends.
                                    if arrived && !is_bot {
                                        play_cue(&fwd_app, verio_app::EarconKind::PeerJoin);
                                    }
                                    let _ = fwd_app.emit(
                                        "peer_connected",
                                        serde_json::json!({
                                            "name": name,
                                            "uuid": uuid,
                                            "user_id": user_id,
                                            "is_bot": is_bot,
                                            // Session 29b Force Stop: a listener's
                                            // Stop control is drawn from this, so a
                                            // bot whose host disabled it arrives
                                            // with `false` and the control is simply
                                            // not offered — rather than offered and
                                            // then ignored.
                                            "force_stop": force_stop,
                                            "arrived": arrived,
                                        }),
                                    );
                                }
                                RoomEvent::PeerDisconnected {
                                    reason,
                                    peer_id,
                                    is_bot,
                                } => {
                                    // Session 29b: a bot is not a participant, so its
                                    // departure is as silent as its arrival. See the
                                    // join cue above for why the pair must agree.
                                    if !is_bot {
                                        play_cue(&fwd_app, verio_app::EarconKind::PeerLeave);
                                    }
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
                                RoomEvent::PeerMuted {
                                    target_user_id,
                                    muted,
                                    by,
                                } => {
                                    // No cue here. A mute is not an arrival or a
                                    // departure, and sounding one would be
                                    // indistinguishable from somebody actually
                                    // leaving the call.
                                    let _ = fwd_app.emit(
                                        "peer_muted",
                                        serde_json::json!({
                                            "target_user_id": target_user_id,
                                            "muted": muted,
                                            "by": by,
                                        }),
                                    );
                                }
                                RoomEvent::MusicStopRequested { by } => {
                                    // Single deterministic path for remote `MusicStop`:
                                    // `bot_pump` defers to the voice room forwarder so
                                    // enforcement and host toast notification (`acted`)
                                    // never race between the two sockets.
                                    let bridge = tauri::Manager::state::<Bridge>(&fwd_app);
                                    let (hosts, force_stop) = {
                                        let guard = bridge.bot.lock().expect("bot lock");
                                        match guard.as_ref() {
                                            Some(bot) => (true, bot.accepts_remote_stop()),
                                            None => (false, false),
                                        }
                                    };
                                    let acted = hosts
                                        && !verio_app::music_bot::remote_stop_is_refused(force_stop);
                                    if acted {
                                        stop_bot(&fwd_app, &bridge);
                                    } else if hosts {
                                        tracing::info!(
                                            by = %by,
                                            "music bot: refusing a remote stop — Force Stop is off"
                                        );
                                    }
                                    let _ = fwd_app.emit(
                                        "music_stop_requested",
                                        serde_json::json!({
                                            "by": by,
                                            "acted": acted,
                                        }),
                                    );
                                }
                                RoomEvent::TransportModeChanged { mode } => {
                                    let _ = fwd_app.emit(
                                        "transport_mode_changed",
                                        serde_json::json!({ "mode": mode }),
                                    );
                                }
                                RoomEvent::ChatReceived {
                                    id,
                                    sender_user_id,
                                    sender_name,
                                    text,
                                    timestamp_ms,
                                } => {
                                    let _ = fwd_app.emit(
                                        "chat_message",
                                        serde_json::json!({
                                            "id": id,
                                            "sender_user_id": sender_user_id,
                                            "sender_name": sender_name,
                                            "text": text,
                                            "timestamp_ms": timestamp_ms,
                                        }),
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
            spawn_hotkey_poller(
                app.handle().clone(),
                Arc::clone(&hotkey_state),
                Arc::clone(&bridge.tx),
            );
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

            #[cfg(target_os = "windows")]
            if let Some(w) = app.get_webview_window("main") {
                use tauri::window::{Color, Effect, EffectsBuilder};
                let _ = w.set_effects(
                    EffectsBuilder::new()
                        .effect(Effect::Acrylic)
                        .color(Color(0x24, 0x1B, 0x3A, 0x88))
                        .build(),
                );
                if let Ok(hwnd) = w.hwnd() {
                    enable_windows_acrylic(hwnd.0);
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
        .on_window_event(|window, event| {
            if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                tauri::Manager::state::<Bridge>(window.app_handle())
                    .settings
                    .flush();
            }
        })
        .build(tauri::generate_context!())
        .expect("Verio failed to build")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::Exit | tauri::RunEvent::ExitRequested { .. }
            ) {
                tauri::Manager::state::<Bridge>(app).settings.flush();
            }
        });
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
                    // Session 29b: unless a bot owns the bus — see
                    // `voice_music_presence`.
                    let (music, music_title) =
                        voice_music_presence(&tauri::Manager::state::<Bridge>(&app).music);
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
                // Session 29c fix: the shortcut and the button flip the same state,
                // so they make the same sound. This path used to change mute in
                // silence — the state changed, the cue never played, and the two
                // ways of muting felt like different features.
                play_toggle_earcon(
                    &bridge,
                    if muted {
                        verio_app::EarconKind::MuteOn
                    } else {
                        verio_app::EarconKind::Unmute
                    },
                );
                tracing::info!(muted, "hotkey: mute toggle");
                send_room_state(&bridge);
                emit_audio_state(app, &bridge);
            }
        }
        HotkeyAction::Deafen => {
            if pressed {
                let (deafened, muted) = bridge.tx.toggle_deafen();
                // The same fix, and the same defect was here too: the shortcut
                // changed deafen silently.
                play_toggle_earcon(
                    &bridge,
                    if deafened {
                        verio_app::EarconKind::Deafen
                    } else {
                        verio_app::EarconKind::Undeafen
                    },
                );
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

pub const VK_BACK: i32 = 0x08;
pub const VK_TAB: i32 = 0x09;
pub const VK_RETURN: i32 = 0x0D;
pub const VK_SHIFT: i32 = 0x10;
pub const VK_CONTROL: i32 = 0x11;
pub const VK_MENU: i32 = 0x12; // Alt
pub const VK_CAPITAL: i32 = 0x14; // CapsLock
pub const VK_ESCAPE: i32 = 0x1B;
pub const VK_SPACE: i32 = 0x20;
pub const VK_PRIOR: i32 = 0x21; // Page Up
pub const VK_NEXT: i32 = 0x22; // Page Down
pub const VK_END: i32 = 0x23;
pub const VK_HOME: i32 = 0x24;
pub const VK_LEFT: i32 = 0x25;
pub const VK_UP: i32 = 0x26;
pub const VK_RIGHT: i32 = 0x27;
pub const VK_DOWN: i32 = 0x28;
pub const VK_INSERT: i32 = 0x2D;
pub const VK_DELETE: i32 = 0x2E;
pub const VK_LWIN: i32 = 0x5B;
pub const VK_RWIN: i32 = 0x5C;
pub const VK_NUMPAD0: i32 = 0x60;
pub const VK_NUMPAD1: i32 = 0x61;
pub const VK_NUMPAD2: i32 = 0x62;
pub const VK_NUMPAD3: i32 = 0x63;
pub const VK_NUMPAD4: i32 = 0x64;
pub const VK_NUMPAD5: i32 = 0x65;
pub const VK_NUMPAD6: i32 = 0x66;
pub const VK_NUMPAD7: i32 = 0x67;
pub const VK_NUMPAD8: i32 = 0x68;
pub const VK_NUMPAD9: i32 = 0x69;
pub const VK_MULTIPLY: i32 = 0x6A;
pub const VK_ADD: i32 = 0x6B;
pub const VK_SUBTRACT: i32 = 0x6D;
pub const VK_DECIMAL: i32 = 0x6E;
pub const VK_DIVIDE: i32 = 0x6F;
pub const VK_F1: i32 = 0x70;
pub const VK_OEM_NEC_EQUAL: i32 = 0x92;
pub const VK_OEM_1: i32 = 0xBA; // ;:
pub const VK_OEM_PLUS: i32 = 0xBB; // =+
pub const VK_OEM_COMMA: i32 = 0xBC; // ,<
pub const VK_OEM_MINUS: i32 = 0xBD; // -_
pub const VK_OEM_PERIOD: i32 = 0xBE; // .>
pub const VK_OEM_2: i32 = 0xBF; // /?
pub const VK_OEM_3: i32 = 0xC0; // `~
pub const VK_OEM_4: i32 = 0xDB; // [{
pub const VK_OEM_5: i32 = 0xDC; // \|
pub const VK_OEM_6: i32 = 0xDD; // ]}
pub const VK_OEM_7: i32 = 0xDE; // '"

fn parse_virtual_key(key: &str) -> Option<i32> {
    let lower = key.to_ascii_lowercase();
    match lower.as_str() {
        // Punctuation & symbols
        "/" | "slash" => Some(VK_OEM_2),
        "." | "period" => Some(VK_OEM_PERIOD),
        "," | "comma" => Some(VK_OEM_COMMA),
        ";" | "semicolon" => Some(VK_OEM_1),
        "'" | "quote" => Some(VK_OEM_7),
        "[" | "bracketleft" => Some(VK_OEM_4),
        "]" | "bracketright" => Some(VK_OEM_6),
        "\\" | "backslash" => Some(VK_OEM_5),
        "-" | "minus" => Some(VK_OEM_MINUS),
        "=" | "equal" => Some(VK_OEM_PLUS),
        "`" | "backquote" => Some(VK_OEM_3),

        // Space & control keys
        "space" => Some(VK_SPACE),
        "tab" => Some(VK_TAB),
        "backspace" => Some(VK_BACK),
        "enter" | "return" => Some(VK_RETURN),
        "escape" | "esc" => Some(VK_ESCAPE),
        "capslock" => Some(VK_CAPITAL),
        "insert" => Some(VK_INSERT),
        "delete" | "del" => Some(VK_DELETE),
        "home" => Some(VK_HOME),
        "end" => Some(VK_END),
        "pageup" => Some(VK_PRIOR),
        "pagedown" => Some(VK_NEXT),
        "arrowup" | "up" => Some(VK_UP),
        "arrowdown" | "down" => Some(VK_DOWN),
        "arrowleft" | "left" => Some(VK_LEFT),
        "arrowright" | "right" => Some(VK_RIGHT),

        // Numpad
        "numpad0" => Some(VK_NUMPAD0),
        "numpad1" => Some(VK_NUMPAD1),
        "numpad2" => Some(VK_NUMPAD2),
        "numpad3" => Some(VK_NUMPAD3),
        "numpad4" => Some(VK_NUMPAD4),
        "numpad5" => Some(VK_NUMPAD5),
        "numpad6" => Some(VK_NUMPAD6),
        "numpad7" => Some(VK_NUMPAD7),
        "numpad8" => Some(VK_NUMPAD8),
        "numpad9" => Some(VK_NUMPAD9),
        "numpadmultiply" => Some(VK_MULTIPLY),
        "numpadadd" => Some(VK_ADD),
        "numpadsubtract" => Some(VK_SUBTRACT),
        "numpaddecimal" => Some(VK_DECIMAL),
        "numpaddivide" => Some(VK_DIVIDE),
        "numpadenter" => Some(VK_RETURN),
        "numpadequal" => Some(VK_OEM_NEC_EQUAL),

        // Single alphanumeric characters
        s if s.len() == 1 => {
            let ch = s.chars().next().unwrap();
            if ch.is_ascii_lowercase() {
                Some(ch.to_ascii_uppercase() as i32)
            } else if ch.is_ascii_digit() {
                Some(ch as i32)
            } else {
                None
            }
        }

        // Function keys F1..F24
        s if s.starts_with('f') => {
            if let Ok(num) = s[1..].parse::<i32>() {
                if (1..=24).contains(&num) {
                    Some(VK_F1 + (num - 1))
                } else {
                    None
                }
            } else {
                None
            }
        }

        _ => None,
    }
}

/// Parse a stored binding string ("Ctrl+Shift+M", "B", "/", "Ctrl+/", etc.) into a HotkeyBinding.
fn parse_binding(combo: &str) -> Result<HotkeyBinding, String> {
    let norm = normalize_combo(combo);
    if norm.is_empty() {
        return Err("empty hotkey binding".to_string());
    }

    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut win = false;
    let mut main_vk: Option<i32> = None;

    for part in norm.split('+') {
        match part {
            "Ctrl" => {
                if ctrl {
                    return Err("duplicate Ctrl modifier".to_string());
                }
                ctrl = true;
            }
            "Shift" => {
                if shift {
                    return Err("duplicate Shift modifier".to_string());
                }
                shift = true;
            }
            "Alt" => {
                if alt {
                    return Err("duplicate Alt modifier".to_string());
                }
                alt = true;
            }
            "Super" => {
                if win {
                    return Err("duplicate Super modifier".to_string());
                }
                win = true;
            }
            other => {
                if main_vk.is_some() {
                    return Err(format!("multiple main keys specified: {combo:?}"));
                }
                let vk = parse_virtual_key(other)
                    .ok_or_else(|| format!("unrecognized key {other:?} in combo {combo:?}"))?;
                main_vk = Some(vk);
            }
        }
    }

    let vk = main_vk.ok_or_else(|| format!("no main key in hotkey combo {combo:?}"))?;
    Ok(HotkeyBinding {
        vk,
        ctrl,
        shift,
        alt,
        win,
    })
}

#[cfg(windows)]
fn is_key_down(vk: i32) -> bool {
    unsafe {
        (windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(vk) as u16 & 0x8000) != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModifierState {
    ctrl: bool,
    shift: bool,
    alt: bool,
    win: bool,
}

#[must_use]
fn binding_matches_modifiers(
    binding: &HotkeyBinding,
    key_down: bool,
    strict_modifiers: bool,
    held: ModifierState,
) -> bool {
    if !key_down {
        return false;
    }
    let has_mods = binding.ctrl || binding.shift || binding.alt || binding.win;
    if !strict_modifiers && !has_mods {
        // Single-key PTT must remain active while holding Shift/Ctrl/etc. in games.
        return true;
    }
    binding.ctrl == held.ctrl
        && binding.shift == held.shift
        && binding.alt == held.alt
        && binding.win == held.win
}

#[cfg(windows)]
fn check_binding_down(binding: &HotkeyBinding, strict_modifiers: bool) -> bool {
    let held = ModifierState {
        ctrl: is_key_down(VK_CONTROL),
        shift: is_key_down(VK_SHIFT),
        alt: is_key_down(VK_MENU),
        win: is_key_down(VK_LWIN) || is_key_down(VK_RWIN),
    };
    binding_matches_modifiers(binding, is_key_down(binding.vk), strict_modifiers, held)
}

#[cfg(windows)]
fn spawn_hotkey_poller(
    app: AppHandle,
    state: Arc<Mutex<HotkeyPollerState>>,
    bridge_tx: Arc<TxState>,
) {
    let spawned = std::thread::Builder::new()
        .name("verio-hotkey-poller".into())
        .spawn(move || {
            let mut prev_mute = false;
            let mut prev_deafen = false;
            let mut prev_ptt = false;

            loop {
                std::thread::sleep(std::time::Duration::from_millis(10));

                let (mute_b, deafen_b, ptt_b, paused, seed_edges) = {
                    let mut guard = state.lock().expect("hotkey poller lock");
                    let seed = std::mem::take(&mut guard.seed_edges);
                    (
                        guard.mute.clone(),
                        guard.deafen.clone(),
                        guard.ptt.clone(),
                        guard.paused,
                        seed,
                    )
                };

                if paused {
                    if prev_ptt {
                        prev_ptt = false;
                        bridge_tx.set_ptt_held(false);
                        let bridge = tauri::Manager::state::<Bridge>(&app);
                        send_room_state(&bridge);
                        emit_audio_state(&app, &bridge);
                    }
                    prev_mute = false;
                    prev_deafen = false;
                    continue;
                }

                // Check live key states (single-key PTT ignores held game modifiers)
                let mute_down = mute_b.as_ref().is_some_and(|b| check_binding_down(b, true));
                let deafen_down = deafen_b
                    .as_ref()
                    .is_some_and(|b| check_binding_down(b, true));
                let ptt_down = ptt_b.as_ref().is_some_and(|b| check_binding_down(b, false));

                if seed_edges {
                    prev_mute = mute_down;
                    prev_deafen = deafen_down;
                    prev_ptt = ptt_down;
                    continue;
                }

                // Check Mute (rising edge)
                if mute_down && !prev_mute {
                    handle_shortcut_event(&app, HotkeyAction::Mute, ShortcutState::Pressed);
                }
                prev_mute = mute_down;

                // Check Deafen (rising edge)
                if deafen_down && !prev_deafen {
                    handle_shortcut_event(&app, HotkeyAction::Deafen, ShortcutState::Pressed);
                }
                prev_deafen = deafen_down;

                // Check PTT (level transition)
                if ptt_down != prev_ptt {
                    prev_ptt = ptt_down;
                    bridge_tx.set_ptt_held(ptt_down);
                    let bridge = tauri::Manager::state::<Bridge>(&app);
                    send_room_state(&bridge);
                    emit_audio_state(&app, &bridge);
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!("spawn hotkey poller failed: {e}");
    }
}

#[cfg(not(windows))]
fn spawn_hotkey_poller(
    _app: AppHandle,
    _state: Arc<Mutex<HotkeyPollerState>>,
    _bridge_tx: Arc<TxState>,
) {
}

/// (Re)load all hotkeys from settings into the hardware poller.
fn register_hotkeys(app: &AppHandle) -> Result<(), String> {
    let bridge: State<Bridge> = app.state();
    let hotkeys = bridge.settings.get().hotkeys;

    let mute = if !hotkeys.mute.is_empty() {
        Some(parse_binding(&hotkeys.mute)?)
    } else {
        None
    };
    let deafen = if !hotkeys.deafen.is_empty() {
        Some(parse_binding(&hotkeys.deafen)?)
    } else {
        None
    };
    let ptt = if !hotkeys.ptt.is_empty() {
        Some(parse_binding(&hotkeys.ptt)?)
    } else {
        None
    };

    {
        let mut hp = bridge.hotkeys.lock().expect("hotkeys lock");
        hp.mute = mute;
        hp.deafen = deafen;
        hp.ptt = ptt;
        hp.paused = false;
        hp.seed_edges = true;
    }

    tracing::info!("hotkeys loaded into hardware poller");
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

    #[test]
    fn punctuation_and_oem_bindings_parse() {
        for (combo, expected_vk) in [
            ("/", VK_OEM_2),
            (".", VK_OEM_PERIOD),
            (",", VK_OEM_COMMA),
            (";", VK_OEM_1),
            ("'", VK_OEM_7),
            ("[", VK_OEM_4),
            ("]", VK_OEM_6),
            ("\\", VK_OEM_5),
            ("-", VK_OEM_MINUS),
            ("=", VK_OEM_PLUS),
            ("`", VK_OEM_3),
        ] {
            let res = parse_binding(combo).unwrap_or_else(|e| panic!("failed to parse {combo}: {e}"));
            assert_eq!(res.vk, expected_vk);
            assert!(!res.ctrl && !res.shift && !res.alt && !res.win);

            let with_ctrl = parse_binding(&format!("Ctrl+{combo}")).unwrap();
            assert_eq!(with_ctrl.vk, expected_vk);
            assert!(with_ctrl.ctrl && !with_ctrl.shift && !with_ctrl.alt && !with_ctrl.win);
        }
    }

    #[test]
    fn function_keys_parse() {
        for i in 1..=24 {
            let name = format!("F{i}");
            let res = parse_binding(&name).unwrap();
            assert_eq!(res.vk, VK_F1 + (i - 1));
        }
    }

    /// Contract test with the frontend. `ui/src/lib/hotkey.ts` builds numpad
    /// bindings out of `event.code`, and every name it can emit has to survive
    /// `normalize_combo` and then parse. The frontend deliberately does *not* use
    /// `event.key` for these — NumpadAdd reports `"+"`, and with NumLock off the
    /// digits report `Insert`/`End`/`ArrowDown`, so `key` yields either an
    /// unparseable string or the wrong key entirely.
    #[test]
    fn numpad_bindings_parse() {
        for name in [
            "Numpad0",
            "Numpad1",
            "Numpad2",
            "Numpad3",
            "Numpad4",
            "Numpad5",
            "Numpad6",
            "Numpad7",
            "Numpad8",
            "Numpad9",
            "NumpadAdd",
            "NumpadSubtract",
            "NumpadMultiply",
            "NumpadDivide",
            "NumpadDecimal",
            "NumpadEnter",
            "NumpadEqual",
        ] {
            assert!(parse_binding(name).is_ok(), "{name} did not parse");
            assert!(
                parse_binding(&format!("Ctrl+Shift+{name}")).is_ok(),
                "Ctrl+Shift+{name} did not parse"
            );
        }
    }

    /// Pins the reason the frontend cannot use `event.key` for the numpad: the
    /// string it would build is rejected. `normalize_combo` drops the empty
    /// segments, leaving a bare "Ctrl", which is not a valid shortcut.
    #[test]
    fn bare_double_plus_binding_is_rejected() {
        assert!(parse_binding("Ctrl++").is_err(), "Ctrl++ should be rejected");
        assert!(parse_binding("Ctrl+NumpadAdd").is_ok());
    }

    #[test]
    fn single_key_ptt_tolerates_held_game_modifiers_while_strict_rejects() {
        let bare = HotkeyBinding {
            vk: 0x56,
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
        };
        let shift_held = ModifierState {
            ctrl: false,
            shift: true,
            alt: false,
            win: false,
        };
        let all_held = ModifierState {
            ctrl: true,
            shift: true,
            alt: true,
            win: true,
        };
        let none_held = ModifierState {
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
        };

        // Single-key PTT (`V`): no required modifiers, non-strict check.
        assert!(binding_matches_modifiers(&bare, true, false, shift_held));
        assert!(binding_matches_modifiers(&bare, true, false, all_held));
        assert!(!binding_matches_modifiers(&bare, false, false, all_held));

        // Strict modifier hotkey (e.g., Mute `M` or `Ctrl+Shift+M`): extra Shift fails.
        assert!(!binding_matches_modifiers(&bare, true, true, shift_held));
        assert!(binding_matches_modifiers(&bare, true, true, none_held));

        // Modifier-bound PTT (`Ctrl+V`): still checks exact modifiers even with !strict.
        let ctrl_v = HotkeyBinding {
            vk: 0x56,
            ctrl: true,
            shift: false,
            alt: false,
            win: false,
        };
        let ctrl_only = ModifierState {
            ctrl: true,
            shift: false,
            alt: false,
            win: false,
        };
        assert!(!binding_matches_modifiers(&ctrl_v, true, false, shift_held));
        assert!(!binding_matches_modifiers(&ctrl_v, true, false, all_held));
        assert!(binding_matches_modifiers(&ctrl_v, true, false, ctrl_only));
    }

    #[test]
    fn watcher_owns_bot_only_matches_exact_session_id() {
        assert!(!watcher_owns_bot(None, "sess-1"));
        assert!(session_id_matches(Some("sess-1"), "sess-1"));
        assert!(!session_id_matches(Some("sess-2"), "sess-1"));
        assert!(!session_id_matches(None, "sess-1"));
    }

    #[test]
    fn hotkey_binding_for_action_validates_before_pausing() {
        let hk = verio_app::settings::HotkeySettings {
            mute: "Ctrl+M".into(),
            deafen: "Ctrl+D".into(),
            ptt: "V".into(),
        };
        assert_eq!(hotkey_binding_for_action(&hk, "mute").unwrap(), "Ctrl+M");
        assert_eq!(hotkey_binding_for_action(&hk, "deafen").unwrap(), "Ctrl+D");
        assert_eq!(hotkey_binding_for_action(&hk, "ptt").unwrap(), "V");
        assert!(hotkey_binding_for_action(&hk, "invalid").is_err());
    }
}

