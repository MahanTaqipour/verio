//! VPS WebSocket signaling client.
//!
//! Connects to the VPS `/ws` endpoint to create or join 4-digit room codes,
//! exchange identities, and relay WebRTC SDP offers/answers.

use tungstenite::stream::MaybeTlsStream;
use tungstenite::{connect, Message, WebSocket};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use serde::{Deserialize, Serialize};

use crate::{Identity, Role, Signaling, SignalingError};

pub type WsStream = WebSocket<MaybeTlsStream<TcpStream>>;

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

        let (mut socket, _) = connect(&url).map_err(|e| {
            SignalingError::Protocol(format!("failed to connect to VPS {url}: {e}"))
        })?;

        let msg = WireClientMessage::Create {
            identity: identity.clone(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Protocol(format!("serialize create msg: {e}")))?;
        socket
            .send(Message::Text(text))
            .map_err(|e| SignalingError::Protocol(format!("send create msg: {e}")))?;

        // Wait for Created response
        loop {
            let reply = socket
                .read()
                .map_err(|e| SignalingError::Protocol(format!("read create response: {e}")))?;
            if let Message::Text(t) = reply {
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
                                target_peer: None,
                                stop: Arc::new(AtomicBool::new(false)),
                                initial_peers: Vec::new(),
                            },
                            room,
                        ));
                    }
                    WireServerMessage::Error { message } => {
                        return Err(SignalingError::Protocol(format!("server error: {message}")));
                    }
                    _ => {}
                }
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

        let (mut socket, _) = connect(&url).map_err(|e| {
            SignalingError::Protocol(format!("failed to connect to VPS {url}: {e}"))
        })?;

        let msg = WireClientMessage::Join {
            room: room_code.clone(),
            identity: identity.clone(),
        };
        let text = serde_json::to_string(&msg)
            .map_err(|e| SignalingError::Protocol(format!("serialize join msg: {e}")))?;
        socket
            .send(Message::Text(text))
            .map_err(|e| SignalingError::Protocol(format!("send join msg: {e}")))?;

        // Wait for Joined response
        loop {
            let reply = socket
                .read()
                .map_err(|e| SignalingError::Protocol(format!("read join response: {e}")))?;
            if let Message::Text(t) = reply {
                let parsed: WireServerMessage = serde_json::from_str(&t).map_err(|e| {
                    SignalingError::Protocol(format!("parse join response: {e}"))
                })?;
                match parsed {
                    WireServerMessage::Joined { room, peers } => {
                        tracing::info!(room = %room, count = peers.len(), "joined room on VPS");
                        let peers_at_join = peers.clone();
                        let target_peer = peers.into_iter().next();
                        return Ok(Self {
                            socket,
                            room_code: room,
                            target_peer,
                            stop: Arc::new(AtomicBool::new(false)),
                            initial_peers: peers_at_join,
                        });
                    }
                    WireServerMessage::Error { message } => {
                        return Err(SignalingError::Protocol(format!("server error: {message}")));
                    }
                    _ => {}
                }
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

    /// Keep the WebSocket open and report peers leaving. Polls with a short read
    /// timeout so the room can stop it; on stop we send an explicit Leave (so the
    /// server drops us immediately) and close the socket.
    pub fn run_listener(mut self, mut on_event: impl FnMut(SignalingEvent) + Send + 'static) {
        if let MaybeTlsStream::Plain(s) = self.socket.get_ref() {
            let _ = s.set_read_timeout(Some(std::time::Duration::from_millis(200)));
        }
        loop {
            if self.stop.load(Ordering::Relaxed) {
                if let Ok(text) = serde_json::to_string(&WireClientMessage::Leave {
                    room: self.room_code.clone(),
                }) {
                    let _ = self.socket.send(Message::Text(text));
                }
                let _ = self.socket.close(None);
                tracing::info!("VPS signaling: left room on request");
                break;
            }
            match self.socket.read() {
                Ok(Message::Text(t)) => {
                    if let Ok(parsed) = serde_json::from_str::<WireServerMessage>(&t) {
                        match parsed {
                            WireServerMessage::PeerLeft { peer_id, .. } => {
                                tracing::info!(%peer_id, "VPS signaling: peer left room");
                                on_event(SignalingEvent::PeerLeft(peer_id));
                            }
                            WireServerMessage::PeerJoined { peer, .. } => {
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
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut => {}
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
            // As host, block until a peer joins
            loop {
                let msg = self
                    .socket
                    .read()
                    .map_err(|e| SignalingError::Protocol(format!("read peer_joined: {e}")))?;
                if let Message::Text(t) = msg {
                    let parsed: WireServerMessage = serde_json::from_str(&t).map_err(|e| {
                        SignalingError::Protocol(format!("parse peer_joined: {e}"))
                    })?;
                    match parsed {
                        WireServerMessage::PeerJoined { peer, .. } => {
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

        // Block for answer
        loop {
            let reply = self
                .socket
                .read()
                .map_err(|e| SignalingError::Protocol(format!("read answer: {e}")))?;
            if let Message::Text(t) = reply {
                let parsed: WireServerMessage = serde_json::from_str(&t)
                    .map_err(|e| SignalingError::Protocol(format!("parse answer: {e}")))?;
                match parsed {
                    WireServerMessage::Answer { sdp, .. } => {
                        return Ok(sdp);
                    }
                    WireServerMessage::Error { message } => {
                        return Err(SignalingError::Protocol(format!("server error: {message}")));
                    }
                    _ => {}
                }
            }
        }
    }

    fn receive_offer(&mut self) -> Result<String, SignalingError> {
        loop {
            let reply = self
                .socket
                .read()
                .map_err(|e| SignalingError::Protocol(format!("read offer: {e}")))?;
            if let Message::Text(t) = reply {
                let parsed: WireServerMessage = serde_json::from_str(&t)
                    .map_err(|e| SignalingError::Protocol(format!("parse offer: {e}")))?;
                match parsed {
                    WireServerMessage::Offer { sdp, .. } => {
                        return Ok(sdp);
                    }
                    WireServerMessage::Error { message } => {
                        return Err(SignalingError::Protocol(format!("server error: {message}")));
                    }
                    _ => {}
                }
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
}
