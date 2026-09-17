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
    PeerConnected { name: String, uuid: String },
    PeerDisconnected {
        reason: String,
        peer_id: Option<String>,
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
}

/// Session 12: one remote participant. The relay fans out to every peer in the
/// room, so a single session carries audio for all of them and each datagram is
/// attributed to its sender.
#[derive(Debug, Clone, Serialize)]
pub struct PeerInfo {
    pub uuid: String,
    pub name: String,
    pub speaking: bool,
    pub mute: bool,
    pub deafen: bool,
    /// Session 14: this peer is playing music into the room.
    pub music: bool,
    pub music_title: String,
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
}

struct ActiveSession {
    handle: PeerHandle,
}

impl RoomManager {
    pub fn new(identity: Identity, room_events: Sender<RoomEvent>) -> Self {
        let silence_frame = Arc::new(encode_silence_frame());
        tracing::info!(bytes = silence_frame.len(), "cached Opus silence keep-alive frame");
        let (net_out_tx, net_out_rx) = mpsc::channel::<NetAudioOut>();
        let room = Self {
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
        };
        room.spawn_audio_pump();
        room
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
            if self.peers.remove(&uuid).is_some() {
                self.emit(RoomEvent::PeerDisconnected {
                    reason: "peer timed out".into(),
                    peer_id: Some(uuid),
                });
            }
        }
    }

    /// Session 12: register a peer (from signalling or a relay hello) and tell the UI.
    pub fn add_peer(&mut self, peer: Identity) {
        let uuid = peer.uuid.to_string();
        let is_new = !self.peers.contains_key(&uuid);
        self.peer_seen
            .insert(uuid.clone(), std::time::Instant::now());
        let entry = self.peers.entry(uuid.clone()).or_insert_with(|| PeerInfo {
            uuid: uuid.clone(),
            name: peer.name.clone(),
            speaking: false,
            mute: false,
            deafen: false,
            music: false,
            music_title: String::new(),
        });
        entry.name = peer.name.clone();
        if is_new {
            self.emit(RoomEvent::PeerConnected {
                name: peer.name.clone(),
                uuid,
            });
        }
    }

    /// Session 12: drop a peer and tell the UI; the room idles when it empties.
    pub fn remove_peer(&mut self, uuid: &str) {
        if self.peers.remove(uuid).is_some() {
            self.emit(RoomEvent::PeerDisconnected {
                reason: "peer left room".into(),
                peer_id: Some(uuid.to_string()),
            });
        }
        if self.peers.is_empty() {
            self.teardown_session("all peers left");
            self.set_state(RoomState::Idle);
        }
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
            m.emit(RoomEvent::PeerConnected {
                name: remote.name.clone(),
                uuid: remote.uuid.to_string(),
            });
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
            m.emit(RoomEvent::PeerConnected {
                name: remote.name.clone(),
                uuid: remote.uuid.to_string(),
            });
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
        for peer in session.initial_peers() {
            let mut m = room.lock().expect("room lock");
            m.add_peer(peer);
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
                        message: ControlMessage::Hello { name, .. },
                    } => {
                        let mut m = room.lock().expect("room lock");
                        let uuid = resolve_peer(&peer, &peer_id);
                        m.mark_peer_seen(&uuid);
                        let is_new = !m.peers.contains_key(&uuid);
                        let renamed = m
                            .peers
                            .get(&uuid)
                            .is_some_and(|p| p.name != name);
                        if is_new {
                            m.add_peer(Identity {
                                uuid: verio_discovery::Uuid::parse_str(&uuid).unwrap_or_else(|_| {
                                    verio_discovery::Uuid::nil()
                                }),
                                name: name.clone(),
                                version: String::new(),
                            });
                        } else if renamed {
                            if let Some(p) = m.peers.get_mut(&uuid) {
                                p.name = name.clone();
                            }
                            m.emit(RoomEvent::PeerConnected {
                                name,
                                uuid,
                            });
                        }
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
                        m.teardown_session(&reason);
                        m.set_state(RoomState::Idle);
                        m.emit(RoomEvent::PeerDisconnected {
                            reason,
                            peer_id: Some(peer_id.clone()),
                        });
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


