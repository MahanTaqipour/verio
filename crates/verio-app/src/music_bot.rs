//! Session 29b: the music bot session.
//!
//! A bot is a **second session of the host's own client**, not a separate program
//! and not a second window: the same process opens a second connection to the
//! signalling server with a fresh session id and the host's `user_id`, joins the
//! same room, and streams the host's music as its own audio stream. Every client —
//! including the host — therefore sees two peers from the host: their voice peer
//! and a peer named [`BOT_NAME`].
//!
//! ## Why a second session at all
//!
//! Until now music was mixed into the host's outgoing voice frame by the audio
//! pipeline. That works, and it is why the feature looked finished, but it makes
//! the music inseparable from the voice: one stream, one volume, one mute. Giving
//! the music its own peer means every listener gets an ordinary per-peer volume
//! slider and mute for it — controls that already exist, already work, and are
//! already tested.
//!
//! ## Why it is headless
//!
//! The bot decodes files, encodes Opus and sends. It never opens an audio device,
//! so `verio-audio` is deliberately absent from this path and a bot could run on a
//! machine with no sound card. The consequence worth knowing: the relay does not
//! echo a sender's own datagrams, so the host is a peer of their own bot but never
//! receives from it. The host hears the music through the pipeline's local monitor,
//! which the bot feeds directly (see `monitor_tx`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use verio_discovery::{Identity, VpsSignaling};
use verio_dsp::codec::OpusEncoderWrapper;
use verio_dsp::RNNOISE_FRAME;
use verio_transport::{
    spawn_relay_session_with_candidates, unix_ms, ControlMessage, PeerHandle, TransportEvent,
};

use crate::hotkeys::TxState;
use crate::pipeline::{monitor_contributes, LiveControls, MusicSource};
use crate::room::RoomManager;

/// The display name a bot session joins under.
///
/// Fixed rather than "Music · <nickname>": the bot's `user_id` already ties it to
/// the host, the host's own tile carries the badge, and a fixed name is what the
/// tests and the UI can rely on.
pub const BOT_NAME: &str = "Music";

/// How often the bot re-announces itself, so a peer who joined after us still
/// learns we are a bot. Matches the room's own roster gossip interval.
const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(3);

/// What the bot tells its owner.
#[derive(Debug, Clone)]
pub enum BotEvent {
    /// The session ended on its own — the queue ran out, or the transport dropped.
    /// The bridge treats this exactly like a user-initiated stop: back to IDLE.
    Stopped { reason: String },
}

/// Everything the bot needs from the rest of the app.
pub struct BotConfig {
    /// Signalling server, as configured for the room.
    pub vps_addr: String,
    /// The room to join — the same code the host's voice session is in.
    pub room_code: String,
    /// The host's PERSISTENT id. Shared with the bot on purpose: it is the same
    /// person, and it is what lets a client tell which voice peer is the host.
    pub host_user_id: String,
    /// The host's display name, for the spawn log and the "already playing" toast.
    pub host_name: String,
    /// Session 29b Force Stop: may another peer stop this bot?
    ///
    /// Read from the host's settings at the moment the bot is spawned and fixed for
    /// the session's life. That is what "applied each time the user becomes host"
    /// means: toggling the setting changes the *next* bot, never the running one, so
    /// the room can never be told one thing and served another.
    pub force_stop: bool,
    pub music: Arc<MusicSource>,
    pub controls: Arc<LiveControls>,
    pub tx_state: Arc<TxState>,
    /// Where the host's own copy of the music goes. A slot rather than a sender
    /// because a pipeline restart replaces it, exactly like the room's mute sink.
    pub monitor_tx: Arc<Mutex<Option<Sender<Vec<f32>>>>>,
    pub events: Sender<BotEvent>,
}

/// A running bot session. Dropping this does **not** stop it — call [`Self::stop`],
/// so a forgotten handle cannot leave a session talking into a room forever.
pub struct MusicBot {
    /// The bot's own session id. Distinct from the host's voice session id, which
    /// is what makes the two peers distinguishable on the wire.
    session_id: String,
    /// Session 29b Force Stop: the host's decision, exactly as declared to the room.
    ///
    /// Held here rather than re-read from settings when a request arrives, so the
    /// value that is *enforced* is always the value that was *announced*. Re-reading
    /// would let a settings change make the bot refuse a stop it had already told the
    /// room it would accept — a button that lies.
    force_stop: bool,
    stop: Arc<AtomicBool>,
    signaling_stop: Arc<AtomicBool>,
    handle: PeerHandle,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl MusicBot {
    /// The bot session's id, for logging and diagnostics.
    #[must_use]
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Session 29b Force Stop: may a peer other than the host end this session?
    ///
    /// The single authority for that question. Both readers — the bot's own control
    /// pump, and the host's bridge which also stops on a request — ask here, so the
    /// two cannot disagree about whether a stop was honoured.
    #[must_use]
    pub fn accepts_remote_stop(&self) -> bool {
        self.force_stop
    }

    /// Stop the session: leave the room on the signalling server (so every peer
    /// gets a `PeerLeft` and drops the tile at once), close the relay socket, and
    /// join the worker threads.
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // Leaving the room is what makes the Music tile disappear promptly. The
        // roster sweep would also remove it, but only after its 15 s timeout, and
        // a stale tile that still accepts a volume drag is a worse bug than a slow
        // one.
        self.signaling_stop.store(true, Ordering::Relaxed);
        self.handle.disconnect();
        let mut threads = match self.threads.lock() {
            Ok(t) => t,
            Err(p) => p.into_inner(),
        };
        for t in threads.drain(..) {
            let _ = t.join();
        }
    }
}

/// Start a bot session for `cfg`.
///
/// # Errors
/// The signalling server could not be reached, the room could not be joined, or the
/// relay address could not be resolved. Nothing is left running on an error.
pub fn start(cfg: BotConfig) -> Result<MusicBot, String> {
    // 1. A fresh session id, the host's persistent id, and the bot declaration.
    let session_id = verio_discovery::Uuid::new_v4();
    let identity = Identity {
        uuid: session_id,
        user_id: cfg.host_user_id.clone(),
        is_bot: true,
        // Session 29b Force Stop: announced here, on the registration `Hello`, so a
        // peer that joins the room reads the host's decision from the very first
        // packet rather than defaulting and then being corrected.
        force_stop: cfg.force_stop,
        name: BOT_NAME.to_string(),
        version: crate::APP_VERSION.to_string(),
    };

    // 2. Join the room on the signalling server. This is not needed for the audio
    //    — the relay fans out by room code — but it *is* needed for the roster:
    //    it is what gives every other client a `PeerJoined` now and a `PeerLeft`
    //    the moment we stop.
    let mut session = VpsSignaling::join(&cfg.vps_addr, &cfg.room_code, &identity)
        .map_err(|e| format!("music bot could not join room {}: {e}", cfg.room_code))?;

    // 3. One relay socket for the whole room.
    let relay_addrs = RoomManager::resolve_vps_relay_candidates(&cfg.vps_addr)?;
    let primary = relay_addrs[0];
    let fallbacks = relay_addrs[1..].to_vec();

    let (transport_tx, transport_rx) = mpsc::channel::<TransportEvent>();
    // The relay path derives each datagram's sender from its UUID prefix and runs
    // its own keepalive, so the silence frame is unused here (`_silence_frame`).
    let handle = spawn_relay_session_with_candidates(
        primary,
        fallbacks,
        cfg.room_code.clone(),
        identity.clone(),
        identity.clone(),
        Arc::new(Vec::new()),
        transport_tx,
        "cloud_relay",
    )
    .map_err(|e| format!("music bot relay session: {}", e.0))?;

    let stop = Arc::new(AtomicBool::new(false));
    let signaling_stop = Arc::new(AtomicBool::new(false));
    session.set_stop_flag(Arc::clone(&signaling_stop));

    let mut threads = Vec::new();

    // 4. The control pump: the only thing that can stop us from outside.
    {
        let stop = Arc::clone(&stop);
        let handle = handle.clone();
        let events = cfg.events.clone();
        let session_id = session_id.to_string();
        // Session 29b Force Stop: the pump is where a remote stop is *honoured*, so
        // it is where the host's decision has to be enforced. Passing the value in
        // rather than re-reading settings keeps the enforcement identical to the
        // announcement, for the life of the session.
        let force_stop = cfg.force_stop;
        threads.push(
            std::thread::Builder::new()
                .name("verio-bot-pump".into())
                .spawn(move || {
                    bot_pump(transport_rx, stop, handle, events, session_id, force_stop);
                })
                .map_err(|e| format!("spawn bot pump: {e}"))?,
        );
    }

    // 5. The signalling listener, so our `Leave` reaches the server immediately.
    threads.push(
        std::thread::Builder::new()
            .name("verio-bot-signaling".into())
            .spawn(move || session.run_listener(|_| {}))
            .map_err(|e| format!("spawn bot signalling listener: {e}"))?,
    );

    // 6. The encode loop: decode → 48 kHz mono → Opus → the room.
    {
        let stop = Arc::clone(&stop);
        let handle = handle.clone();
        let events = cfg.events.clone();
        let music = Arc::clone(&cfg.music);
        let controls = Arc::clone(&cfg.controls);
        let tx_state = Arc::clone(&cfg.tx_state);
        let monitor_tx = Arc::clone(&cfg.monitor_tx);
        let host_user_id = cfg.host_user_id.clone();
        // Re-announced with every `Hello`, so a peer who joined after the bot did
        // still learns the host's decision without a message of its own.
        let force_stop = cfg.force_stop;
        threads.push(
            std::thread::Builder::new()
                .name("verio-bot-encode".into())
                .spawn(move || {
                    encode_loop(EncodeLoop {
                        stop,
                        handle,
                        events,
                        music,
                        controls,
                        tx_state,
                        monitor_tx,
                        host_user_id,
                        force_stop,
                    });
                })
                .map_err(|e| format!("spawn bot encode loop: {e}"))?,
        );
    }

    tracing::info!(
        hosted_by = %cfg.host_name,
        session = %short_id(&session_id.to_string()),
        room = %cfg.room_code,
        // Session 29b Force Stop: logged at spawn, because "why can nobody stop
        // this music?" is otherwise a question the log cannot answer.
        force_stop = cfg.force_stop,
        "music bot: hosted by {} session={} force_stop={}",
        cfg.host_name,
        short_id(&session_id.to_string()),
        cfg.force_stop
    );

    Ok(MusicBot {
        session_id: session_id.to_string(),
        force_stop: cfg.force_stop,
        stop,
        signaling_stop,
        handle,
        threads: Mutex::new(threads),
    })
}

/// First 8 characters of a session id — enough to correlate a log line with a peer.
fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Does a relay `Disconnected` mean the session ended *unexpectedly*?
///
/// The transport reports every exit through the same event, including the one it
/// was asked to make: [`MusicBot::stop`] disconnects the socket, the driver exits,
/// and `TransportEvent::Disconnected` arrives carrying `"user disconnect"`. Taken at
/// face value that tells the user their music died because the connection dropped —
/// on the ordinary path, every single time they press Stop.
///
/// So the only thing that makes the disconnect news is that we did not ask for it.
#[must_use]
fn disconnect_ends_the_session(requested_stop: bool) -> bool {
    !requested_stop
}

/// Session 29b Force Stop: is a remote `MusicStop` refused?
///
/// Named rather than inlined for the same reason as [`disconnect_ends_the_session`]
/// — it is the rule the feature turns on, and its *polarity* is the thing that can
/// be got wrong. A control that quietly does the opposite of what its label says is
/// worse than no control, so the direction gets a name and a test instead of an
/// inline `!` that a reader has to re-derive.
///
/// `force_stop` is the host's decision: `true` means "anyone may stop my music", so
/// nothing is refused.
#[must_use]
fn remote_stop_is_refused(force_stop: bool) -> bool {
    !force_stop
}

/// Transport events for the bot session. The only one that matters is a stop.
fn bot_pump(
    rx: mpsc::Receiver<TransportEvent>,
    stop: Arc<AtomicBool>,
    handle: PeerHandle,
    events: Sender<BotEvent>,
    session_id: String,
    force_stop: bool,
) {
    while let Ok(ev) = rx.recv() {
        match ev {
            TransportEvent::Control {
                peer,
                message: ControlMessage::MusicStop,
            } => {
                // Session 29b Force Stop: enforced *here*, where the stop would take
                // effect, not only in the UI that draws the button. Any peer can send
                // this frame — including one that never saw a button, or one that
                // ignores its own disabled state — so a check that lived only in the
                // interface would be decoration.
                if remote_stop_is_refused(force_stop) {
                    tracing::info!(
                        by = %peer,
                        session = %short_id(&session_id),
                        "music bot: stop refused — the host disabled Force Stop"
                    );
                    // Not a `break`: the session stays up and keeps playing. Refusing
                    // is a no-op, and the room is never told anything happened.
                    continue;
                }
                tracing::info!(by = %peer, session = %short_id(&session_id), "music bot: stop requested");
                stop.store(true, Ordering::Relaxed);
                // The bridge hears about it through the encode loop's exit, so the
                // room returns to IDLE by the same path a local Stop takes.
                let _ = events.send(BotEvent::Stopped {
                    reason: "stopped by another peer".to_string(),
                });
                break;
            }
            TransportEvent::Disconnected { reason } => {
                // See `disconnect_ends_the_session`: a stop we asked for arrives
                // here too, and must not be reported as a dropped connection.
                if disconnect_ends_the_session(stop.load(Ordering::Relaxed)) {
                    tracing::warn!(%reason, "music bot: relay session dropped");
                    stop.store(true, Ordering::Relaxed);
                    let _ = events.send(BotEvent::Stopped {
                        reason: format!("the connection dropped ({reason})"),
                    });
                }
                break;
            }
            _ => {}
        }
    }
    // The socket is already gone; this just stops a lingering keepalive.
    handle.disconnect();
}

struct EncodeLoop {
    stop: Arc<AtomicBool>,
    handle: PeerHandle,
    events: Sender<BotEvent>,
    music: Arc<MusicSource>,
    controls: Arc<LiveControls>,
    tx_state: Arc<TxState>,
    monitor_tx: Arc<Mutex<Option<Sender<Vec<f32>>>>>,
    /// Re-sent with every announcement so a peer who missed the registration
    /// `Hello` still learns which person this bot belongs to.
    host_user_id: String,
    /// Session 29b Force Stop: re-sent with every announcement too, so a peer who
    /// joined after the bot learns the host's decision, and a peer whose first
    /// `Hello` was lost is corrected within one announce interval rather than never.
    force_stop: bool,
}

/// Decode the host's music and send it into the room until told to stop.
///
/// Deliberately *not* the audio pipeline: no capture device, no RNNoise, no VAD
/// gate, no DTX. DTX in particular is switched **off** explicitly — it would
/// collapse sustained music into near-silent packets, which is the same reason the
/// voice pipeline disables it while music plays.
fn encode_loop(cfg: EncodeLoop) {
    let mut encoder = match OpusEncoderWrapper::new() {
        Ok(e) => e,
        Err(e) => {
            tracing::error!("music bot: encoder init failed: {e}");
            let _ = cfg.events.send(BotEvent::Stopped {
                reason: format!("could not start the audio encoder ({e})"),
            });
            return;
        }
    };
    if let Err(e) = encoder.set_dtx(false) {
        tracing::warn!("music bot: DTX disable failed: {e}");
    }

    let mut buf = [0.0_f32; RNNOISE_FRAME];
    let mut last_announce = std::time::Instant::now() - ANNOUNCE_INTERVAL;
    let mut announced_title = String::new();
    let mut applied_kbps = 0u32;

    loop {
        if cfg.stop.load(Ordering::Relaxed) {
            break;
        }

        // Announce on a timer so a peer who joined after us still learns what we
        // are, and immediately whenever the track changes.
        let title = cfg.music.status().title.unwrap_or_default();
        if last_announce.elapsed() >= ANNOUNCE_INTERVAL || title != announced_title {
            last_announce = std::time::Instant::now();
            announced_title = title.clone();
            send_announcements(&cfg, &title);
        }

        if !cfg.music.is_active() {
            // Not playing. A pause keeps the session alive on purpose ("queue
            // non-empty or paused → bot stays alive"), so only a queue that has
            // actually run out ends it.
            if cfg.music.status().queue_finished {
                let _ = cfg.events.send(BotEvent::Stopped {
                    reason: "the queue finished".to_string(),
                });
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }

        // Session 14's quality tiers, re-read every pass so a settings change
        // lands. Voice and music share one Opus configuration here: the plan for
        // this session is to reuse the voice bitrate and revisit it only if the
        // music sounds thin.
        let want_kbps = cfg.controls.voice_kbps();
        if want_kbps != applied_kbps {
            match encoder.set_bitrate_bps((want_kbps * 1000) as i32) {
                Ok(()) => applied_kbps = want_kbps,
                Err(e) => tracing::warn!("music bot: bitrate change failed: {e}"),
            }
        }

        let n = cfg.music.next_chunk(&mut buf);
        if n == 0 {
            // Either the queue is advancing to the next entry (which happens on
            // another task) or it has just ended. The next pass decides which.
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }

        let volume = cfg.music.volume();
        let chunk: Vec<f32> = buf[..n].iter().map(|s| s * volume).collect();

        // The host's own copy. Gated exactly like the pipeline's monitor: on, not
        // deafened, and music actually playing.
        if monitor_contributes(cfg.music.monitor(), cfg.tx_state.is_deafened(), true) {
            let sink = cfg.monitor_tx.lock().ok().and_then(|g| g.clone());
            if let Some(tx) = sink {
                let _ = tx.send(chunk.clone());
            }
        }

        match encoder.push(&chunk) {
            Ok(Some(packet)) => cfg.handle.send_audio(unix_ms(), packet),
            Ok(None) => {}
            Err(e) => tracing::error!("music bot: opus encode failed: {e}"),
        }
    }
    tracing::info!("music bot: encode loop stopped");
}

/// Tell the room who we are and what we are playing.
fn send_announcements(cfg: &EncodeLoop, title: &str) {
    cfg.handle.send_control(ControlMessage::Hello {
        name: BOT_NAME.to_string(),
        version: crate::APP_VERSION.to_string(),
        // The host's id, not an empty string: if the registration `Hello` was lost
        // this is what repairs the roster entry, and an empty id could never do
        // that — the roster refuses to overwrite a known id with nothing.
        user_id: cfg.host_user_id.clone(),
        is_bot: true,
        // Session 29b Force Stop: the host's decision, repeated with every
        // announcement. Fixed for the session, so this can never contradict the
        // registration `Hello` — it only ever repeats it.
        force_stop: cfg.force_stop,
    });
    cfg.handle.send_control(ControlMessage::State {
        // A bot never speaks and is never muted; it plays.
        speaking: false,
        mute: false,
        deafen: false,
        music: true,
        music_title: title.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the guard: the ordinary Stop must not be reported as a
    /// failure. This is the case that fires on every normal stop.
    #[test]
    fn a_disconnect_we_asked_for_is_not_news() {
        assert!(
            !disconnect_ends_the_session(true),
            "our own stop disconnects the socket; that is not a dropped connection"
        );
    }

    /// And the case the guard exists to protect, so it cannot pass by returning
    /// `false` unconditionally — which would silently swallow a real dropout.
    #[test]
    fn a_disconnect_we_did_not_ask_for_is_news() {
        assert!(
            disconnect_ends_the_session(false),
            "a relay that drops on its own has to end the session and say so"
        );
    }

    // -- Session 29b Force Stop ----------------------------------------------

    /// The default case: the host allows a remote stop, so a request is honoured.
    /// The plan calls this "default: ON", and it is the behaviour that has to work
    /// for a listener who presses Stop without knowing the setting exists.
    #[test]
    fn a_remote_stop_is_honoured_when_the_host_allows_it() {
        assert!(
            !remote_stop_is_refused(true),
            "Force Stop on means another peer's request has to be acted on"
        );
    }

    /// The case the setting exists for, and the one that makes the toggle real.
    /// Without it the function could return `false` unconditionally and the switch
    /// would change nothing while looking, in the UI, exactly like one that works.
    #[test]
    fn a_remote_stop_is_refused_when_the_host_forbids_it() {
        assert!(
            remote_stop_is_refused(false),
            "Force Stop off means another peer's request has to be ignored"
        );
    }
}
