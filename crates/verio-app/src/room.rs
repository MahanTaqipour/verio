//! Phase 2 room session: minimal state machine + wiring audio ↔ transport.
//!
//! State machine: `Idle → Connecting → Connected → Closed` (a Closed session
//! returns the room to Idle so a new session can start; the UI sees the whole
//! sequence via [`RoomEvent`]s).
//!
//! Wiring (audio never touches the Tauri IPC boundary):
//! - OUTGOING: the pipeline's processing thread pushes encoded Opus frames into
//!   a channel ([`NetAudioOut`], sender cloned into every `PipelineConfig`);
//!   a permanent pump thread forwards them to the active peer's transport.
//! - INCOMING: the transport pump thread pushes [`NetAudioIn`] into the slot
//!   the pipeline's RX thread installed (`set_net_in`), so network audio feeds
//!   the existing RX path (per-peer pre-buffer → decode → mix).
//!
//! At most ONE remote peer this phase (mesh >2 peers is Phase 3).

use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use verio_discovery::{
    create_answer_invite, create_invite, parse_invite, DirectSession, Identity, InviteKind,
    Role, Signaling,
};
use verio_transport::{
    spawn_session, ControlMessage, PeerHandle, SessionSetup, SdpPendingOffer,
    TransportEvent,
};

use crate::db::Db;
use crate::pipeline::{NetAudioIn, NetAudioOut};

/// The remote identity learned at signaling time.
#[derive(Debug, Clone)]
pub struct RemotePeer {
    pub uuid: verio_discovery::Identity,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomState {
    Idle,
    Connecting,
    Connected,
    Closed,
}

/// Events the room emits; the Tauri layer forwards these to the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RoomEvent {
    StateChanged { state: RoomState },
    RoomCreated { code: String },
    RoomJoined { code: String },
    PeerConnected {
        name: String,
        uuid: String,
        /// Session 29: the peer's persistent id, so a tile can offer a room-wide
        /// mute. Empty until their `Hello` reaches us, at which point the roster
        /// refresh re-emits this event carrying it.
        user_id: String,
        /// Session 29b: this peer is a music bot session, not a person. `false`
        /// covers both "a person" and "we have not been told yet".
        is_bot: bool,
        /// Session 29b: this bot accepts a remote stop. Meaningful only when
        /// `is_bot`; `false` for a person, and `false` for a bot we have not been
        /// told about — the fail-closed direction, where the cost is a control that
        /// is not offered rather than one that is offered and ignored.
        force_stop: bool,
        /// `true` only when this peer was not in the roster before — a genuine
        /// arrival. `false` covers the roster adopted when we join a populated
        /// room, and a peer renaming themselves: the roster changed, but nobody
        /// arrived, so a consumer must not announce it.
        arrived: bool,
    },
    PeerDisconnected {
        reason: String,
        peer_id: Option<String>,
        /// Session 29b: this was a bot session, not a person.
        ///
        /// Carried so the departure cue can stay silent for it. The join cue is
        /// already withheld from bots — they are not participants, and a bot is
        /// something the user started themselves — so without this the pair would be
        /// asymmetric: no sound when the music starts, a chime when it stops.
        is_bot: bool,
    },
    /// Remote speaking/mute/deafen/music state (control channel).
    PeerState {
        peer_id: String,
        speaking: bool,
        mute: bool,
        deafen: bool,
        music: bool,
        music_title: String,
    },
    TransportModeChanged { mode: String },
    Rtt { ms: f64 },
    Error { message: String },
    /// Session 29: a room-wide mute changed — either one this client set, or one
    /// another peer broadcast.
    ///
    /// Emitted for every change, including the ones this client originated, so the
    /// UI has one code path for "the mute set changed" rather than two that can
    /// drift apart.
    PeerMuted {
        /// The *persistent* id of the silenced peer, not their wire id: that is
        /// what the mute is keyed on.
        target_user_id: String,
        muted: bool,
        /// Display name of whoever set it. Empty when unknown — after a restart we
        /// know a peer is muted but no longer who muted them, and saying "Muted"
        /// is honest where a remembered name would be a guess.
        by: String,
    },
    /// Session 29b: a peer asked for the music to stop.
    ///
    /// The room does not act on this — only the host's client owns a bot session,
    /// so only the host's client can stop one. The room's whole job here is to
    /// carry the request and its actor up to whoever *can* act, which is also why
    /// this is a distinct event rather than a reuse of `PeerState`: a stop is
    /// something that happened, not a description of how things are.
    MusicStopRequested {
        /// Display name of whoever asked. Empty when we have no roster entry for
        /// them, which the UI renders as a nameless "somebody" rather than
        /// inventing a name.
        by: String,
    },
}

/// Session 12: one remote participant. The relay fans out to every peer in the
/// room, so a single session carries audio for all of them and each datagram is
/// attributed to its sender.
#[derive(Debug, Clone, Serialize)]
pub struct PeerInfo {
    pub uuid: String,
    /// Session 29: the peer's PERSISTENT id, learned from their `Hello`. Empty
    /// until that arrives, or for a peer whose build predates the field.
    ///
    /// This is what a room-wide mute targets. `uuid` cannot be: it is the session
    /// id and changes on every relaunch, so a mute persisted against it would
    /// never match the same person again.
    pub user_id: String,
    pub name: String,
    pub speaking: bool,
    pub mute: bool,
    pub deafen: bool,
    /// Session 14: this peer is playing music into the room.
    pub music: bool,
    pub music_title: String,
    /// Session 29b: this peer is a music bot session rather than a person.
    ///
    /// The host's voice peer and their bot peer share a `user_id`, so this is the
    /// only thing that separates them — and it is declared, never inferred.
    pub is_bot: bool,
    /// Session 29b: this bot accepts a remote stop — the host's "Force Stop"
    /// setting, as it stood when the bot was spawned.
    ///
    /// Meaningful only when `is_bot`. The UI reads the two together: it offers a
    /// Stop control only on a peer that is a bot *and* accepts one, so a peer whose
    /// host disabled force-stop is not shown a button that would do nothing.
    ///
    /// Always `false` for a person, where nothing reads it.
    pub force_stop: bool,
}

/// Session 29: one room-wide mute, as the UI sees it.
///
/// Kept out of [`PeerInfo`] on purpose. A tile already carries the *local* volume
/// mute this user set for themselves; folding the room mute into the same field
/// would leave the UI unable to say which of the two it is showing, and the two
/// have different owners and different lifetimes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MuteInfo {
    /// The silenced peer's persistent id — match it against
    /// [`PeerInfo::user_id`], never against [`PeerInfo::uuid`].
    pub target_user_id: String,
    /// Who set it. Empty when unknown, e.g. restored from disk.
    pub by: String,
}

/// A half-finished invite: the offer was created and serialized into a blob; the
/// `Rtc`+socket wait in this state until the answer blob arrives.
struct PendingInvite {
    setup: SessionSetup,
    pending: SdpPendingOffer,
}

pub struct RoomManager {
    identity: Identity,
    state: RoomState,
    /// Session 12: every peer in the room, keyed by UUID (was a single peer_name).
    peers: std::collections::HashMap<String, PeerInfo>,
    pub room_code: Option<String>,
    pub transport_mode: Option<String>,
    embedded_host: Option<verio_transport::EmbeddedVoiceHost>,
    /// Shared with the pump threads so they can reach the active session.
    active: Arc<Mutex<Option<ActiveSession>>>,
    /// Outgoing encoded frames from the pipeline (receiver kept in a slot so a
    /// pipeline restart just swaps the receiver under the same pump).
    net_out: Arc<Mutex<Option<mpsc::Receiver<NetAudioOut>>>>,
    net_out_tx: mpsc::Sender<NetAudioOut>,
    /// Incoming frames slot the pipeline RX thread installs its sender into.
    net_in: Arc<Mutex<Option<Sender<NetAudioIn>>>>,
    /// Cached 2-byte Opus silence frame for the 400 ms keep-alive.
    silence_frame: Arc<Vec<u8>>,
    room_events: Sender<RoomEvent>,
    pending: Option<PendingInvite>,
    /// Session 11: set to make the VPS signalling listener leave the room and close
    /// its socket, so peers drop us immediately instead of waiting for a timeout.
    signaling_stop: Arc<std::sync::atomic::AtomicBool>,
    /// Session 14: last time each peer was heard from on the relay, so peers that
    /// vanish without a signalling event are still pruned.
    peer_seen: std::collections::HashMap<String, std::time::Instant>,
    /// Session 29: the room-wide mute — presence means "silenced in my mix", and
    /// the value names whoever set it.
    ///
    /// Keyed by the target's PERSISTENT user id, never their wire uuid: a uuid is
    /// minted per process, so a mute stored against it would fail to match the
    /// same person on their next launch, which is exactly when a persisted mute is
    /// supposed to still apply.
    ///
    /// This map is the single authority. [`Self::project_mutes`] derives the
    /// mixer's view from it; nothing derives it from the mixer.
    room_mutes: std::collections::HashMap<String, String>,
    /// Session 29: where [`Self::project_mutes`] writes the wire-id projection the
    /// audio mixer reads. `None` until a pipeline exists — the room is constructed
    /// first — and replaced on every pipeline restart, at which point the room
    /// re-derives the whole projection into the new set.
    mute_sink: Option<Arc<Mutex<std::collections::HashSet<String>>>>,
    /// Session 28: the local store, so a mute outlives the process.
    db: Db,
}

struct ActiveSession {
    handle: PeerHandle,
}

impl RoomManager {
    pub fn new(identity: Identity, room_events: Sender<RoomEvent>, db: Db) -> Self {
        let silence_frame = Arc::new(encode_silence_frame());
        tracing::info!(bytes = silence_frame.len(), "cached Opus silence keep-alive frame");
        let (net_out_tx, net_out_rx) = mpsc::channel::<NetAudioOut>();
        let mut room = Self {
            identity,
            state: RoomState::Idle,
            peers: std::collections::HashMap::new(),
            room_code: None,
            transport_mode: None,
            embedded_host: None,
            active: Arc::new(Mutex::new(None)),
            net_out: Arc::new(Mutex::new(Some(net_out_rx))),
            net_out_tx,
            net_in: Arc::new(Mutex::new(None)),
            silence_frame,
            room_events,
            pending: None,
            signaling_stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            peer_seen: std::collections::HashMap::new(),
            room_mutes: std::collections::HashMap::new(),
            mute_sink: None,
            db,
        };
        room.restore_mutes();
        room.spawn_audio_pump();
        room
    }

    /// Session 29: bring the persisted mutes back.
    ///
    /// Only the boolean is stored, so the actor is unknown here — recorded as an
    /// empty name, which the UI renders as a plain "Muted" rather than attributing
    /// the mute to somebody who never made it.
    fn restore_mutes(&mut self) {
        match self.db.muted_users() {
            Ok(ids) => {
                for id in ids {
                    self.room_mutes.insert(id, String::new());
                }
            }
            // Not fatal: the room still works, it just forgets mutes on restart.
            Err(e) => tracing::error!("could not restore the mute list: {e}"),
        }
    }

    // -- accessors -----------------------------------------------------------

    pub fn state(&self) -> RoomState {
        self.state
    }

    /// First peer's name (kept for the single-peer snapshot field).
    pub fn peer_name(&self) -> Option<&str> {
        self.peers.values().next().map(|p| p.name.as_str())
    }

    /// Session 12: snapshot of every peer in the room.
    #[must_use]
    pub fn peers(&self) -> Vec<PeerInfo> {
        let mut v: Vec<PeerInfo> = self.peers.values().cloned().collect();
        v.sort_by(|a, b| a.uuid.cmp(&b.uuid));
        v
    }

    /// Session 14: note that a peer is alive (any relay traffic counts).
    pub fn mark_peer_seen(&mut self, uuid: &str) {
        self.peer_seen
            .insert(uuid.to_string(), std::time::Instant::now());
    }

    /// Session 14: re-announce ourselves through the relay. Peers that joined while
    /// our signalling listener was busy still learn about us this way, and it also
    /// propagates a nickname change live.
    pub fn send_hello(&mut self) {
        if let Some(active) = self.active.lock().expect("active lock").as_ref() {
            active.handle.send_control(ControlMessage::Hello {
                name: self.identity.name.clone(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                user_id: self.identity.user_id.clone(),
                // Session 29b: carried on every re-announce, not just the first, so
                // a peer who joined after us still learns what we are.
                is_bot: self.identity.is_bot,
                // Session 29b Force Stop: sent for completeness rather than because
                // a person's value means anything. The room's own session is a voice
                // peer, so this is always `false`; a bot announces its own through
                // `music_bot.rs`, on a session of its own.
                force_stop: self.identity.force_stop,
            });
        }
    }

    /// Session 14: drop peers we have not heard from within `max_age`. This is what
    /// removes a stale card when someone leaves without a clean signalling event.
    pub fn prune_stale_peers(&mut self, max_age: std::time::Duration) {
        let now = std::time::Instant::now();
        let stale: Vec<String> = self
            .peer_seen
            .iter()
            .filter(|(_, seen)| now.duration_since(**seen) > max_age)
            .map(|(uuid, _)| uuid.clone())
            .collect();
        for uuid in stale {
            self.peer_seen.remove(&uuid);
            // Read before the removal takes the entry with it.
            let was_bot = self.peers.get(&uuid).is_some_and(|p| p.is_bot);
            if self.peers.remove(&uuid).is_some() {
                self.emit(RoomEvent::PeerDisconnected {
                    reason: "peer timed out".into(),
                    peer_id: Some(uuid),
                    is_bot: was_bot,
                });
            }
        }
    }

    /// Session 12: a peer arrived (from signalling or a relay hello) — tell the UI.
    pub fn add_peer(&mut self, peer: Identity) {
        self.upsert_peer(peer, true);
    }

    /// Session 27: put a peer in the roster **without** announcing an arrival.
    ///
    /// Two callers: the roster the server lists when we join a populated room,
    /// and a peer whose display name changed. In both cases nobody joined —
    /// either *we* did, or they were already here. Announcing these played one
    /// join cue per person already present, so joining a six-person call greeted
    /// you with six chimes while the one person who actually joined heard
    /// nothing of their own arrival.
    pub fn adopt_peer(&mut self, peer: Identity) {
        self.upsert_peer(peer, false);
    }

    /// Shared body of [`add_peer`] / [`adopt_peer`].
    ///
    /// Every peer that reaches the roster is announced exactly once, so a
    /// consumer that builds its peer list from these events never misses one.
    /// `arrived` is what separates "someone joined" from "here is who is here".
    fn upsert_peer(&mut self, peer: Identity, arrived: bool) {
        let uuid = peer.uuid.to_string();
        let is_new = !self.peers.contains_key(&uuid);
        self.peer_seen
            .insert(uuid.clone(), std::time::Instant::now());
        let entry = self.peers.entry(uuid.clone()).or_insert_with(|| PeerInfo {
            uuid: uuid.clone(),
            user_id: peer.user_id.clone(),
            name: peer.name.clone(),
            speaking: false,
            mute: false,
            deafen: false,
            music: false,
            music_title: String::new(),
            is_bot: peer.is_bot,
            // Session 29b Force Stop: read from the identity that arrived, not from
            // any local setting — the host's decision belongs to the host, and a
            // listener's own preference must never stand in for it.
            force_stop: peer.force_stop,
        });
        entry.name = peer.name.clone();
        // An empty id means "unknown" (a peer built before Session 29), so it must
        // never erase an id we already learned.
        if !peer.user_id.is_empty() {
            entry.user_id = peer.user_id.clone();
        }
        // Session 29b: the same asymmetry for the bot flag. Only a relay `Hello`
        // can declare a bot — a signalling-sourced identity never can, because the
        // server's `PeerIdentity` has no such field and serde drops it — so a
        // `false` arriving down that path must not clear a `true` we already
        // learned. A bot also cannot stop being a bot; its session simply ends,
        // and the roster entry goes with it.
        if peer.is_bot {
            entry.is_bot = true;
        }
        // Session 29b Force Stop: the same asymmetry, for the same reason. Only a
        // relay `Hello` can carry the host's decision — a signalling identity never
        // can, because the server's `PeerIdentity` has no such field — so a `false`
        // arriving down that path must not clear a `true` we already learned. Unlike
        // a mute, this value cannot change during a session: the host's setting is
        // applied when the bot is spawned. So ratcheting it one way loses nothing.
        if peer.force_stop {
            entry.force_stop = true;
        }
        let is_bot = entry.is_bot;
        let force_stop = entry.force_stop;
        // `!arrived` forces the event even for a peer we already know, because
        // an adopt is the only thing that tells the UI about a rename.
        if is_new || !arrived {
            self.emit(RoomEvent::PeerConnected {
                name: peer.name,
                uuid,
                user_id: peer.user_id,
                is_bot,
                force_stop,
                arrived,
            });
        }
        // Session 29: a peer who arrives already silenced — because we muted them
        // before, or before we restarted — must be gated from their very first
        // packet, so the projection follows the roster.
        self.project_mutes();
    }

    /// Session 12: drop a peer and tell the UI; the room idles when it empties.
    pub fn remove_peer(&mut self, uuid: &str) {
        // Session 29b: a bot is not a participant, so its departure must not count
        // as "everybody left".
        //
        // A host alone in a room has an empty roster, and the only way the roster
        // ever becomes non-empty for them is their own bot joining it. So without
        // this flag the teardown below fires the moment they stop the music: they
        // get thrown out of their own call for having played a track. Read before
        // the removal, because the flag lives on the entry being removed.
        let was_bot = self.peers.get(uuid).is_some_and(|p| p.is_bot);
        if self.peers.remove(uuid).is_some() {
            self.emit(RoomEvent::PeerDisconnected {
                reason: "peer left room".into(),
                peer_id: Some(uuid.to_string()),
                is_bot: was_bot,
            });
        }
        if !was_bot && self.peers.is_empty() {
            self.teardown_session("all peers left");
            self.set_state(RoomState::Idle);
        }
        // Session 29: the gate mirrors the roster, so it shrinks with it. The
        // *mute* does not: it is keyed on the person and stays in the list, ready
        // to apply again the moment they come back.
        self.project_mutes();
    }

    pub fn transport_mode(&self) -> Option<&str> {
        self.transport_mode.as_deref()
    }

    pub fn diagnostics(&self) -> Option<verio_transport::TransportDiagnostics> {
        let active = self.active.lock().expect("active lock");
        active.as_ref().map(|a| a.handle.diagnostics())
    }

    /// A connection/invite flow is in progress or a peer is connected.
    pub fn is_busy(&self) -> bool {
        matches!(self.state, RoomState::Connecting | RoomState::Connected)
    }

    /// Sender the pipeline's processing thread pushes outgoing frames into.
    pub fn net_out_tx(&self) -> mpsc::Sender<NetAudioOut> {
        self.net_out_tx.clone()
    }

    /// Called on every pipeline (re)start: the RX thread's sender for incoming
    /// network audio (`None` when the pipeline is stopped).
    pub fn set_net_in(&self, tx: Option<Sender<NetAudioIn>>) {
        *self.net_in.lock().expect("net_in lock") = tx;
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    // -- event helpers -------------------------------------------------------

    fn emit(&self, ev: RoomEvent) {
        let _ = self.room_events.send(ev);
    }

    fn set_state(&mut self, state: RoomState) {
        if self.state != state {
            tracing::info!(?state, "room state");
            self.state = state;
            self.emit(RoomEvent::StateChanged { state });
        }
    }

    fn begin_connecting(&mut self) {
        self.set_state(RoomState::Connecting);
    }

    fn fail(&mut self, message: String) {
        tracing::error!(%message, "room connect failed");
        self.emit(RoomEvent::Error { message: message.clone() });
        self.teardown_session("failed");
        self.set_state(RoomState::Idle);
    }

    fn teardown_session(&mut self, reason: &str) {
        // Leave the room on the server at once (see run_listener).
        self.signaling_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(active) = self.active.lock().expect("active lock").take() {
            active.handle.disconnect();
        }
        if let Some(mut host) = self.embedded_host.take() {
            host.stop();
        }
        self.pending = None;
        self.peers.clear();
        self.peer_seen.clear();
        self.room_code = None;
        self.transport_mode = None;
        let _ = reason;
    }

    pub fn room_code(&self) -> Option<&str> {
        self.room_code.as_deref()
    }

        // -- pumps / control -----------------------------------------------------

    /// Permanent audio pump: forwards encoded frames from the pipeline channel
    /// to the active peer's transport. Runs for the whole app lifetime; when no
    /// session is active it idles. Uses only the lock-shielded slots (never the
    /// room lock), so it cannot deadlock with commands.
    pub fn spawn_audio_pump(&self) {
        let active = Arc::clone(&self.active);
        let net_out = Arc::clone(&self.net_out);
        let spawned = std::thread::Builder::new()
            .name("verio-audio-pump".into())
            .spawn(move || {
                use std::sync::mpsc::RecvTimeoutError;
                loop {
                    let job = {
                        let a = active.lock().expect("active lock");
                        a.as_ref().map(|s| s.handle.clone())
                    };
                    if let Some(handle) = job {
                        let pkt = {
                            let guard = net_out.lock().expect("net_out lock");
                            match guard.as_ref() {
                                Some(rx) => rx.recv_timeout(std::time::Duration::from_millis(250)),
                                None => {
                                    drop(guard);
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                    continue;
                                }
                            }
                        };
                        match pkt {
                            Ok(p) => handle.send_audio(p.capture_ts_ms, p.opus),
                            Err(RecvTimeoutError::Timeout) => {}
                            // Pipeline (re)started — senders dropped. The slot
                            // gets a fresh receiver; wait for it.
                            Err(RecvTimeoutError::Disconnected) => {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                    } else {
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            });
        if let Err(e) = spawned {
            tracing::error!("spawn audio pump failed: {e}");
        }
    }

    /// Forward local speaking/mute/deafen state to the peer (control channel).
    pub fn send_state(
        &mut self,
        speaking: bool,
        mute: bool,
        deafen: bool,
        music: bool,
        music_title: String,
    ) {
        if let Some(active) = self.active.lock().expect("active lock").as_ref() {
            active.handle.send_control(ControlMessage::State {
                speaking,
                mute,
                deafen,
                music,
                music_title,
            });
        }
    }

    /// User-requested disconnect (or invite cancel).
    pub fn disconnect(&mut self) {
        self.teardown_session("user requested");
        self.set_state(RoomState::Idle);
    }

    /// The pipeline RX thread installs its sender here on every (re)start.
    /// Returns the outgoing sender for the pipeline config while we're at it.
    pub fn pipeline_endpoints(&self) -> mpsc::Sender<NetAudioOut> {
        self.net_out_tx.clone()
    }

    // -- room-wide mute (Session 29) -----------------------------------------

    /// Install the mixer's mute gate, then re-derive it from the current state.
    ///
    /// Called once per pipeline start. A restart hands us a brand-new empty set, so
    /// re-projecting here is what stops an output-device change from silently
    /// lifting every mute.
    pub fn set_mute_sink(&mut self, sink: Arc<Mutex<std::collections::HashSet<String>>>) {
        self.mute_sink = Some(sink);
        self.project_mutes();
    }

    /// Session 29: the room-wide mutes, for the UI's initial snapshot.
    #[must_use]
    pub fn mutes(&self) -> Vec<MuteInfo> {
        let mut v: Vec<MuteInfo> = self
            .room_mutes
            .iter()
            .map(|(id, by)| MuteInfo {
                target_user_id: id.clone(),
                by: by.clone(),
            })
            .collect();
        v.sort_by(|a, b| a.target_user_id.cmp(&b.target_user_id));
        v
    }

    /// Session 29: silence (or un-silence) a peer for the whole room, and tell
    /// everyone else.
    ///
    /// # Errors
    /// The target's persistent id is empty. A peer whose `Hello` we never saw has
    /// nothing a mute can be keyed on, and muting their wire uuid instead would
    /// quietly stop working the next time they launch.
    pub fn set_room_mute(&mut self, target_user_id: &str, muted: bool) -> Result<(), String> {
        if target_user_id.trim().is_empty() {
            return Err("cannot mute a peer whose persistent id is unknown".into());
        }
        let by = self.identity.name.clone();
        self.apply_mute(target_user_id, muted, &by, true);
        Ok(())
    }

    /// Session 29b Force Stop: ask this room's music host to stop.
    ///
    /// Sends the request and returns. The room deliberately does **not** act on it:
    /// only the client that owns the bot session can end one, so this is a request
    /// and not a command, and whether it is honoured is the host's decision — one
    /// announced when the bot joined, not renegotiated now.
    ///
    /// Nothing here says who is asking. The relay stamps the actor onto every frame,
    /// and that is the whole reason the payload is empty: the host is told who
    /// stopped their music, and a request that named its own sender would let anyone
    /// blame anyone.
    ///
    /// # Errors
    /// We are not in a room, or the room has no live transport.
    pub fn request_music_stop(&self) -> Result<(), String> {
        let active = self.active.lock().expect("active lock");
        let Some(active) = active.as_ref() else {
            return Err("Join a room before stopping its music.".into());
        };
        active.handle.send_control(ControlMessage::MusicStop);
        Ok(())
    }

    /// The single path every mute takes, whoever it came from.
    ///
    /// `broadcast` is false for a mute we just received: echoing it back would
    /// bounce it around the room for as long as the room exists.
    fn apply_mute(&mut self, target_user_id: &str, muted: bool, by: &str, broadcast: bool) {
        // Reachable from the wire, not just from our own command: a peer running a
        // build without a persistent id, or a malformed frame, can name no target.
        // Dropping it beats storing a mute that matches nobody and would be
        // restored, uselessly, on every future launch.
        if target_user_id.trim().is_empty() {
            tracing::warn!(by, "ignoring a room mute that names no target");
            return;
        }
        if muted {
            self.room_mutes
                .insert(target_user_id.to_string(), by.to_string());
        } else {
            self.room_mutes.remove(target_user_id);
        }
        self.project_mutes();
        // A failed write costs the mute its survival across launches, not its
        // effect now, so it is logged rather than propagated.
        if let Err(e) = self.db.set_muted(target_user_id, muted) {
            tracing::error!(target_user_id, "could not persist the mute: {e}");
        }
        if broadcast {
            if let Some(active) = self.active.lock().expect("active lock").as_ref() {
                active.handle.send_control(ControlMessage::MuteUser {
                    target_user_id: target_user_id.to_string(),
                    muted,
                });
            }
        }
        self.emit(RoomEvent::PeerMuted {
            target_user_id: target_user_id.to_string(),
            muted,
            by: by.to_string(),
        });
    }

    /// Derive the mixer's gate from the authoritative mute set plus the roster.
    ///
    /// Recomputed whole rather than patched: the set is a handful of entries, and a
    /// full derivation cannot drift out of step with its source the way an
    /// add/remove pair can when a peer joins, leaves, or is muted while away.
    ///
    /// Note what is deliberately absent — a check for "is this me?". The mixer only
    /// gates the peers in its remote-audio map and a client never receives its own
    /// audio back, so a self-mute cannot silence the muter. That is true by
    /// construction, and a guard for it could never fire.
    fn project_mutes(&self) {
        let Some(sink) = self.mute_sink.as_ref() else {
            return;
        };
        let Ok(mut gate) = sink.lock() else {
            tracing::error!("mute gate poisoned — mutes not applied");
            return;
        };
        gate.clear();
        for (uuid, peer) in &self.peers {
            if peer.user_id.is_empty() {
                continue;
            }
            if self.room_mutes.contains_key(&peer.user_id) {
                gate.insert(uuid.clone());
            }
        }
    }

    // -- session activation --------------------------------------------------

    /// Shared tail of every connect flow: put the session live and spawn its
    /// transport-event pump.
    fn activate_session(
        room: &Arc<Mutex<RoomManager>>,
        setup: SessionSetup,
        remote: Identity,
    ) -> Result<(), String> {
        let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();
        let silence = {
            let m = room.lock().expect("room lock");
            Arc::clone(&m.silence_frame)
        };
        let handle =
            spawn_session(setup, remote.clone(), silence, event_tx).map_err(|e| e.0)?;
        {
            let mut m = room.lock().expect("room lock");
            m.add_peer(remote.clone());
            m.transport_mode = Some("direct_p2p".to_string());
            m.set_state(RoomState::Connected);
            m.emit(RoomEvent::TransportModeChanged { mode: "direct_p2p".to_string() });
            *m.active.lock().expect("active lock") = Some(ActiveSession { handle });
        }
        spawn_transport_pump(Arc::clone(room), event_rx, remote);
        Ok(())
    }

    /// Activate a voice session over the fallback UDP relay or peer-host.
    fn activate_relay_session(
        room: &Arc<Mutex<RoomManager>>,
        relay_addr: SocketAddr,
        fallback_addrs: Vec<SocketAddr>,
        room_code: String,
        remote: Identity,
        mode_label: &'static str,
    ) -> Result<(), String> {
        let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();
        let (silence, local_id) = {
            let m = room.lock().expect("room lock");
            (Arc::clone(&m.silence_frame), m.identity.clone())
        };
        let handle = verio_transport::spawn_relay_session_with_candidates(
            relay_addr,
            fallback_addrs,
            room_code,
            local_id,
            remote.clone(),
            silence,
            event_tx,
            mode_label,
        ).map_err(|e| e.0)?;
        {
            let mut m = room.lock().expect("room lock");
            m.add_peer(remote.clone());
            m.transport_mode = Some(mode_label.to_string());
            m.set_state(RoomState::Connected);
            m.emit(RoomEvent::TransportModeChanged { mode: mode_label.to_string() });
            // `add_peer` above already announced this peer if it was new. A
            // second `PeerConnected` here fired a duplicate join cue for the same
            // peer on every relay call; a peer adopted from the server's roster
            // was announced by that adopt, so nothing is lost by dropping it.
            *m.active.lock().expect("active lock") = Some(ActiveSession { handle });
        }
        spawn_transport_pump(Arc::clone(room), event_rx, remote);
        Ok(())
    }

    // -- connect flows (blocking — run on worker threads) --------------------

    /// Direct connect by `ip:port` code (client side of the TCP handshake).
    pub fn run_connect_direct(room: &Arc<Mutex<RoomManager>>, addr: String) {
        {
            let mut m = room.lock().expect("room lock");
            if m.is_busy() {
                m.emit(RoomEvent::Error {
                    message: "a connection is already in progress".into(),
                });
                return;
            }
            m.begin_connecting();
        }
        let result = (|| -> Result<(), String> {
            tracing::info!(%addr, "direct connect: dialing");
            let session = DirectSession::connect(&addr).map_err(|e| e.to_string())?;
            Self::run_signaling_session(room, session, false, None)
        })();
        if let Err(e) = result {
            room.lock().expect("room lock").fail(e);
        }
    }

    /// Listener side: a signaling connection already accepted by the TCP
    /// listener. The identity exchange decides the role exactly as when we
    /// dialed (LARGER uuid = offerer) — dialing direction is irrelevant.
    pub fn run_accept_direct(room: &Arc<Mutex<RoomManager>>, session: DirectSession) {
        {
            let mut m = room.lock().expect("room lock");
            if m.is_busy() {
                // A second incoming dial while busy: drop it silently.
                tracing::warn!("incoming direct connection ignored — room busy");
                return;
            }
            m.begin_connecting();
        }
        if let Err(e) = Self::run_signaling_session(room, session, false, None) {
            room.lock().expect("room lock").fail(e);
        }
    }

    /// Helper to resolve the VPS UDP STUN addresses (standard port 3478 preferred, then configured UDP port).
    fn resolve_vps_stuns(vps_addr: &str) -> Vec<SocketAddr> {
        let (host, _tcp, udp_port) = verio_discovery::vps::extract_host_and_ports(vps_addr);
        let mut stuns = Vec::new();
        // 1. Try standard STUN port 3478 (whitelisted for VoIP/WebRTC)
        if let Ok(mut it) = format!("{host}:3478").to_socket_addrs() {
            if let Some(addr) = it.next() {
                stuns.push(addr);
            }
        }
        // 2. Try configured UDP relay/STUN port
        if let Ok(mut it) = format!("{host}:{udp_port}").to_socket_addrs() {
            if let Some(addr) = it.next() {
                if !stuns.contains(&addr) {
                    stuns.push(addr);
                }
            }
        }
        stuns
    }

    /// Helper to resolve all candidate VPS UDP Relay addresses (standard port 3478 + configured UDP port).
    pub fn resolve_vps_relay_candidates(vps_addr: &str) -> Result<Vec<SocketAddr>, String> {
        let (host, _tcp, udp_port) = verio_discovery::vps::extract_host_and_ports(vps_addr);
        let mut candidates = Vec::new();
        // 1. Try standard VoIP port 3478 (whitelisted across firewalls/NATs)
        if let Ok(mut it) = format!("{host}:3478").to_socket_addrs() {
            if let Some(addr) = it.next() {
                candidates.push(addr);
            }
        }
        // 2. Try configured UDP port
        if let Ok(mut it) = format!("{host}:{udp_port}").to_socket_addrs() {
            if let Some(addr) = it.next() {
                if !candidates.contains(&addr) {
                    candidates.push(addr);
                }
            }
        }
        if candidates.is_empty() {
            Err(format!("failed to resolve VPS relay address for {host}"))
        } else {
            Ok(candidates)
        }
    }

    /// Helper to resolve the VPS UDP Relay address (port 3478 preferred, then configured UDP port).
    pub fn resolve_vps_relay(vps_addr: &str) -> Result<SocketAddr, String> {
        let candidates = Self::resolve_vps_relay_candidates(vps_addr)?;
        Ok(candidates[0])
    }

    /// Create a room on the VPS signaling server and wait for a peer to join.
    pub fn run_create_room(
        room: &Arc<Mutex<RoomManager>>,
        vps_addr: String,
        transport_mode: crate::settings::TransportModeSetting,
        host_port: u16,
    ) -> Result<String, String> {
        let identity = {
            let mut m = room.lock().expect("room lock");
            if m.is_busy() {
                return Err("a connection is already in progress".into());
            }
            m.begin_connecting();
            m.identity.clone()
        };
        let (session, code) = verio_discovery::VpsSignaling::create(&vps_addr, &identity)
            .map_err(|e| Self::fail_flow(room, e.to_string()))?;
        {
            let mut m = room.lock().expect("room lock");
            m.room_code = Some(code.clone());
            m.emit(RoomEvent::RoomCreated { code: code.clone() });
        }
        let room_clone = Arc::clone(room);
        let vps_addr_clone = vps_addr.clone();
        let code_clone = code.clone();
        std::thread::Builder::new()
            .name("verio-vps-signaling".into())
            .spawn(move || {
                if let Err(e) = Self::run_vps_room_session(
                    &room_clone,
                    session,
                    &vps_addr_clone,
                    &code_clone,
                    transport_mode,
                    host_port,
                    true,
                ) {
                    room_clone.lock().expect("room lock").fail(e);
                }
            })
            .map_err(|e| Self::fail_flow(room, format!("spawn signaling thread: {e}")))?;
        Ok(code)
    }

    /// Join an existing room code on the VPS signaling server.
    pub fn run_join_room(
        room: &Arc<Mutex<RoomManager>>,
        vps_addr: String,
        code: String,
        transport_mode: crate::settings::TransportModeSetting,
    ) -> Result<(), String> {
        let identity = {
            let mut m = room.lock().expect("room lock");
            if m.is_busy() {
                return Err("a connection is already in progress".into());
            }
            m.begin_connecting();
            m.room_code = Some(code.clone());
            m.identity.clone()
        };
        let session = verio_discovery::VpsSignaling::join(&vps_addr, &code, &identity)
            .map_err(|e| Self::fail_flow(room, e.to_string()))?;
        {
            let m = room.lock().expect("room lock");
            m.emit(RoomEvent::RoomJoined { code: code.clone() });
        }
        let room_clone = Arc::clone(room);
        let vps_addr_clone = vps_addr.clone();
        let code_clone = code.clone();
        std::thread::Builder::new()
            .name("verio-vps-signaling".into())
            .spawn(move || {
                if let Err(e) = Self::run_vps_room_session(
                    &room_clone,
                    session,
                    &vps_addr_clone,
                    &code_clone,
                    transport_mode,
                    8444,
                    false,
                ) {
                    room_clone.lock().expect("room lock").fail(e);
                }
            })
            .map_err(|e| Self::fail_flow(room, format!("spawn signaling thread: {e}")))?;
        Ok(())
    }

    /// Unified VPS room session orchestrator.
    #[allow(clippy::too_many_arguments)]
    fn run_vps_room_session(
        room: &Arc<Mutex<RoomManager>>,
        mut session: verio_discovery::VpsSignaling,
        vps_addr: &str,
        room_code: &str,
        transport_mode: crate::settings::TransportModeSetting,
        host_port: u16,
        is_creator: bool,
    ) -> Result<(), String> {
        let identity = {
            let m = room.lock().expect("room lock");
            m.identity.clone()
        };
        let (remote, role) = session
            .exchange_identities(&identity)
            .map_err(|e| e.to_string())?;

        tracing::info!(
            peer = %remote.name,
            uuid = %remote.uuid,
            ?transport_mode,
            is_creator,
            "signaling: peer identified, selecting voice transport"
        );

        // Session 11: the VPS relay is the only transport for room calls. The old
        // transport-mode picker and the force-relay switch are gone, so a room call
        // always resolves to the relay path (ICE/STUN gathering and the P2P
        // handshake are no longer reachable from the 4-digit room flow).
        // Session 12: adopt the peers the server already listed for this room.
        // Session 27: `adopt_peer`, not `add_peer` — these peers did not join,
        // *we* did, and announcing each of them played a join cue per head.
        for peer in session.initial_peers() {
            let mut m = room.lock().expect("room lock");
            m.adopt_peer(peer);
        }

        let effective_mode = crate::settings::TransportModeSetting::CloudRelay;
        let relay_label: &'static str = "cloud_relay";
        let res = match effective_mode {
            crate::settings::TransportModeSetting::CloudRelay | crate::settings::TransportModeSetting::Auto => {
                let relay_addrs = Self::resolve_vps_relay_candidates(vps_addr)?;
                let primary = relay_addrs[0];
                let fallbacks = relay_addrs[1..].to_vec();
                tracing::info!(%primary, ?fallbacks, "activating relay for room call");
                Self::activate_relay_session(room, primary, fallbacks, room_code.to_string(), remote, relay_label)
            }
            crate::settings::TransportModeSetting::PeerHost => {
                if is_creator {
                    let host = verio_transport::EmbeddedVoiceHost::start(host_port)
                        .map_err(|e| e.0)?;
                    let actual_port = host.local_port();
                    {
                        let mut m = room.lock().expect("room lock");
                        m.embedded_host = Some(host);
                    }
                    let host_addr: SocketAddr = format!("127.0.0.1:{actual_port}")
                        .parse()
                        .map_err(|e| format!("parse host addr: {e}"))?;
                    Self::activate_relay_session(room, host_addr, Vec::new(), room_code.to_string(), remote, "peer_host")
                } else {
                    let relay_addrs = Self::resolve_vps_relay_candidates(vps_addr)?;
                    let primary = relay_addrs[0];
                    let fallbacks = relay_addrs[1..].to_vec();
                    Self::activate_relay_session(room, primary, fallbacks, room_code.to_string(), remote, "peer_host")
                }
            }
            crate::settings::TransportModeSetting::DirectP2P => {
                let stun_addrs = Self::resolve_vps_stuns(vps_addr);
                Self::run_vps_p2p_handshake(room, &mut session, &stun_addrs, remote, role)
            }
        };

        if res.is_ok() {
            // Keep the WebSocket open on a helper thread so we hear about peers
            // leaving, and so leaving ourselves reaches the server immediately.
            let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
            {
                let mut m = room.lock().expect("room lock");
                m.signaling_stop = Arc::clone(&stop);
            }
            session.set_stop_flag(stop);

            // Session 14: roster gossip. Re-announce ourselves every few seconds so
            // peers learn about us even if a signalling event was missed, and prune
            // peers we stop hearing from so stale cards disappear.
            {
                let room_weak = Arc::downgrade(room);
                let spawned = std::thread::Builder::new()
                    .name("verio-roster".into())
                    .spawn(move || loop {
                        std::thread::sleep(std::time::Duration::from_secs(3));
                        let Some(r) = room_weak.upgrade() else {
                            return;
                        };
                        let mut m = r.lock().expect("room lock");
                        if matches!(m.state, RoomState::Idle) {
                            return;
                        }
                        m.send_hello();
                        m.prune_stale_peers(std::time::Duration::from_secs(15));
                    });
                if let Err(e) = spawned {
                    tracing::error!("spawn roster thread failed: {e}");
                }
            }
            let room_weak = Arc::downgrade(room);
            std::thread::Builder::new()
                .name("verio-vps-ws-listener".into())
                .spawn(move || {
                    session.run_listener(move |event| {
                        let Some(r) = room_weak.upgrade() else {
                            return;
                        };
                        match event {
                            verio_discovery::vps::SignalingEvent::PeerJoined(peer) => {
                                r.lock().expect("room lock").add_peer(peer);
                            }
                            verio_discovery::vps::SignalingEvent::PeerLeft(peer_id) => {
                                r.lock().expect("room lock").remove_peer(&peer_id);
                            }
                        }
                    });
                })
                .ok();
        }

        res
    }

    /// Pure direct WebRTC P2P handshake (str0m) for DirectP2P mode.
    fn run_vps_p2p_handshake(
        room: &Arc<Mutex<RoomManager>>,
        session: &mut verio_discovery::VpsSignaling,
        stun_addrs: &[SocketAddr],
        remote: Identity,
        role: Role,
    ) -> Result<(), String> {
        let mut setup = SessionSetup::with_custom_stuns(true, stun_addrs).map_err(|e| e.0)?;
        match role {
            Role::Offerer => {
                let (offer, pending) = setup.make_offer().map_err(|e| e.0)?;
                tracing::info!(peer = %remote.name, offer = %offer, "signaling: generated SDP offer");
                let answer = session.exchange_offer(&offer).map_err(|e| e.to_string())?;
                tracing::info!(peer = %remote.name, answer = %answer, "signaling: received SDP answer");
                setup.accept_answer(pending, &answer).map_err(|e| e.0)?;
                tracing::info!(peer = %remote.name, "signaling: offered, answer accepted");
            }
            Role::Answerer => {
                let offer = session.receive_offer().map_err(|e| e.to_string())?;
                tracing::info!(peer = %remote.name, offer = %offer, "signaling: received SDP offer");
                let answer = setup.accept_offer(&offer).map_err(|e| e.0)?;
                tracing::info!(peer = %remote.name, answer = %answer, "signaling: generated SDP answer");
                session.send_answer(&answer).map_err(|e| e.to_string())?;
                tracing::info!(peer = %remote.name, "signaling: answered");
            }
        }
        Self::activate_session(room, setup, remote)
    }

    /// Shared signaling handshake: identities → role → SDP exchange.
    fn run_signaling_session(
        room: &Arc<Mutex<RoomManager>>,
        mut session: impl Signaling,
        internet_mode: bool,
        custom_stun: Option<SocketAddr>,
    ) -> Result<(), String> {
        let identity = {
            let m = room.lock().expect("room lock");
            m.identity.clone()
        };
        let (remote, role) = session
            .exchange_identities(&identity)
            .map_err(|e| e.to_string())?;
        let mut setup = SessionSetup::with_custom_stun(internet_mode, custom_stun).map_err(|e| e.0)?;
        match role {
            Role::Offerer => {
                let (offer, pending) = setup.make_offer().map_err(|e| e.0)?;
                let answer = session.exchange_offer(&offer).map_err(|e| e.to_string())?;
                setup.accept_answer(pending, &answer).map_err(|e| e.0)?;
                tracing::info!(peer = %remote.name, "signaling: offered, answer accepted");
            }
            Role::Answerer => {
                let offer = session.receive_offer().map_err(|e| e.to_string())?;
                let answer = setup.accept_offer(&offer).map_err(|e| e.0)?;
                session.send_answer(&answer).map_err(|e| e.to_string())?;
                tracing::info!(peer = %remote.name, "signaling: answered");
            }
        }
        Self::activate_session(room, setup, remote)
    }


    /// Step 1 of the invite flow: create the offer blob (STUN-gathers the
    /// srflx candidate first — non-trickle). Leaves the session pending until
    /// [`Self::run_complete_invite`].
    pub fn run_create_invite(room: &Arc<Mutex<RoomManager>>) -> Result<String, String> {
        {
            let mut m = room.lock().expect("room lock");
            if m.is_busy() {
                return Err("a connection is already in progress".into());
            }
            m.begin_connecting();
        }
        let identity = {
            let m = room.lock().expect("room lock");
            m.identity.clone()
        };
        tracing::info!("invite: building session (STUN gathering)");
        let mut setup = match SessionSetup::new(true) {
            Ok(s) => s,
            Err(e) => return Err(Self::fail_flow(room, e.0)),
        };
        let (offer, pending) = match setup.make_offer() {
            Ok(v) => v,
            Err(e) => return Err(Self::fail_flow(room, e.0)),
        };
        let blob = create_invite(&identity, &offer);
        {
            let mut m = room.lock().expect("room lock");
            m.pending = Some(PendingInvite { setup, pending });
        }
        tracing::info!("invite: offer blob created (waiting for the answer blob)");
        Ok(blob)
    }

    /// Shared: record the failure (Error event + back to Idle) and pass the
    /// message on as the command's Err.
    fn fail_flow(room: &Arc<Mutex<RoomManager>>, message: String) -> String {
        room.lock().expect("room lock").fail(message.clone());
        message
    }

    /// Step 3 of the invite flow: apply the friend's answer blob.
    pub fn run_complete_invite(room: &Arc<Mutex<RoomManager>>, blob: String) -> Result<(), String> {
        let payload = parse_invite(&blob).map_err(|e| e.to_string())?;
        if payload.kind != InviteKind::Answer {
            return Err(
                "that is an OFFER blob — paste the friend's ANSWER blob here".to_string(),
            );
        }
        let PendingInvite { mut setup, pending } = {
            let mut m = room.lock().expect("room lock");
            m.pending
                .take()
                .ok_or_else(|| "no invite in progress — create one first".to_string())?
        };
        if let Err(e) = setup.accept_answer(pending, &payload.sdp) {
            return Err(Self::fail_flow(room, e.0));
        }
        tracing::info!(peer = %payload.identity.name, "invite: answer accepted");
        if let Err(e) = Self::activate_session(room, setup, payload.identity) {
            return Err(Self::fail_flow(room, e));
        }
        Ok(())
    }

    /// Friend side (step 2 of the invite flow): accept the offer blob, answer,
    /// go live immediately, and return the answer blob for the inviter.
    pub fn run_accept_invite(
        room: &Arc<Mutex<RoomManager>>,
        blob: String,
    ) -> Result<String, String> {
        {
            let mut m = room.lock().expect("room lock");
            if m.is_busy() {
                return Err("a connection is already in progress".into());
            }
            m.begin_connecting();
        }
        // Bad blob: nothing was started — back to Idle, return the error
        // directly (no Error event storm).
        let back_to_idle = |room: &Arc<Mutex<RoomManager>>| {
            room.lock().expect("room lock").set_state(RoomState::Idle);
        };
        let payload = match parse_invite(&blob) {
            Ok(p) => p,
            Err(e) => {
                back_to_idle(room);
                return Err(e.to_string());
            }
        };
        if payload.kind != InviteKind::Offer {
            back_to_idle(room);
            return Err("that is an ANSWER blob — paste an OFFER blob here".to_string());
        }
        let identity = {
            let m = room.lock().expect("room lock");
            m.identity.clone()
        };
        tracing::info!(peer = %payload.identity.name, "invite: accepting offer (STUN gathering)");
        let mut setup = match SessionSetup::new(true) {
            Ok(s) => s,
            Err(e) => return Err(Self::fail_flow(room, e.0)),
        };
        let answer = match setup.accept_offer(&payload.sdp) {
            Ok(a) => a,
            Err(e) => return Err(Self::fail_flow(room, e.0)),
        };
        let answer_blob = create_answer_invite(&identity, &answer);
        if let Err(e) = Self::activate_session(room, setup, payload.identity) {
            return Err(Self::fail_flow(room, e));
        }
        tracing::info!("invite: answered and live — paste the answer blob back");
        Ok(answer_blob)
    }
}

/// Per-session thread: transport events → room events + state transitions.
/// (`remote` is kept in the signature for context; the wire `hello` carries
/// the authoritative display name.)
/// Session 12: relay events carry the sender; single-peer transports do not, in
/// which case the session's peer is the only candidate.
fn resolve_peer(event_peer: &str, fallback: &str) -> String {
    if event_peer.is_empty() {
        fallback.to_string()
    } else {
        event_peer.to_string()
    }
}

fn spawn_transport_pump(
    room: Arc<Mutex<RoomManager>>,
    event_rx: mpsc::Receiver<TransportEvent>,
    remote: Identity,
) {
    let peer_id = remote.uuid.to_string();
    let spawned = std::thread::Builder::new()
        .name("verio-room-pump".into())
        .spawn(move || {
            while let Ok(ev) = event_rx.recv() {
                match ev {
                    TransportEvent::Connected => {
                        let mut m = room.lock().expect("room lock");
                        m.set_state(RoomState::Connected);
                    }
                    TransportEvent::Audio {
                        peer,
                        seq,
                        capture_ts_ms,
                        opus,
                    } => {
                        let tx = {
                            let m = room.lock().expect("room lock");
                            let guard = m.net_in.lock().expect("net_in lock");
                            guard.clone()
                        };
                        if let Some(tx) = tx {
                            let _ = tx.send(NetAudioIn {
                                peer_id: resolve_peer(&peer, &peer_id),
                                seq,
                                capture_ts_ms,
                                opus,
                            });
                        }
                    }
                    TransportEvent::Control {
                        peer,
                        message:
                            ControlMessage::State {
                                speaking,
                                mute,
                                deafen,
                                music,
                                music_title,
                            },
                    } => {
                        let mut m = room.lock().expect("room lock");
                        let id = resolve_peer(&peer, &peer_id);
                        m.mark_peer_seen(&id);
                        if let Some(p) = m.peers.get_mut(&id) {
                            p.speaking = speaking;
                            p.mute = mute;
                            p.deafen = deafen;
                            p.music = music;
                            p.music_title = music_title.clone();
                        }
                        m.emit(RoomEvent::PeerState {
                            peer_id: id,
                            speaking,
                            mute,
                            deafen,
                            music,
                            music_title,
                        });
                    }
                    TransportEvent::Control {
                        peer,
                        message:
                            ControlMessage::Hello {
                                name,
                                user_id,
                                is_bot,
                                force_stop,
                                ..
                            },
                    } => {
                        let mut m = room.lock().expect("room lock");
                        let uuid = resolve_peer(&peer, &peer_id);
                        m.mark_peer_seen(&uuid);
                        let is_new = !m.peers.contains_key(&uuid);
                        // Session 29: refresh the roster when the name OR the
                        // persistent id changed. The id matters because a room-wide
                        // mute is keyed on it, so learning it late must update the
                        // roster rather than be discarded as "not a rename". An
                        // empty id never overwrites a known one — that is a peer
                        // built before this field existed.
                        //
                        // Session 29b: the bot flag refreshes it for the same
                        // reason, and only in one direction — a bot is declared by
                        // its `Hello`, so a `false` here must not clear a `true`
                        // that arrived earlier. Without this clause a bot whose
                        // `Hello` landed before its signalling `PeerJoined` would
                        // settle back into looking like a person.
                        //
                        // The force-stop decision rides the same clause and the same
                        // direction: this message is the only thing that can teach
                        // it, and learning it late is precisely why the clause
                        // exists — the Stop control has to appear on a listener's
                        // screen when the bot's announcement arrives, not whenever
                        // its tile happened to.
                        let changed = m.peers.get(&uuid).is_some_and(|p| {
                            p.name != name
                                || (p.user_id != user_id && !user_id.is_empty())
                                || (is_bot && !p.is_bot)
                                || (force_stop && !p.force_stop)
                        });
                        if is_new || changed {
                            let identity = Identity {
                                uuid: verio_discovery::Uuid::parse_str(&uuid).unwrap_or_else(|_| {
                                    verio_discovery::Uuid::nil()
                                }),
                                user_id,
                                is_bot,
                                force_stop,
                                name,
                                version: String::new(),
                            };
                            if is_new {
                                m.add_peer(identity);
                            } else {
                                // A rename is a roster update, not an arrival.
                                // `adopt_peer` refreshes the name and tells the
                                // UI; emitting `PeerConnected` by hand here used
                                // to play a join cue every time a peer renamed
                                // themselves.
                                m.adopt_peer(identity);
                            }
                        }
                    }
                    TransportEvent::Control {
                        peer,
                        message: ControlMessage::MuteUser {
                            target_user_id,
                            muted,
                        },
                    } => {
                        let mut m = room.lock().expect("room lock");
                        let actor = resolve_peer(&peer, &peer_id);
                        m.mark_peer_seen(&actor);
                        // The actor comes from the envelope, not the payload: the
                        // relay stamps every control frame with its sender, so the
                        // message neither has to repeat it nor gets to lie about it.
                        let by = m
                            .peers
                            .get(&actor)
                            .map(|p| p.name.clone())
                            .unwrap_or_default();
                        // `broadcast: false` — this is the mute we just received.
                        // Sending it on would have it circulate for the life of the
                        // room.
                        m.apply_mute(&target_user_id, muted, &by, false);
                    }
                    TransportEvent::Control {
                        peer,
                        message: ControlMessage::MusicStop,
                    } => {
                        let mut m = room.lock().expect("room lock");
                        let actor = resolve_peer(&peer, &peer_id);
                        m.mark_peer_seen(&actor);
                        // The actor again comes from the envelope. This is the one
                        // that matters most: the host is told *who* stopped their
                        // music, so taking that from the payload would let anyone
                        // blame anyone.
                        let by = m
                            .peers
                            .get(&actor)
                            .map(|p| p.name.clone())
                            .unwrap_or_default();
                        m.emit(RoomEvent::MusicStopRequested { by });
                    }
                    TransportEvent::Rtt { ms } => {
                        let m = room.lock().expect("room lock");
                        m.emit(RoomEvent::Rtt { ms });
                    }
                    // Ping/Pong are answered inside the transport driver and
                    // never surfaced as room events.
                    // Ping/Pong are the liveness signal for the roster sweep.
                    TransportEvent::Control {
                        peer,
                        message: ControlMessage::Ping { .. } | ControlMessage::Pong { .. },
                    } => {
                        let mut m = room.lock().expect("room lock");
                        m.mark_peer_seen(&resolve_peer(&peer, &peer_id));
                    }
                    TransportEvent::Disconnected { reason } => {
                        let mut m = room.lock().expect("room lock");
                        // Only report a departure if this teardown is what ended
                        // the call. Both `disconnect()` (the user pressing Leave)
                        // and `remove_peer` (the last peer leaving) idle the room
                        // *before* the transport winds down, so without this guard
                        // the leave cue played when you left your own call, and
                        // twice for the last peer's already-reported departure.
                        let was_active = !matches!(m.state, RoomState::Idle);
                        // Read before the teardown clears the roster. The transport
                        // signs with whichever peer joined first, so for a host
                        // alone in a room that peer is their own bot.
                        let was_bot = m.peers.get(&peer_id).is_some_and(|p| p.is_bot);
                        m.teardown_session(&reason);
                        m.set_state(RoomState::Idle);
                        if was_active {
                            m.emit(RoomEvent::PeerDisconnected {
                                reason,
                                peer_id: Some(peer_id.clone()),
                                is_bot: was_bot,
                            });
                        }
                        break;
                    }
                }
            }
        });
    if let Err(e) = spawned {
        tracing::error!("spawn room pump failed: {e}");
    }
}





/// Encode the network keep-alive frame via the pipeline's encoder settings:
/// consecutive silent 20 ms frames until DTX collapses the packet (see
/// [`verio_dsp::codec::OpusEncoderWrapper::opus_silence_frame`]).
fn encode_silence_frame() -> Vec<u8> {
    match verio_dsp::codec::OpusEncoderWrapper::opus_silence_frame() {
        Ok(pkt) => pkt,
        Err(e) => {
            tracing::error!("silence frame generation failed: {e}");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name: &str, n: u128) -> Identity {
        Identity {
            uuid: verio_discovery::Uuid::from_u128(n),
            // Distinct per (name, n) so mute tests cannot accidentally collide.
            user_id: format!("user-{name}-{n}"),
            is_bot: false,
            force_stop: false,
            name: name.to_string(),
            version: String::new(),
        }
    }

    /// Session 29b: a music bot session. Same shape as a person's, except for the
    /// declaration — which is the whole point, since nothing else about it differs.
    ///
    /// `force_stop: true` matches the product default, so a test that says nothing
    /// about Force Stop exercises the ordinary case. Use
    /// [`unforceable_bot_identity`] for the host who turned it off.
    fn bot_identity(name: &str, n: u128, host_user_id: &str) -> Identity {
        Identity {
            uuid: verio_discovery::Uuid::from_u128(n),
            user_id: host_user_id.to_string(),
            is_bot: true,
            force_stop: true,
            name: name.to_string(),
            version: String::new(),
        }
    }

    /// Session 29b: a bot whose host disabled Force Stop. Identical to
    /// [`bot_identity`] but for the one field — which is the point, because
    /// everything else about the two sessions is indistinguishable, so the flag is
    /// the only thing a listener has to go on.
    fn unforceable_bot_identity(name: &str, n: u128, host_user_id: &str) -> Identity {
        Identity {
            force_stop: false,
            ..bot_identity(name, n, host_user_id)
        }
    }

    /// A private, empty local store per test. The room persists mutes through it,
    /// so the tests exercise the real write path rather than a stub that could not
    /// fail.
    ///
    /// The name is made unique per call, not per test: tests run in parallel and a
    /// shared directory has them deleting each other's database out from under an
    /// open connection ("database is locked"), which then leaks one test's mutes
    /// into the next.
    fn temp_db(name: &str) -> Db {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("verio-room-test-{name}-{n}"));
        let _ = std::fs::remove_dir_all(&dir);
        Db::open(&dir).expect("open test database")
    }

    fn manager() -> (RoomManager, mpsc::Receiver<RoomEvent>) {
        let (tx, rx) = mpsc::channel();
        (
            RoomManager::new(identity("self", 999), tx, temp_db("manager")),
            rx,
        )
    }

    /// `(name, arrived)` for every roster event drained so far.
    fn peer_events(rx: &mpsc::Receiver<RoomEvent>) -> Vec<(String, bool)> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerConnected { name, arrived, .. } = ev {
                out.push((name, arrived));
            }
        }
        out
    }

    // -- Session 27: which roster changes are *arrivals* ----------------------
    //
    // `arrived` is what the shell keys the join cue off, so each of these is
    // audible. Before this, joining a populated room played one cue per person
    // already in it, plus a duplicate for the transport's remote peer.

    #[test]
    fn adopting_the_roster_at_join_time_is_not_an_arrival() {
        let (mut m, rx) = manager();
        for (i, name) in ["Ada", "Bo", "Cy"].iter().enumerate() {
            m.adopt_peer(identity(name, i as u128 + 1));
        }
        let events = peer_events(&rx);
        assert_eq!(events.len(), 3, "the roster must still reach the UI");
        assert!(
            events.iter().all(|(_, arrived)| !arrived),
            "adopted peers must not be announced as arrivals: {events:?}"
        );
    }

    #[test]
    fn a_peer_who_actually_joins_is_an_arrival() {
        let (mut m, rx) = manager();
        m.adopt_peer(identity("Ada", 1));
        let _ = peer_events(&rx); // discard the adopt
        m.add_peer(identity("Bo", 2));
        assert_eq!(peer_events(&rx), vec![("Bo".to_string(), true)]);
    }

    #[test]
    fn re_adding_a_known_peer_does_not_re_announce() {
        let (mut m, rx) = manager();
        m.add_peer(identity("Ada", 1));
        let _ = peer_events(&rx);
        m.add_peer(identity("Ada", 1));
        assert!(
            peer_events(&rx).is_empty(),
            "a repeat add must not fire a second join cue"
        );
    }

    #[test]
    fn a_rename_updates_the_roster_without_announcing_an_arrival() {
        let (mut m, rx) = manager();
        m.add_peer(identity("Ada", 1));
        let _ = peer_events(&rx);
        m.adopt_peer(identity("Ada Lovelace", 1));
        assert_eq!(
            peer_events(&rx),
            vec![("Ada Lovelace".to_string(), false)],
            "the UI needs the new name, but nobody arrived"
        );
        let uuid = verio_discovery::Uuid::from_u128(1).to_string();
        assert_eq!(
            m.peers.get(&uuid).map(|p| p.name.as_str()),
            Some("Ada Lovelace")
        );
    }

    #[test]
    fn every_peer_that_reaches_the_roster_is_announced_once() {
        // The frontend builds its peer list from these events, so a peer in the
        // roster that was never announced would be invisible in the call — which
        // is the trap in "just stop emitting for adopted peers".
        let (mut m, rx) = manager();
        m.adopt_peer(identity("Ada", 1)); // pre-existing when we joined
        m.add_peer(identity("Bo", 2)); // arrives after us
        m.add_peer(identity("Bo", 2)); // repeat — must stay silent
        m.adopt_peer(identity("Cy", 3)); // later roster refresh
        let announced: Vec<String> = peer_events(&rx).into_iter().map(|(n, _)| n).collect();
        assert_eq!(announced, vec!["Ada", "Bo", "Cy"]);
        assert_eq!(m.peers.len(), 3);
    }

    #[test]
    fn an_empty_id_never_erases_one_we_already_learned() {
        // A peer built before Session 29 sends no `user_id`. If that empty string
        // overwrote a known id, the tile would lose its room-mute control the next
        // time the peer renamed themselves — a capability that disappears for a
        // reason nothing on screen explains.
        let (mut m, _rx) = manager();
        m.add_peer(identity("Bo", 2));
        assert_eq!(
            m.peers.get(&uuid(2)).map(|p| p.user_id.as_str()),
            Some("user-Bo-2")
        );

        m.adopt_peer(Identity {
            uuid: verio_discovery::Uuid::from_u128(2),
            user_id: String::new(),
            is_bot: false,
            force_stop: false,
            name: "Bo Renamed".to_string(),
            version: String::new(),
        });
        assert_eq!(
            m.peers.get(&uuid(2)).map(|p| p.user_id.as_str()),
            Some("user-Bo-2"),
            "the id we knew survived a Hello that carried none"
        );
        assert_eq!(
            m.peers.get(&uuid(2)).map(|p| p.name.as_str()),
            Some("Bo Renamed"),
            "…while the rename still took effect"
        );
    }

    #[test]
    fn a_peer_event_carries_the_persistent_id_a_room_mute_needs() {
        // The tile offers "mute for everyone" only once it holds an id to send, so
        // an event that dropped this field would leave the control absent with
        // nothing to explain why — the same shape of defect as the roster that
        // announced an arrival it never had.
        let (mut m, rx) = manager();
        m.add_peer(identity("Bo", 2));
        let mut seen = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerConnected { name, user_id, .. } = ev {
                seen.push((name, user_id));
            }
        }
        assert_eq!(seen, vec![("Bo".to_string(), "user-Bo-2".to_string())]);
    }

    // -- Session 29: the room-wide mute ---------------------------------------
    //
    // One thing these cannot reach: whether `apply_mute(broadcast: true)` actually
    // sends. With no live session there is no handle to send through, so the two
    // values are indistinguishable from in here. The receive path is covered
    // instead — it is the half that changes state.

    /// The mixer's mute gate: the set of wire ids currently silenced.
    type Gate = Arc<Mutex<std::collections::HashSet<String>>>;

    /// A gate to project into, plus a second handle to read back what was derived.
    fn sink() -> (Gate, Gate) {
        let s = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
        (Arc::clone(&s), s)
    }

    /// What the mixer would see, sorted so the comparison is order-independent.
    fn gated(sink: &Gate) -> Vec<String> {
        let mut v: Vec<String> = sink.lock().expect("gate lock").iter().cloned().collect();
        v.sort();
        v
    }

    fn uuid(n: u128) -> String {
        verio_discovery::Uuid::from_u128(n).to_string()
    }

    /// `(target, muted, by)` for every mute event drained so far.
    fn mute_events(rx: &mpsc::Receiver<RoomEvent>) -> Vec<(String, bool, String)> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerMuted {
                target_user_id,
                muted,
                by,
            } = ev
            {
                out.push((target_user_id, muted, by));
            }
        }
        out
    }

    #[test]
    fn a_mute_targets_the_person_so_it_survives_their_next_launch() {
        // The reason the mute is keyed on `user_id` at all. A wire id is minted per
        // process, so a mute stored against it would stop matching the next time
        // this person launched — which is exactly when a persisted mute is supposed
        // to still be in force.
        let (mut m, _rx) = manager();
        let (write, read) = sink();
        m.set_mute_sink(write);

        m.add_peer(identity("Bo", 2));
        m.set_room_mute("user-Bo-2", true).expect("mute");
        assert_eq!(gated(&read), vec![uuid(2)]);

        // Bo quits and comes back: same person, new session id.
        m.remove_peer(&uuid(2));
        m.add_peer(Identity {
            uuid: verio_discovery::Uuid::from_u128(7),
            user_id: "user-Bo-2".to_string(),
            is_bot: false,
            force_stop: false,
            name: "Bo".to_string(),
            version: String::new(),
        });
        assert_eq!(
            gated(&read),
            vec![uuid(7)],
            "the same person is still muted under their new session id"
        );
    }

    #[test]
    fn a_peer_with_no_persistent_id_cannot_be_muted() {
        // A peer whose `Hello` we never saw has nothing a mute can key on. Muting
        // their wire id instead would appear to work and then quietly stop.
        let (mut m, _rx) = manager();
        assert!(m.set_room_mute("", true).is_err());
        assert!(m.set_room_mute("   ", true).is_err());
        assert!(m.mutes().is_empty());
        assert!(m.db.muted_users().expect("read").is_empty());
    }

    #[test]
    fn the_gate_mirrors_the_mute_list_and_the_roster() {
        let (mut m, _rx) = manager();
        let (write, read) = sink();
        m.set_mute_sink(write);
        m.add_peer(identity("Bo", 2));
        m.add_peer(identity("Cy", 3));

        m.set_room_mute("user-Bo-2", true).expect("mute");
        assert_eq!(gated(&read), vec![uuid(2)]);

        // An unrelated peer leaving must not disturb the gate.
        m.remove_peer(&uuid(3));
        assert_eq!(gated(&read), vec![uuid(2)]);

        // The muted peer leaving clears it from the gate, but the *mute* stays on
        // the list, ready to apply again the moment they return.
        m.remove_peer(&uuid(2));
        assert!(gated(&read).is_empty());
        assert_eq!(
            m.mutes(),
            vec![MuteInfo {
                target_user_id: "user-Bo-2".to_string(),
                by: "self".to_string(),
            }]
        );
    }

    #[test]
    fn a_pipeline_restart_re_applies_the_mutes_it_already_held() {
        // A restart hands the room a brand-new, empty gate. Without the re-derive
        // in `set_mute_sink`, changing your output device would silently lift every
        // mute in the room — with nothing in the UI to say so.
        let (mut m, _rx) = manager();
        m.add_peer(identity("Bo", 2));
        m.set_room_mute("user-Bo-2", true).expect("mute");

        let (write, read) = sink();
        m.set_mute_sink(write);
        assert_eq!(gated(&read), vec![uuid(2)]);
    }

    #[test]
    fn a_mute_outlives_the_process() {
        let db = temp_db("mute-restart");
        let (tx, _rx) = mpsc::channel();
        let mut first = RoomManager::new(identity("self", 999), tx, db.clone());
        first.set_room_mute("user-Bo-2", true).expect("mute");
        drop(first);

        let (tx2, _rx2) = mpsc::channel();
        let second = RoomManager::new(identity("self", 999), tx2, db);
        assert_eq!(
            second.mutes(),
            vec![MuteInfo {
                target_user_id: "user-Bo-2".to_string(),
                // Not persisted: we know they are muted, not who muted them, and an
                // invented name would be worse than none.
                by: String::new(),
            }]
        );
    }

    #[test]
    fn a_mute_change_is_reported_to_the_ui_with_who_made_it() {
        // The target's own client is told who silenced it. Being silenced with no
        // explanation is the bug `by` exists to avoid.
        let (mut m, rx) = manager();
        m.add_peer(identity("Bo", 2));
        m.set_room_mute("user-Bo-2", true).expect("mute");
        m.set_room_mute("user-Bo-2", false).expect("unmute");
        assert_eq!(
            mute_events(&rx),
            vec![
                ("user-Bo-2".to_string(), true, "self".to_string()),
                ("user-Bo-2".to_string(), false, "self".to_string()),
            ]
        );
    }

    #[test]
    fn a_mute_that_arrives_from_the_wire_is_recorded_and_persisted() {
        // `apply_mute(.., broadcast: false)` is the receive path. It has to land in
        // the list, the gate and the store exactly like a local one — otherwise a
        // peer's mute would work until you restarted and then forget itself.
        let (mut m, rx) = manager();
        let (write, read) = sink();
        m.set_mute_sink(write);
        m.add_peer(identity("Bo", 2));

        m.apply_mute("user-Bo-2", true, "Cy", false);

        assert_eq!(gated(&read), vec![uuid(2)]);
        assert_eq!(
            m.db.muted_users().expect("read"),
            vec!["user-Bo-2".to_string()]
        );
        assert_eq!(
            mute_events(&rx),
            vec![("user-Bo-2".to_string(), true, "Cy".to_string())]
        );
    }

    #[test]
    fn a_wire_mute_with_no_target_is_dropped_rather_than_stored() {
        // Reachable from the wire: a peer built before the persistent id existed
        // sends an empty target. Storing it would put a mute in the list that
        // matches nobody, and restore it uselessly on every launch from then on.
        let (mut m, rx) = manager();
        m.apply_mute("  ", true, "Cy", false);
        assert!(m.mutes().is_empty());
        assert!(m.db.muted_users().expect("read").is_empty());
        assert!(mute_events(&rx).is_empty());
    }

    // -- Session 29b: the music bot peer --------------------------------------

    /// The bot is a peer like any other, and the roster must say what it is —
    /// nothing else about it is distinguishable from a person.
    #[test]
    fn a_bot_peer_is_marked_as_a_bot() {
        let (mut m, _rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));
        let peer = m.peers.get(&uuid(42)).expect("bot in the roster");
        assert!(peer.is_bot, "the bot declaration must reach the roster");
        assert_eq!(peer.name, "Music");
    }

    /// Session 29b Force Stop: the host's decision has to reach the roster, because
    /// a listener's Stop control is drawn from it and from nothing else.
    #[test]
    fn a_bot_carries_the_force_stop_decision() {
        let (mut m, _rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));
        assert!(
            m.peers.get(&uuid(42)).expect("bot").force_stop,
            "a bot whose host allows a remote stop has to say so"
        );
    }

    /// And the other direction, which is the one the whole feature exists for: a
    /// host who turned Force Stop off must be visibly distinguishable, or listeners
    /// would be shown a button that does nothing.
    #[test]
    fn a_bot_whose_host_disabled_force_stop_says_so() {
        let (mut m, _rx) = manager();
        m.add_peer(unforceable_bot_identity("Music", 43, "user-Kaveh-1"));
        assert!(
            !m.peers.get(&uuid(43)).expect("bot").force_stop,
            "a bot whose host disabled Force Stop must not appear stoppable"
        );
    }

    /// The safe direction for a peer we have not been told about: no control is
    /// offered, rather than one that is offered and silently ignored.
    #[test]
    fn an_ordinary_peer_never_accepts_a_remote_stop() {
        let (mut m, _rx) = manager();
        m.add_peer(identity("Bo", 2));
        assert!(!m.peers.get(&uuid(2)).expect("peer").force_stop);
    }

    /// The UI builds its tile list from this event, so the flag has to be on it —
    /// the tile has no other way to know whether to draw a Stop control.
    #[test]
    fn the_roster_event_carries_the_force_stop_decision() {
        let (mut m, rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));

        let mut seen = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerConnected {
                is_bot, force_stop, ..
            } = ev
            {
                seen.push((is_bot, force_stop));
            }
        }
        assert_eq!(seen, vec![(true, true)]);
    }

    /// A signalling adopt carries no force-stop decision — the server's
    /// `PeerIdentity` has no such field and serde drops it — so it must not clear
    /// the one we learned from the relay `Hello`. If it did, the Stop control would
    /// vanish from a listener's screen moments after it appeared.
    #[test]
    fn a_signalling_identity_does_not_clear_force_stop() {
        let (mut m, _rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));
        m.adopt_peer(identity("Music", 42));

        assert!(
            m.peers.get(&uuid(42)).expect("bot").force_stop,
            "a signalling adopt must not revoke the host's force-stop decision"
        );
    }

    /// The host is whoever's voice peer shares the bot's `user_id`. That is the
    /// only link between the two sessions, so it has to be on both.
    #[test]
    fn a_bot_and_its_host_share_a_persistent_id() {
        let (mut m, _rx) = manager();
        let host_user_id = "user-Kaveh-1";
        m.add_peer(identity("Kaveh", 1));
        m.add_peer(bot_identity("Music", 42, host_user_id));

        let host = m
            .peers
            .values()
            .find(|p| p.user_id == host_user_id && !p.is_bot)
            .expect("a voice peer with the bot's user id");
        assert_eq!(host.name, "Kaveh");
        let bot = m.peers.get(&uuid(42)).expect("bot");
        assert_eq!(bot.user_id, host.user_id, "the two sessions are one person");
        assert_ne!(bot.uuid, host.uuid, "but not one session");
    }

    /// A `PeerJoined` from the signalling server cannot carry the bot flag — the
    /// server's own identity struct has no such field — so its `false` must not
    /// erase a `true` we already learned from the relay. Without this asymmetry the
    /// bot would flip back into looking like a person whenever signalling won a
    /// race.
    #[test]
    fn a_signalling_identity_does_not_clear_the_bot_flag() {
        let (mut m, _rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));
        m.adopt_peer(identity("Music", 42));

        assert!(
            m.peers.get(&uuid(42)).expect("bot").is_bot,
            "a signalling adopt must not demote a bot to a person"
        );
    }

    /// The UI learns what a peer is from the roster event, so the flag has to be
    /// on it — the tile has no other way to know it should offer a Stop control.
    #[test]
    fn the_roster_event_carries_the_bot_flag() {
        let (mut m, rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));

        let mut flags = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerConnected { name, is_bot, .. } = ev {
                flags.push((name, is_bot));
            }
        }
        assert_eq!(flags, vec![("Music".to_string(), true)]);
    }

    /// A person is not a bot, and the default has to be the safe direction: a peer
    /// that predates the field reads as a person rather than being dressed up as a
    /// music bot and given a Stop button.
    #[test]
    fn an_ordinary_peer_is_not_a_bot() {
        let (mut m, _rx) = manager();
        m.add_peer(identity("Bo", 2));
        assert!(!m.peers.get(&uuid(2)).expect("peer").is_bot);
    }

    /// The flag has to be on the event the UI builds its list from, and it must
    /// survive a rename — a rename is a roster update, not a change of nature.
    #[test]
    fn a_rename_keeps_the_bot_flag() {
        let (mut m, rx) = manager();
        m.add_peer(bot_identity("Music", 42, "user-Kaveh-1"));
        let _ = rx.try_recv();
        m.adopt_peer(bot_identity("Music (Kaveh)", 42, "user-Kaveh-1"));

        let mut last = None;
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerConnected { name, is_bot, .. } = ev {
                last = Some((name, is_bot));
            }
        }
        assert_eq!(last, Some(("Music (Kaveh)".to_string(), true)));
    }

    /// Session 29b: stopping the music must not throw the host out of their own
    /// call.
    ///
    /// A host alone in a room has an empty roster, and the only peer that ever
    /// appears in it is their own bot — so the bot's departure is the first time
    /// that roster empties, and `remove_peer`'s "all peers left" teardown fired.
    /// The host lost their room (and their room code) for having played a track.
    #[test]
    fn a_bot_leaving_does_not_end_the_room() {
        let (mut m, rx) = manager();
        let bot_id = uuid(42);
        m.set_state(RoomState::Connected);
        m.add_peer(bot_identity("Music", 42, "user-self-999"));

        m.remove_peer(&bot_id);

        assert_eq!(
            m.state(),
            RoomState::Connected,
            "the host is still in the room after their own bot leaves"
        );
        // The guard must not swallow the departure itself: the Music tile still
        // has to go, or a stopped bot leaves a card nobody can explain.
        let mut gone = false;
        while let Ok(ev) = rx.try_recv() {
            if let RoomEvent::PeerDisconnected { peer_id, .. } = ev {
                gone = peer_id.as_deref() == Some(bot_id.as_str());
            }
        }
        assert!(gone, "the bot's tile must still be removed");
    }

    /// The counterpart, so the guard above cannot pass by having disabled the
    /// teardown outright: a *person* leaving an otherwise empty roster still ends
    /// the room, exactly as it did before.
    #[test]
    fn the_last_person_leaving_still_ends_the_room() {
        let (mut m, _rx) = manager();
        m.set_state(RoomState::Connected);
        m.add_peer(identity("Ada", 1));

        m.remove_peer(&uuid(1));

        assert_eq!(m.state(), RoomState::Idle);
    }
}


