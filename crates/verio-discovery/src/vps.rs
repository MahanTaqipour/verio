//! VPS WebSocket signaling client.
//!
//! Connects to the VPS `/ws` endpoint to create or join 4-digit room codes,
//! exchange identities, and relay WebRTC SDP offers/answers.

use tungstenite::client::{uri_mode, IntoClientRequest};
use tungstenite::stream::{MaybeTlsStream, Mode};
use tungstenite::{client_tls, Message, WebSocket};
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};

use crate::{Identity, Role, Signaling, SignalingError, Uuid};

pub type WsStream = WebSocket<MaybeTlsStream<TcpStream>>;

const WS_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WS_POLL_TIMEOUT: Duration = Duration::from_millis(25);

/// Set read timeout on both plain TCP and `native-tls` WebSocket streams.
pub fn set_ws_read_timeout(socket: &mut WsStream, dur: Option<Duration>) {
    match socket.get_ref() {
        MaybeTlsStream::Plain(s) => {
            let _ = s.set_read_timeout(dur);
        }
        MaybeTlsStream::NativeTls(s) => {
            let _ = s.get_ref().set_read_timeout(dur);
        }
        _ => {}
    }
}

fn connect_ws(server_url: &str) -> Result<WsStream, SignalingError> {
    let url = normalize_ws_url(server_url);
    let request = url
        .as_str()
        .into_client_request()
        .map_err(|e| SignalingError::Protocol(format!("invalid VPS URL {url}: {e}")))?;
    let uri = request.uri();
    let mode = uri_mode(uri)
        .map_err(|e| SignalingError::Protocol(format!("unsupported VPS URL {url}: {e}")))?;
    let host = uri
        .host()
        .ok_or_else(|| SignalingError::Protocol(format!("missing host in VPS URL {url}")))?;
    let host = if host.starts_with('[') && host.ends_with(']') {
        &host[1..host.len() - 1]
    } else {
        host
    };
    let port = uri.port_u16().unwrap_or(match mode {
        Mode::Plain => 80,
        Mode::Tls => 443,
    });
    let addrs: Vec<_> = (host, port)
        .to_socket_addrs()
        .map_err(|e| SignalingError::Protocol(format!("resolve VPS {host}:{port}: {e}")))?
        .collect();
    let mut last_err = None;
    let mut tcp = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, WS_CONNECT_TIMEOUT) {
            Ok(s) => {
                tcp = Some(s);
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let stream = tcp.ok_or_else(|| {
        SignalingError::Protocol(format!(
            "failed to connect to VPS {url}: {}",
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "no resolved addresses".to_string())
        ))
    })?;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(WS_CONNECT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WS_CONNECT_TIMEOUT));

    let (mut socket, _) = client_tls(request, stream).map_err(|e| {
        SignalingError::Protocol(format!("WebSocket handshake failed for {url}: {e}"))
    })?;
    set_ws_read_timeout(&mut socket, Some(WS_POLL_TIMEOUT));
    Ok(socket)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireClientMessage {
    Create {
        identity: Identity,
    },
    Join {
        room: String,
        identity: Identity,
    },
    Offer {
        room: String,
        target: String,
        sdp: String,
    },
    Answer {
        room: String,
        target: String,
        sdp: String,
    },
    Leave {
        room: String,
    },
    ScreenState {
        room: String,
        sharing: bool,
        #[serde(default)]
        label: String,
    },
    ScreenSignal {
        room: String,
        target: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServerConfig {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum WireServerMessage {
    Created {
        room: String,
        peer_id: String,
    },
    Joined {
        room: String,
        peers: Vec<Identity>,
    },
    PeerJoined {
        room: String,
        peer: Identity,
    },
    PeerLeft {
        room: String,
        peer_id: String,
    },
    Offer {
        room: String,
        from: String,
        sdp: String,
    },
    Answer {
        room: String,
        from: String,
        sdp: String,
    },
    Candidate {
        room: String,
        from: String,
        candidate: String,
    },
    IceConfig {
        servers: Vec<IceServerConfig>,
    },
    PeerScreenState {
        room: String,
        peer_id: String,
        sharing: bool,
        #[serde(default)]
        label: String,
    },
    ScreenSignal {
        room: String,
        from: String,
        payload: serde_json::Value,
    },
    Error {
        message: String,
    },
}

/// Session 12: what the listener reports while the room is live.
#[derive(Debug, Clone)]
pub enum SignalingEvent {
    /// A peer entered the room (relayed by the server).
    PeerJoined(Identity),
    /// A peer left the room (its UUID).
    PeerLeft(String),
    /// Session 37A: STUN/TURN ICE server configuration pushed by VPS signaling server.
    IceConfig(Vec<IceServerConfig>),
    /// Session 37A: a remote peer started or stopped screen sharing.
    PeerScreenState {
        peer_id: String,
        sharing: bool,
        label: String,
    },
    /// Session 37A: targeted 1-to-1 WebRTC screen-share SDP/ICE signaling.
    ScreenSignal {
        from: String,
        payload: serde_json::Value,
    },
}

/// Session 37A: outbound control commands forwarded to the live VPS WebSocket listener.
#[derive(Debug, Clone)]
pub enum SignalingCommand {
    ScreenState {
        sharing: bool,
        label: String,
    },
    ScreenSignal {
        target: String,
        payload: serde_json::Value,
    },
}

pub struct VpsSignaling {
    socket: WsStream,
    room_code: String,
    local_uuid: Uuid,
    target_peer: Option<Identity>,
    /// Session 11: set by the room to leave the room and close the socket.
    stop: Arc<AtomicBool>,
    /// Session 12: peers already in the room when we joined (empty for the creator).
    initial_peers: Vec<Identity>,
    /// Session 37A: outbound signaling command receiver drained in `run_listener`.
    cmd_rx: Option<std::sync::mpsc::Receiver<SignalingCommand>>,
    /// Session 37A: signaling events received before `run_listener` starts.
    pending_events: Vec<SignalingEvent>,
}

impl VpsSignaling {
    /// Connect to VPS and create a new room. Returns the signaling session and 4-digit code.
    pub fn create(server_url: &str, identity: &Identity) -> Result<(Self, String), SignalingError> {
        let url = normalize_ws_url(server_url);
        tracing::info!(%url, "connecting to VPS signaling server to create room");

        let mut socket = connect_ws(&url)?;

        let msg = WireClientMessage::Create {
            identity: identity.clone(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Protocol(format!("serialize create msg: {e}")))?;
        socket
            .send(Message::Text(text))
            .map_err(|e| SignalingError::Protocol(format!("send create msg: {e}")))?;

        // Wait for Created response within a bounded timeout.
        let deadline = Instant::now() + WS_CONNECT_TIMEOUT;
        loop {
            let reply = match socket.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    if Instant::now() >= deadline {
                        return Err(SignalingError::Protocol(
                            "timed out waiting for room creation response".into(),
                        ));
                    }
                    continue;
                }
                Err(e) => {
                    return Err(SignalingError::Protocol(format!(
                        "read create response: {e}"
                    )));
                }
            };
            match reply {
                Message::Text(t) => {
                    let parsed: WireServerMessage = serde_json::from_str(&t).map_err(|e| {
                        SignalingError::Protocol(format!("parse create response: {e}"))
                    })?;
                    match parsed {
                        WireServerMessage::Created { room, .. } => {
                            tracing::info!(room = %room, "room created on VPS");
                            return Ok((
                                Self {
                                    socket,
                                    room_code: room.clone(),
                                    local_uuid: identity.uuid,
                                    target_peer: None,
                                    stop: Arc::new(AtomicBool::new(false)),
                                    initial_peers: Vec::new(),
                                    cmd_rx: None,
                                    pending_events: Vec::new(),
                                },
                                room,
                            ));
                        }
                        WireServerMessage::IceConfig { servers } => {
                            tracing::debug!(count = servers.len(), "buffered early IceConfig during create");
                            // Unlikely before Created, but preserve if sent out-of-order.
                        }
                        WireServerMessage::Error { message } => {
                            return Err(SignalingError::Protocol(format!(
                                "server error: {message}"
                            )));
                        }
                        _ => {}
                    }
                }
                Message::Close(_) => {
                    return Err(SignalingError::Protocol(
                        "server closed connection before room was created".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    /// Connect to VPS and join an existing 4-digit room.
    pub fn join(
        server_url: &str,
        room_code: &str,
        identity: &Identity,
    ) -> Result<Self, SignalingError> {
        let url = normalize_ws_url(server_url);
        let room_code = room_code.trim().to_string();
        tracing::info!(%url, room = %room_code, "connecting to VPS signaling server to join room");

        let mut socket = connect_ws(&url)?;

        let msg = WireClientMessage::Join {
            room: room_code.clone(),
            identity: identity.clone(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Protocol(format!("serialize join msg: {e}")))?;
        socket
            .send(Message::Text(text))
            .map_err(|e| SignalingError::Protocol(format!("send join msg: {e}")))?;

        // Wait for Joined response within a bounded timeout.
        let deadline = Instant::now() + WS_CONNECT_TIMEOUT;
        loop {
            let reply = match socket.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    if Instant::now() >= deadline {
                        return Err(SignalingError::Protocol(
                            "timed out waiting for room join response".into(),
                        ));
                    }
                    continue;
                }
                Err(e) => {
                    return Err(SignalingError::Protocol(format!(
                        "read join response: {e}"
                    )));
                }
            };
            match reply {
                Message::Text(t) => {
                    let parsed: WireServerMessage = serde_json::from_str(&t).map_err(|e| {
                        SignalingError::Protocol(format!("parse join response: {e}"))
                    })?;
                    match parsed {
                        WireServerMessage::Joined { room, peers } => {
                            let filtered: Vec<Identity> = peers
                                .into_iter()
                                .filter(|p| p.uuid != identity.uuid)
                                .collect();
                            tracing::info!(room = %room, count = filtered.len(), "joined room on VPS");
                            let peers_at_join = filtered.clone();
                            let target_peer = filtered.into_iter().next();
                            return Ok(Self {
                                socket,
                                room_code: room,
                                local_uuid: identity.uuid,
                                target_peer,
                                stop: Arc::new(AtomicBool::new(false)),
                                initial_peers: peers_at_join,
                                cmd_rx: None,
                                pending_events: Vec::new(),
                            });
                        }
                        WireServerMessage::Error { message } => {
                            return Err(SignalingError::Protocol(format!(
                                "server error: {message}"
                            )));
                        }
                        _ => {}
                    }
                }
                Message::Close(_) => {
                    return Err(SignalingError::Protocol(
                        "server closed connection before join completed".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    pub fn room_code(&self) -> &str {
        &self.room_code
    }

    /// Session 11: hand the listener a flag the room can set to leave cleanly.
    pub fn set_stop_flag(&mut self, flag: Arc<AtomicBool>) {
        self.stop = flag;
    }

    /// Session 37A: attach an outbound signaling command receiver drained by `run_listener`.
    pub fn set_command_rx(&mut self, rx: std::sync::mpsc::Receiver<SignalingCommand>) {
        self.cmd_rx = Some(rx);
    }

    /// Session 12: peers that were already in the room when we joined.
    #[must_use]
    pub fn initial_peers(&self) -> Vec<Identity> {
        self.initial_peers.clone()
    }

    /// Session 37A: serialize and flush an outbound screen-share command over the WebSocket.
    fn send_command(&mut self, cmd: SignalingCommand) {
        let msg = match cmd {
            SignalingCommand::ScreenState { sharing, label } => WireClientMessage::ScreenState {
                room: self.room_code.clone(),
                sharing,
                label,
            },
            SignalingCommand::ScreenSignal { target, payload } => WireClientMessage::ScreenSignal {
                room: self.room_code.clone(),
                target,
                payload,
            },
        };
        if let Ok(text) = serde_json::to_string(&msg) {
            if let Err(e) = self.socket.send(Message::Text(text)) {
                tracing::warn!(%e, "VPS signaling: send command failed");
                return;
            }
            let _ = self.socket.flush();
        }
    }

    /// Explicitly send a `Leave` message and close the signaling WebSocket.
    pub fn leave_and_close(mut self) {
        if let Ok(text) = serde_json::to_string(&WireClientMessage::Leave {
            room: self.room_code.clone(),
        }) {
            let _ = self.socket.send(Message::Text(text));
        }
        let _ = self.socket.close(None);
    }

    /// Keep the WebSocket open and report peers leaving. Polls with a short read
    /// timeout so the room can stop it; on stop we send an explicit Leave (so the
    /// server drops us immediately) and close the socket.
    pub fn run_listener(mut self, mut on_event: impl FnMut(SignalingEvent) + Send + 'static) {
        set_ws_read_timeout(&mut self.socket, Some(WS_POLL_TIMEOUT));
        let local_uuid_str = self.local_uuid.to_string();
        let cmd_rx = self.cmd_rx.take();
        for ev in std::mem::take(&mut self.pending_events) {
            on_event(ev);
        }
        loop {
            if self.stop.load(Ordering::Relaxed) {
                self.leave_and_close();
                tracing::info!("VPS signaling: left room on request");
                break;
            }
            if let Some(rx) = cmd_rx.as_ref() {
                while let Ok(cmd) = rx.try_recv() {
                    self.send_command(cmd);
                }
            }
            match self.socket.read() {
                Ok(Message::Text(t)) => {
                    if let Ok(parsed) = serde_json::from_str::<WireServerMessage>(&t) {
                        match parsed {
                            WireServerMessage::PeerLeft { peer_id, .. } => {
                                if peer_id == local_uuid_str {
                                    continue;
                                }
                                tracing::info!(%peer_id, "VPS signaling: peer left room");
                                on_event(SignalingEvent::PeerLeft(peer_id));
                            }
                            WireServerMessage::PeerJoined { peer, .. } => {
                                if peer.uuid == self.local_uuid {
                                    tracing::debug!(uuid = %peer.uuid, "VPS signaling: ignoring self PeerJoined");
                                    continue;
                                }
                                tracing::info!(peer = %peer.name, uuid = %peer.uuid, "VPS signaling: peer joined room");
                                on_event(SignalingEvent::PeerJoined(peer));
                            }
                            WireServerMessage::IceConfig { servers } => {
                                tracing::debug!(count = servers.len(), "VPS signaling: received IceConfig");
                                on_event(SignalingEvent::IceConfig(servers));
                            }
                            WireServerMessage::PeerScreenState {
                                peer_id,
                                sharing,
                                label,
                                ..
                            } => {
                                if peer_id == local_uuid_str {
                                    continue;
                                }
                                tracing::info!(%peer_id, sharing, "VPS signaling: peer screen state updated");
                                on_event(SignalingEvent::PeerScreenState {
                                    peer_id,
                                    sharing,
                                    label,
                                });
                            }
                            WireServerMessage::ScreenSignal { from, payload, .. } => {
                                if from == local_uuid_str {
                                    continue;
                                }
                                on_event(SignalingEvent::ScreenSignal { from, payload });
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Message::Ping(payload)) => {
                    let _ = self.socket.send(Message::Pong(payload));
                    let _ = self.socket.flush();
                }
                Ok(Message::Pong(_)) => {}
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
                Err(tungstenite::Error::ConnectionClosed)
                | Err(tungstenite::Error::AlreadyClosed) => break,
                Err(e) => {
                    tracing::warn!(%e, "VPS signaling: listener stopped");
                    break;
                }
            }
        }
    }
}

impl Signaling for VpsSignaling {
    fn exchange_identities(
        &mut self,
        identity: &Identity,
    ) -> Result<(Identity, Role), SignalingError> {
        let remote = if let Some(peer) = self.target_peer.take() {
            peer
        } else {
            // Wait until another peer joins, checking stop every WS_POLL_TIMEOUT.
            loop {
                if self.stop.load(Ordering::Relaxed) {
                    return Err(SignalingError::Protocol("signaling stopped".into()));
                }
                let msg = match self.socket.read() {
                    Ok(m) => m,
                    Err(tungstenite::Error::Io(e))
                        if e.kind() == ErrorKind::WouldBlock
                            || e.kind() == ErrorKind::TimedOut =>
                    {
                        continue;
                    }
                    Err(e) => {
                        return Err(SignalingError::Protocol(format!("read peer_joined: {e}")));
                    }
                };
                match msg {
                    Message::Text(t) => {
                        let parsed: WireServerMessage = serde_json::from_str(&t).map_err(|e| {
                            SignalingError::Protocol(format!("parse peer_joined: {e}"))
                        })?;
                        match parsed {
                            WireServerMessage::PeerJoined { peer, .. } => {
                                if peer.uuid == identity.uuid {
                                    continue;
                                }
                                break peer;
                            }
                            WireServerMessage::Error { message } => {
                                return Err(SignalingError::Protocol(format!(
                                    "server error: {message}"
                                )));
                            }
                            _ => {}
                        }
                    }
                    Message::Close(_) => {
                        return Err(SignalingError::Protocol(
                            "server closed connection while waiting for peer".into(),
                        ));
                    }
                    _ => {}
                }
            }
        };

        // Compare UUIDs: larger is offerer
        let role = if identity.uuid > remote.uuid {
            Role::Offerer
        } else {
            Role::Answerer
        };

        self.target_peer = Some(remote.clone());
        Ok((remote, role))
    }

    fn exchange_offer(&mut self, offer: &str) -> Result<String, SignalingError> {
        let target = self
            .target_peer
            .as_ref()
            .ok_or_else(|| SignalingError::Protocol("missing target peer".into()))?
            .uuid
            .to_string();

        let msg = WireClientMessage::Offer {
            room: self.room_code.clone(),
            target,
            sdp: offer.to_string(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Protocol(format!("serialize offer: {e}")))?;
        self.socket
            .send(Message::Text(text))
            .map_err(|e| SignalingError::Protocol(format!("send offer: {e}")))?;

        // Block for answer, respecting stop flag and WS_POLL_TIMEOUT wakeups.
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(SignalingError::Protocol("signaling stopped".into()));
            }
            let reply = match self.socket.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    return Err(SignalingError::Protocol(format!("read answer: {e}")));
                }
            };
            match reply {
                Message::Text(t) => {
                    let parsed: WireServerMessage = serde_json::from_str(&t)
                        .map_err(|e| SignalingError::Protocol(format!("parse answer: {e}")))?;
                    match parsed {
                        WireServerMessage::Answer { sdp, .. } => {
                            return Ok(sdp);
                        }
                        WireServerMessage::Error { message } => {
                            return Err(SignalingError::Protocol(format!(
                                "server error: {message}"
                            )));
                        }
                        _ => {}
                    }
                }
                Message::Close(_) => {
                    return Err(SignalingError::Protocol(
                        "server closed connection while waiting for answer".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    fn receive_offer(&mut self) -> Result<String, SignalingError> {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return Err(SignalingError::Protocol("signaling stopped".into()));
            }
            let reply = match self.socket.read() {
                Ok(m) => m,
                Err(tungstenite::Error::Io(e))
                    if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    return Err(SignalingError::Protocol(format!("read offer: {e}")));
                }
            };
            match reply {
                Message::Text(t) => {
                    let parsed: WireServerMessage = serde_json::from_str(&t)
                        .map_err(|e| SignalingError::Protocol(format!("parse offer: {e}")))?;
                    match parsed {
                        WireServerMessage::Offer { sdp, .. } => {
                            return Ok(sdp);
                        }
                        WireServerMessage::Error { message } => {
                            return Err(SignalingError::Protocol(format!(
                                "server error: {message}"
                            )));
                        }
                        _ => {}
                    }
                }
                Message::Close(_) => {
                    return Err(SignalingError::Protocol(
                        "server closed connection while waiting for offer".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    fn send_answer(&mut self, answer: &str) -> Result<(), SignalingError> {
        let target = self
            .target_peer
            .as_ref()
            .ok_or_else(|| SignalingError::Protocol("missing target peer".into()))?
            .uuid
            .to_string();

        let msg = WireClientMessage::Answer {
            room: self.room_code.clone(),
            target,
            sdp: answer.to_string(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Protocol(format!("serialize answer: {e}")))?;
        self.socket
            .send(Message::Text(text))
            .map_err(|e| SignalingError::Protocol(format!("send answer: {e}")))?;
        Ok(())
    }
}


fn split_host_and_port(authority: &str, default_port: u16) -> (String, u16) {
    if let Some(rest) = authority.strip_prefix('[') {
        if let Some((host, after_bracket)) = rest.split_once(']') {
            let port = after_bracket
                .strip_prefix(':')
                .and_then(|p| p.parse::<u16>().ok())
                .unwrap_or(default_port);
            return (format!("[{host}]"), port);
        }
    }
    if let Some((h, p)) = authority.rsplit_once(':') {
        if let Ok(port) = p.parse::<u16>() {
            return (h.to_string(), port);
        }
    }
    (authority.to_string(), default_port)
}

pub fn extract_host_and_ports(server_url: &str) -> (String, u16, u16) {
    let normalized = normalize_ws_url(server_url);
    let (is_wss, rest) = if let Some(r) = normalized.strip_prefix("wss://") {
        (true, r)
    } else if let Some(r) = normalized.strip_prefix("ws://") {
        (false, r)
    } else {
        (false, normalized.as_str())
    };
    let host_port = rest.split('/').next().unwrap_or("verio.drfake.ir:9091");
    let (host, tcp_port) = split_host_and_port(host_port, if is_wss { 443 } else { 9091 });
    let udp_port = tcp_port.wrapping_add(1);
    (host, tcp_port, udp_port)
}

pub fn normalize_ws_url(raw: &str) -> String {
    let raw = raw.trim();
    let (is_wss, rest) = if let Some(r) = raw.strip_prefix("wss://") {
        (true, r)
    } else if let Some(r) = raw.strip_prefix("ws://") {
        (false, r)
    } else {
        (false, raw)
    };
    let scheme = if is_wss { "wss://" } else { "ws://" };

    let (authority, tail) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, ""),
    };

    let has_explicit_port = if let Some(after_bracket) = authority.strip_prefix('[') {
        after_bracket
            .split_once(']')
            .is_some_and(|(_, suffix)| suffix.starts_with(':') && suffix.len() > 1)
    } else if let Some((_, p)) = authority.rsplit_once(':') {
        !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())
    } else {
        false
    };

    let normalized_authority = if !is_wss && !has_explicit_port && !authority.is_empty() {
        format!("{authority}:9091")
    } else {
        authority.to_string()
    };

    let normalized_tail = if tail.ends_with("/ws") {
        tail.to_string()
    } else if tail.ends_with('/') {
        format!("{tail}ws")
    } else {
        format!("{tail}/ws")
    };

    format!("{scheme}{normalized_authority}{normalized_tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_ws_url() {
        assert_eq!(normalize_ws_url("verio.drfake.ir"), "ws://verio.drfake.ir:9091/ws");
        assert_eq!(normalize_ws_url("ws://verio.drfake.ir"), "ws://verio.drfake.ir:9091/ws");
        assert_eq!(normalize_ws_url("ws://verio.drfake.ir:9091"), "ws://verio.drfake.ir:9091/ws");
        assert_eq!(normalize_ws_url("127.0.0.1:8443"), "ws://127.0.0.1:8443/ws");
        assert_eq!(normalize_ws_url("ws://127.0.0.1:8443"), "ws://127.0.0.1:8443/ws");
        assert_eq!(normalize_ws_url("ws://127.0.0.1:8443/ws"), "ws://127.0.0.1:8443/ws");
        assert_eq!(normalize_ws_url("wss://verio.drfake.ir"), "wss://verio.drfake.ir/ws");
        assert_eq!(normalize_ws_url("wss://my-vps.com:8443/"), "wss://my-vps.com:8443/ws");
        assert_eq!(normalize_ws_url("wss://my-vps.com:8443/ws"), "wss://my-vps.com:8443/ws");
    }

    #[test]
    fn test_extract_host_and_ports() {
        let (host, tcp, udp) = extract_host_and_ports("ws://141.11.1.110:9091/ws");
        assert_eq!(host, "141.11.1.110");
        assert_eq!(tcp, 9091);
        assert_eq!(udp, 9092);

        let (host2, tcp2, udp2) = extract_host_and_ports("myvps.com:8443");
        assert_eq!(host2, "myvps.com");
        assert_eq!(tcp2, 8443);
        assert_eq!(udp2, 8444);

        let (host3, tcp3, udp3) = extract_host_and_ports("verio.drfake.ir");
        assert_eq!(host3, "verio.drfake.ir");
        assert_eq!(tcp3, 9091);
        assert_eq!(udp3, 9092);
    }

    #[test]
    fn test_vps_signaling_join_filters_self_and_stops_within_poll_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let local_id = Identity {
            uuid: Uuid::from_u128(100),
            user_id: "user-a".into(),
            is_bot: false,
            force_stop: false,
            name: "Alice".into(),
            version: "0.1.0".into(),
        };
        let remote_id = Identity {
            uuid: Uuid::from_u128(200),
            user_id: "user-b".into(),
            is_bot: false,
            force_stop: false,
            name: "Bob".into(),
            version: "0.1.0".into(),
        };
        let local_clone = local_id.clone();
        let remote_clone = remote_id.clone();

        let (saw_leave_tx, saw_leave_rx) = std::sync::mpsc::channel::<String>();
        let server_thread = std::thread::spawn(move || {
            for conn_idx in 0..2 {
                let (stream, _) = listener.accept().unwrap();
                let mut ws = tungstenite::accept(stream).unwrap();
                let Message::Text(req_txt) = ws.read().unwrap() else {
                    panic!("expected join message");
                };
                let WireClientMessage::Join { room, .. } =
                    serde_json::from_str(&req_txt).unwrap()
                else {
                    panic!("expected Join variant");
                };
                // First session simulates a ghost self entry in Joined; second session is clean empty 1111.
                let peers = if conn_idx == 0 {
                    vec![local_clone.clone(), remote_clone.clone()]
                } else {
                    Vec::new()
                };
                let joined = WireServerMessage::Joined { room, peers };
                ws.send(Message::Text(serde_json::to_string(&joined).unwrap()))
                    .unwrap();

                if conn_idx == 0 {
                    // Send a duplicate self PeerJoined followed by an explicit stop -> expect Leave.
                    let self_join = WireServerMessage::PeerJoined {
                        room: "1111".into(),
                        peer: local_clone.clone(),
                    };
                    ws.send(Message::Text(serde_json::to_string(&self_join).unwrap()))
                        .unwrap();
                    loop {
                        match ws.read() {
                            Ok(Message::Text(t)) => {
                                if let Ok(WireClientMessage::Leave { room }) =
                                    serde_json::from_str(&t)
                                {
                                    let _ = saw_leave_tx.send(room);
                                    break;
                                }
                            }
                            Ok(Message::Close(_)) | Err(_) => break,
                            Ok(_) => {}
                        }
                    }
                }
            }
        });

        let url = format!("ws://{addr}/ws");
        let mut session = VpsSignaling::join(&url, "1111", &local_id).unwrap();
        let initial = session.initial_peers();
        assert_eq!(initial.len(), 1, "self UUID must be filtered from initial_peers");
        assert_eq!(initial[0].uuid, remote_id.uuid);

        let stop = Arc::new(AtomicBool::new(false));
        session.set_stop_flag(Arc::clone(&stop));
        let (ev_tx, ev_rx) = std::sync::mpsc::channel();
        let listener_thread = std::thread::spawn(move || {
            session.run_listener(move |ev| {
                let _ = ev_tx.send(ev);
            });
        });

        // Give run_listener time to read self PeerJoined (it must NOT emit PeerJoined for self).
        std::thread::sleep(Duration::from_millis(80));
        assert!(ev_rx.try_recv().is_err(), "self PeerJoined must be ignored");

        let stop_start = Instant::now();
        stop.store(true, Ordering::Relaxed);
        listener_thread.join().unwrap();
        assert!(
            stop_start.elapsed() < Duration::from_secs(2),
            "run_listener must unblock within WS_POLL_TIMEOUT and exit cleanly"
        );
        assert_eq!(saw_leave_rx.recv_timeout(Duration::from_secs(2)).unwrap(), "1111");

        // Immediately rejoin empty 1111 with zero blocking.
        let session2 = VpsSignaling::join(&url, "1111", &local_id).unwrap();
        assert!(session2.initial_peers().is_empty());
        session2.leave_and_close();

        server_thread.join().unwrap();
    }

    #[test]
    fn test_vps_connect_ws_fails_fast_when_port_closed() {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = probe.local_addr().unwrap();
        drop(probe);

        let id = Identity {
            uuid: Uuid::from_u128(77),
            user_id: "u".into(),
            is_bot: false,
            force_stop: false,
            name: "Tester".into(),
            version: "0.1.0".into(),
        };
        let start = Instant::now();
        let res = VpsSignaling::join(&format!("ws://{addr}/ws"), "1111", &id);
        assert!(res.is_err());
        assert!(start.elapsed() < Duration::from_secs(6));
    }

    #[test]
    fn test_vps_run_listener_flushes_pong_on_ping() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let local_id = Identity {
            uuid: Uuid::from_u128(101),
            user_id: "user-pong".into(),
            is_bot: false,
            force_stop: false,
            name: "Alice".into(),
            version: "0.1.0".into(),
        };

        let (saw_pong_tx, saw_pong_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let server_thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let Message::Text(req_txt) = ws.read().unwrap() else {
                panic!("expected join message");
            };
            let WireClientMessage::Join { room, .. } =
                serde_json::from_str(&req_txt).unwrap()
            else {
                panic!("expected Join variant");
            };
            let joined = WireServerMessage::Joined {
                room,
                peers: Vec::new(),
            };
            ws.send(Message::Text(serde_json::to_string(&joined).unwrap()))
                .unwrap();

            // Send Ping with payload
            let ping_payload = vec![0xDE, 0xAD, 0xBE, 0xEF];
            ws.send(Message::Ping(ping_payload.clone())).unwrap();

            // Wait to receive Pong back
            loop {
                match ws.read() {
                    Ok(Message::Pong(payload)) => {
                        let _ = saw_pong_tx.send(payload);
                        break;
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        let url = format!("ws://{addr}/ws");
        let mut session = VpsSignaling::join(&url, "1111", &local_id).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        session.set_stop_flag(Arc::clone(&stop));

        let listener_thread = std::thread::spawn(move || {
            session.run_listener(|_| {});
        });

        let pong_payload = saw_pong_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("server must receive flushed Pong from client");
        assert_eq!(pong_payload, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        stop.store(true, Ordering::Relaxed);
        listener_thread.join().unwrap();
        server_thread.join().unwrap();
    }

    #[test]
    fn test_vps_run_listener_bidirectional_screen_commands_and_events() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let local_id = Identity {
            uuid: Uuid::from_u128(202),
            user_id: "user-screen".into(),
            is_bot: false,
            force_stop: false,
            name: "Bob".into(),
            version: "0.2.0".into(),
        };

        let (server_rx_tx, server_rx_rx) = std::sync::mpsc::channel::<WireClientMessage>();
        let server_thread = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let Message::Text(req_txt) = ws.read().unwrap() else {
                panic!("expected join message");
            };
            let WireClientMessage::Join { room, .. } =
                serde_json::from_str(&req_txt).unwrap()
            else {
                panic!("expected Join variant");
            };
            ws.send(Message::Text(
                serde_json::to_string(&WireServerMessage::Joined {
                    room: room.clone(),
                    peers: Vec::new(),
                })
                .unwrap(),
            ))
            .unwrap();

            // Push IceConfig, PeerScreenState catch-up, and ScreenSignal immediately after Joined.
            let ice = WireServerMessage::IceConfig {
                servers: vec![IceServerConfig {
                    urls: vec!["turn:193.24.105.202:3479?transport=udp".into()],
                    username: Some("1700043200:u".into()),
                    credential: Some("cred".into()),
                }],
            };
            ws.send(Message::Text(serde_json::to_string(&ice).unwrap()))
                .unwrap();

            let peer_state = WireServerMessage::PeerScreenState {
                room: room.clone(),
                peer_id: "sharer-uuid".into(),
                sharing: true,
                label: "Primary Display".into(),
            };
            ws.send(Message::Text(serde_json::to_string(&peer_state).unwrap()))
                .unwrap();

            let inbound_sig = WireServerMessage::ScreenSignal {
                room,
                from: "sharer-uuid".into(),
                payload: serde_json::json!({ "kind": "offer", "sdp": "v=0" }),
            };
            ws.send(Message::Text(serde_json::to_string(&inbound_sig).unwrap()))
                .unwrap();

            loop {
                match ws.read() {
                    Ok(Message::Text(txt)) => {
                        if let Ok(parsed) = serde_json::from_str::<WireClientMessage>(&txt) {
                            let is_leave = matches!(parsed, WireClientMessage::Leave { .. });
                            let _ = server_rx_tx.send(parsed);
                            if is_leave {
                                break;
                            }
                        }
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        let url = format!("ws://{addr}/ws");
        let mut session = VpsSignaling::join(&url, "1111", &local_id).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<SignalingCommand>();
        session.set_stop_flag(Arc::clone(&stop));
        session.set_command_rx(cmd_rx);

        let (ev_tx, ev_rx) = std::sync::mpsc::channel::<SignalingEvent>();
        let listener_thread = std::thread::spawn(move || {
            session.run_listener(move |ev| {
                let _ = ev_tx.send(ev);
            });
        });

        // Verify inbound events arrived.
        let mut got_ice = false;
        let mut got_state = false;
        let mut got_sig = false;
        for _ in 0..3 {
            match ev_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("expected inbound SignalingEvent")
            {
                SignalingEvent::IceConfig(servers) => {
                    assert_eq!(servers.len(), 1);
                    assert_eq!(servers[0].username.as_deref(), Some("1700043200:u"));
                    got_ice = true;
                }
                SignalingEvent::PeerScreenState {
                    peer_id,
                    sharing,
                    label,
                } => {
                    assert_eq!(peer_id, "sharer-uuid");
                    assert!(sharing);
                    assert_eq!(label, "Primary Display");
                    got_state = true;
                }
                SignalingEvent::ScreenSignal { from, payload } => {
                    assert_eq!(from, "sharer-uuid");
                    assert_eq!(payload["kind"], "offer");
                    got_sig = true;
                }
                _ => {}
            }
        }
        assert!(got_ice && got_state && got_sig);

        // Send outbound ScreenState + ScreenSignal and ensure fast delivery while WS read is idle.
        let start = Instant::now();
        cmd_tx
            .send(SignalingCommand::ScreenState {
                sharing: true,
                label: "Screen 2".into(),
            })
            .unwrap();
        cmd_tx
            .send(SignalingCommand::ScreenSignal {
                target: "sharer-uuid".into(),
                payload: serde_json::json!({ "kind": "answer", "sdp": "v=0" }),
            })
            .unwrap();

        match server_rx_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server must receive ScreenState")
        {
            WireClientMessage::ScreenState { sharing, label, .. } => {
                assert!(sharing);
                assert_eq!(label, "Screen 2");
            }
            other => panic!("expected ScreenState, got {other:?}"),
        }
        match server_rx_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("server must receive ScreenSignal")
        {
            WireClientMessage::ScreenSignal {
                target, payload, ..
            } => {
                assert_eq!(target, "sharer-uuid");
                assert_eq!(payload["kind"], "answer");
            }
            other => panic!("expected ScreenSignal, got {other:?}"),
        }
        assert!(start.elapsed() < Duration::from_secs(1));

        stop.store(true, Ordering::Relaxed);
        listener_thread.join().unwrap();
        server_thread.join().unwrap();
    }
}
