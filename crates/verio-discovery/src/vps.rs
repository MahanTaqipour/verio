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
const WS_POLL_TIMEOUT: Duration = Duration::from_millis(200);

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
                                },
                                room,
                            ));
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

    /// Session 12: peers that were already in the room when we joined.
    #[must_use]
    pub fn initial_peers(&self) -> Vec<Identity> {
        self.initial_peers.clone()
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
        loop {
            if self.stop.load(Ordering::Relaxed) {
                self.leave_and_close();
                tracing::info!("VPS signaling: left room on request");
                break;
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
                            _ => {}
                        }
                    }
                }
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


pub fn extract_host_and_ports(server_url: &str) -> (String, u16, u16) {
    let s = server_url.trim();
    let s = s.trim_start_matches("ws://").trim_start_matches("wss://");
    let host_port = s.split('/').next().unwrap_or("127.0.0.1:8443");
    let (host, tcp_port) = if let Some((h, p)) = host_port.split_once(':') {
        let port = p.parse::<u16>().unwrap_or(8443);
        (h.to_string(), port)
    } else {
        (host_port.to_string(), 8443)
    };
    let udp_port = tcp_port.wrapping_add(1);
    (host, tcp_port, udp_port)
}

fn normalize_ws_url(raw: &str) -> String {
    let raw = raw.trim();
    let with_scheme = if raw.starts_with("ws://") || raw.starts_with("wss://") {
        raw.to_string()
    } else {
        format!("ws://{raw}")
    };
    if with_scheme.ends_with("/ws") {
        with_scheme
    } else if with_scheme.ends_with('/') {
        format!("{with_scheme}ws")
    } else {
        format!("{with_scheme}/ws")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_ws_url() {
        assert_eq!(normalize_ws_url("127.0.0.1:8443"), "ws://127.0.0.1:8443/ws");
        assert_eq!(normalize_ws_url("ws://127.0.0.1:8443"), "ws://127.0.0.1:8443/ws");
        assert_eq!(normalize_ws_url("ws://127.0.0.1:8443/ws"), "ws://127.0.0.1:8443/ws");
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
}
