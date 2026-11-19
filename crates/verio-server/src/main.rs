//! Verio VPS signaling server and fallback UDP relay.
//!
//! - WebSocket endpoint `/ws`:
//!   - `{"type": "create"}` -> generates random 4-digit code (e.g. `4921`), returns `{"type": "created", "room": "4921"}`.
//!   - `{"type": "join", "room": "4921"}` -> connects peer to room.
//!   - Relays WebRTC SDP offers, answers, and candidates between peers in the same room.
//! - Fallback UDP relay (port 8444):
//!   - Relays voice packets between peers in the same room when direct WebRTC hole-punching fails.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Notify, RwLock};

const DEFAULT_TCP_PORT: u16 = 8443;
const DEFAULT_UDP_PORT: u16 = 8444;
pub const MAIN_ROOT_ROOM: &str = "1111";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub uuid: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub force_stop: bool,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Create {
        identity: PeerIdentity,
    },
    Join {
        room: String,
        identity: PeerIdentity,
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
    Candidate {
        room: String,
        target: String,
        candidate: String,
    },
    Leave {
        room: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Created {
        room: String,
        peer_id: String,
    },
    Joined {
        room: String,
        peers: Vec<PeerIdentity>,
    },
    PeerJoined {
        room: String,
        peer: PeerIdentity,
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

struct PeerSession {
    conn_id: u64,
    identity: PeerIdentity,
    tx: mpsc::UnboundedSender<Message>,
    close_notify: Arc<Notify>,
}

struct Room {
    peers: HashMap<String, PeerSession>,
}

#[derive(Clone)]
struct AppState {
    rooms: Arc<RwLock<HashMap<String, Room>>>,
    next_conn_id: Arc<AtomicU64>,
}

impl AppState {
    fn new() -> Self {
        Self {
            rooms: Arc::new(RwLock::new(HashMap::new())),
            next_conn_id: Arc::new(AtomicU64::new(1)),
        }
    }

    fn allocate_conn_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerConfig {
    pub tcp_port: u16,
    pub udp_port: u16,
    pub main_root: bool,
}

pub fn parse_args_from<I, T>(args: I) -> Result<Option<ServerConfig>, String>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let mut iter = args.into_iter();
    let mut tcp_port = None;
    let mut udp_port = None;
    let mut main_root = false;

    while let Some(arg) = iter.next() {
        let s = arg.as_ref();
        match s {
            "-h" | "--help" => {
                println!(
                    "Verio Signaling Server & Fallback UDP Relay\n\n\
                     Usage: verio-server [OPTIONS]\n\n\
                     Options:\n  \
                       -p, --port <PORT>        TCP port for WebSocket signaling [default: 8443]\n  \
                       -u, --udp-port <PORT>    UDP port for fallback voice relay [default: 8444 (or TCP port + 1)]\n  \
                       -mr, --main-root <VALUE> Main root room 1111 (active | inactive) [default: inactive]\n  \
                       -h, --help               Print help information"
                );
                return Ok(None);
            }
            "-p" | "--port" => {
                let val = iter
                    .next()
                    .ok_or_else(|| format!("missing port argument for {s}"))?;
                let port = val
                    .as_ref()
                    .parse::<u16>()
                    .map_err(|e| format!("invalid port '{}': {e}", val.as_ref()))?;
                tcp_port = Some(port);
            }
            "-u" | "--udp-port" => {
                let val = iter
                    .next()
                    .ok_or_else(|| format!("missing port argument for {s}"))?;
                let port = val
                    .as_ref()
                    .parse::<u16>()
                    .map_err(|e| format!("invalid port '{}': {e}", val.as_ref()))?;
                udp_port = Some(port);
            }
            "-mr" | "--main-root" => {
                let val = iter
                    .next()
                    .ok_or_else(|| format!("missing main-root value for {s}"))?;
                match val.as_ref().to_ascii_lowercase().as_str() {
                    "active" => main_root = true,
                    "inactive" => main_root = false,
                    other => {
                        return Err(format!(
                            "invalid --main-root value '{other}': expected 'active' or 'inactive'"
                        ));
                    }
                }
            }
            opt if opt.starts_with("-p=") || opt.starts_with("--port=") => {
                let val = opt.split_once('=').unwrap().1;
                let port = val
                    .parse::<u16>()
                    .map_err(|e| format!("invalid port '{val}': {e}"))?;
                tcp_port = Some(port);
            }
            opt if opt.starts_with("-u=") || opt.starts_with("--udp-port=") => {
                let val = opt.split_once('=').unwrap().1;
                let port = val
                    .parse::<u16>()
                    .map_err(|e| format!("invalid port '{val}': {e}"))?;
                udp_port = Some(port);
            }
            opt if opt.starts_with("-mr=") || opt.starts_with("--main-root=") => {
                let val = opt.split_once('=').unwrap().1;
                match val.to_ascii_lowercase().as_str() {
                    "active" => main_root = true,
                    "inactive" => main_root = false,
                    other => {
                        return Err(format!(
                            "invalid --main-root value '{other}': expected 'active' or 'inactive'"
                        ));
                    }
                }
            }
            other => {
                return Err(format!("unknown option '{other}'. Run with --help for usage."));
            }
        }
    }

    let final_tcp = tcp_port.unwrap_or(DEFAULT_TCP_PORT);
    let final_udp = udp_port.unwrap_or_else(|| {
        if tcp_port.is_some() {
            final_tcp.wrapping_add(1)
        } else {
            DEFAULT_UDP_PORT
        }
    });

    Ok(Some(ServerConfig {
        tcp_port: final_tcp,
        udp_port: final_udp,
        main_root,
    }))
}

#[tokio::main]
async fn main() {
    let config = match parse_args_from(std::env::args().skip(1)) {
        Ok(Some(c)) => c,
        Ok(None) => std::process::exit(0),
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    };

    tracing_subscriber::fmt::init();

    let state = AppState::new();

    if config.main_root {
        let mut rooms = state.rooms.write().await;
        rooms.insert(
            MAIN_ROOT_ROOM.to_string(),
            Room {
                peers: HashMap::new(),
            },
        );
        tracing::info!(room = %MAIN_ROOT_ROOM, "main root room initialized (permanent)");
    }
    let relay_hub = SharedRelayHub::default();

    // Spawn background pruner so dead UDP relay targets and stale MusicBot
    // entries are swept out of every room (including MAIN_ROOT_ROOM "1111").
    let prune_hub = relay_hub.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval_at(
            tokio::time::Instant::now() + std::time::Duration::from_secs(10),
            std::time::Duration::from_secs(10),
        );
        loop {
            interval.tick().await;
            prune_hub
                .prune_stale(std::time::Duration::from_secs(30))
                .await;
        }
    });

    // Spawn Fallback UDP Relay on configured UDP port
    let hub_clone1 = relay_hub.clone();
    let udp_port = config.udp_port;
    tokio::spawn(async move {
        run_udp_relay(hub_clone1, udp_port).await;
    });

    // If configured UDP port is not 3478, also spawn on standard VoIP port 3478 (STUN + Relay multiplexed)
    if udp_port != 3478 {
        let hub_clone2 = relay_hub.clone();
        tokio::spawn(async move {
            run_udp_relay(hub_clone2, 3478).await;
        });
    }

    // Build Axum WebSocket app on configured TCP port
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], config.tcp_port));
    tracing::info!("Verio signaling server listening on ws://{addr}/ws");
    tracing::info!("Verio fallback UDP relay listening on 0.0.0.0:{udp_port}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind tcp listener");
    axum::serve(listener, app).await.expect("serve axum app");
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let conn_id = state.allocate_conn_id();
    let close_notify = Arc::new(Notify::new());
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Forward messages from mpsc channel to the actual WebSocket
    let send_exit_notify = Arc::clone(&close_notify);
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sender.send(msg).await.is_err() {
                break;
            }
        }
        send_exit_notify.notify_one();
    });

    let mut current_room: Option<String> = None;
    let mut current_peer_id: Option<String> = None;
    let mut ping_interval = tokio::time::interval_at(
        tokio::time::Instant::now() + std::time::Duration::from_secs(20),
        std::time::Duration::from_secs(20),
    );
    let mut last_rx = tokio::time::Instant::now();

    loop {
        let msg = tokio::select! {
            _ = close_notify.notified() => {
                tracing::debug!(conn_id, "closing signaling socket due to send failure or session replacement");
                break;
            }
            _ = ping_interval.tick() => {
                if last_rx.elapsed() >= std::time::Duration::from_secs(90) {
                    tracing::warn!(conn_id, "closing inactive signaling websocket after 90s without traffic");
                    break;
                }
                if tx.send(Message::Ping(Vec::new().into())).is_err() {
                    break;
                }
                continue;
            }
            next = receiver.next() => {
                match next {
                    Some(Ok(msg)) => {
                        last_rx = tokio::time::Instant::now();
                        msg
                    }
                    Some(Err(e)) => {
                        tracing::debug!(conn_id, %e, "websocket receive ended");
                        break;
                    }
                    None => break,
                }
            }
        };

        let text = match msg {
            Message::Text(t) => t.to_string(),
            Message::Binary(b) => match String::from_utf8(b.to_vec()) {
                Ok(s) => s,
                Err(_) => continue,
            },
            Message::Close(_) => break,
            _ => continue,
        };

        let client_msg: ClientMessage = match serde_json::from_str(&text) {
            Ok(m) => m,
            Err(e) => {
                let err = ServerMessage::Error {
                    message: format!("invalid message JSON: {e}"),
                };
                if let Ok(err_json) = serde_json::to_string(&err) {
                    let _ = tx.send(Message::Text(err_json.into()));
                }
                continue;
            }
        };

        match client_msg {
            ClientMessage::Create { identity } => {
                if let (Some(prev_room), Some(prev_peer)) =
                    (current_room.take(), current_peer_id.take())
                {
                    remove_peer_from_room(&state, &prev_room, &prev_peer, conn_id).await;
                }
                let peer_id = identity.uuid.clone();
                let code = create_room_session(
                    &state,
                    identity,
                    conn_id,
                    tx.clone(),
                    Arc::clone(&close_notify),
                )
                .await;
                current_room = Some(code);
                current_peer_id = Some(peer_id);
            }

            ClientMessage::Join { room, identity } => {
                if let (Some(prev_room), Some(prev_peer)) =
                    (current_room.take(), current_peer_id.take())
                {
                    remove_peer_from_room(&state, &prev_room, &prev_peer, conn_id).await;
                }
                let peer_id = identity.uuid.clone();
                if join_room_session(
                    &state,
                    &room,
                    identity,
                    conn_id,
                    tx.clone(),
                    Arc::clone(&close_notify),
                )
                .await
                {
                    current_room = Some(room);
                    current_peer_id = Some(peer_id);
                }
            }

            ClientMessage::Offer { room, target, sdp } => {
                let rooms = state.rooms.read().await;
                if let Some(r) = rooms.get(&room) {
                    if let Some(target_peer) = r.peers.get(&target) {
                        let from = current_peer_id.clone().unwrap_or_default();
                        let relay = ServerMessage::Offer { room, from, sdp };
                        if let Ok(json) = serde_json::to_string(&relay) {
                            let _ = target_peer.tx.send(Message::Text(json.into()));
                        }
                    }
                }
            }

            ClientMessage::Answer { room, target, sdp } => {
                let rooms = state.rooms.read().await;
                if let Some(r) = rooms.get(&room) {
                    if let Some(target_peer) = r.peers.get(&target) {
                        let from = current_peer_id.clone().unwrap_or_default();
                        let relay = ServerMessage::Answer { room, from, sdp };
                        if let Ok(json) = serde_json::to_string(&relay) {
                            let _ = target_peer.tx.send(Message::Text(json.into()));
                        }
                    }
                }
            }

            ClientMessage::Candidate {
                room,
                target,
                candidate,
            } => {
                let rooms = state.rooms.read().await;
                if let Some(r) = rooms.get(&room) {
                    if let Some(target_peer) = r.peers.get(&target) {
                        let from = current_peer_id.clone().unwrap_or_default();
                        let relay = ServerMessage::Candidate {
                            room,
                            from,
                            candidate,
                        };
                        if let Ok(json) = serde_json::to_string(&relay) {
                            let _ = target_peer.tx.send(Message::Text(json.into()));
                        }
                    }
                }
            }

            ClientMessage::Leave { room } => {
                if let Some(peer_id) = current_peer_id.take() {
                    remove_peer_from_room(&state, &room, &peer_id, conn_id).await;
                }
                current_room = None;
            }
        }
    }

    // Clean up on disconnect
    if let (Some(room), Some(peer_id)) = (current_room, current_peer_id) {
        remove_peer_from_room(&state, &room, &peer_id, conn_id).await;
    }

    send_task.abort();
}

async fn create_room_session(
    state: &AppState,
    identity: PeerIdentity,
    conn_id: u64,
    tx: mpsc::UnboundedSender<Message>,
    close_notify: Arc<Notify>,
) -> String {
    let peer_id = identity.uuid.clone();
    let mut rooms = state.rooms.write().await;

    // Generate random 4-digit code (1000..=9999) excluding MAIN_ROOT_ROOM
    let mut rng = rand::thread_rng();
    let mut code;
    loop {
        code = format!("{:04}", rng.gen_range(1000..=9999));
        if code != MAIN_ROOT_ROOM && !rooms.contains_key(&code) {
            break;
        }
    }

    let mut peers = HashMap::new();
    peers.insert(
        peer_id.clone(),
        PeerSession {
            conn_id,
            identity: identity.clone(),
            tx: tx.clone(),
            close_notify,
        },
    );
    rooms.insert(code.clone(), Room { peers });

    tracing::info!(room = %code, host = %identity.name, conn_id, "room created");
    let resp = ServerMessage::Created {
        room: code.clone(),
        peer_id,
    };
    if let Ok(resp_json) = serde_json::to_string(&resp) {
        let _ = tx.send(Message::Text(resp_json.into()));
    }
    code
}

async fn join_room_session(
    state: &AppState,
    room: &str,
    identity: PeerIdentity,
    conn_id: u64,
    tx: mpsc::UnboundedSender<Message>,
    close_notify: Arc<Notify>,
) -> bool {
    let peer_id = identity.uuid.clone();
    let mut rooms = state.rooms.write().await;

    if let Some(r) = rooms.get_mut(room) {
        // Prune dead senders first so closed WebSockets never linger as ghosts.
        r.peers.retain(|existing_id, p| {
            let alive = !p.tx.is_closed();
            if !alive {
                tracing::warn!(room = %room, peer_id = %existing_id, "pruned dead signaling session before join");
            }
            alive
        });

        // If the same peer_id reconnects before its previous socket closed,
        // replace it immediately and signal the old socket task to shut down.
        if let Some(old) = r.peers.remove(&peer_id) {
            tracing::info!(
                room = %room,
                %peer_id,
                old_conn_id = old.conn_id,
                new_conn_id = conn_id,
                "replacing superseded session for same peer_id"
            );
            old.close_notify.notify_one();
        }

        // Notify existing peers, pruning any sender that fails along the way.
        let notify = ServerMessage::PeerJoined {
            room: room.to_string(),
            peer: identity.clone(),
        };
        if let Ok(notify_json) = serde_json::to_string(&notify) {
            r.peers.retain(|other_id, p| {
                let ok = !p.tx.is_closed()
                    && p.tx.send(Message::Text(notify_json.clone().into())).is_ok();
                if !ok {
                    tracing::warn!(room = %room, peer_id = %other_id, "pruned dead signaling session during PeerJoined");
                }
                ok
            });
        }

        // Build Joined payload from living remote peers only (self not yet inserted).
        let existing_peers: Vec<PeerIdentity> =
            r.peers.values().map(|p| p.identity.clone()).collect();

        r.peers.insert(
            peer_id,
            PeerSession {
                conn_id,
                identity: identity.clone(),
                tx: tx.clone(),
                close_notify,
            },
        );

        tracing::info!(room = %room, peer = %identity.name, conn_id, "peer joined room");

        let resp = ServerMessage::Joined {
            room: room.to_string(),
            peers: existing_peers,
        };
        if let Ok(resp_json) = serde_json::to_string(&resp) {
            let _ = tx.send(Message::Text(resp_json.into()));
        }
        true
    } else {
        let err = ServerMessage::Error {
            message: format!("room {room} not found"),
        };
        if let Ok(err_json) = serde_json::to_string(&err) {
            let _ = tx.send(Message::Text(err_json.into()));
        }
        false
    }
}

async fn remove_peer_from_room(state: &AppState, room_code: &str, peer_id: &str, conn_id: u64) {
    let mut rooms = state.rooms.write().await;
    let mut is_empty = false;
    if let Some(r) = rooms.get_mut(room_code) {
        let should_remove = r
            .peers
            .get(peer_id)
            .is_some_and(|session| session.conn_id == conn_id);
        if !should_remove {
            tracing::debug!(
                room = %room_code,
                peer_id = %peer_id,
                conn_id,
                "ignoring leave/disconnect for superseded or unknown session"
            );
            return;
        }
        r.peers.remove(peer_id);
        tracing::info!(room = %room_code, peer_id = %peer_id, conn_id, "peer left room");

        // Notify remaining peers and prune closed channels immediately
        let notify = ServerMessage::PeerLeft {
            room: room_code.to_string(),
            peer_id: peer_id.to_string(),
        };
        if let Ok(json) = serde_json::to_string(&notify) {
            r.peers.retain(|other_id, p| {
                let ok = !p.tx.is_closed()
                    && p.tx.send(Message::Text(json.clone().into())).is_ok();
                if !ok {
                    tracing::warn!(room = %room_code, peer_id = %other_id, "pruned dead signaling session during PeerLeft");
                }
                ok
            });
        }

        if r.peers.is_empty() {
            if room_code != MAIN_ROOT_ROOM {
                is_empty = true;
            } else {
                tracing::info!(room = %room_code, "main root room 1111 is empty but kept alive");
            }
        }
    }
    if is_empty {
        rooms.remove(room_code);
        tracing::info!(room = %room_code, "empty room closed");
    }
}

#[derive(Clone)]
pub struct RelayClient {
    pub addr: SocketAddr,
    pub last_seen: std::time::Instant,
    pub socket: Arc<tokio::net::UdpSocket>,
}

/// Session 7: relay packet-log throttle. The first 20 packets are always logged,
/// after that at most 10 lines per second so a 50 pps audio stream cannot flood
/// the server log.
const RELAY_LOG_BURST: u32 = 20;
const RELAY_LOG_PER_SEC: u32 = 10;

struct PacketLogThrottle {
    seen: u32,
    window_start: std::time::Instant,
    in_window: u32,
}

impl PacketLogThrottle {
    fn new() -> Self {
        Self {
            seen: 0,
            window_start: std::time::Instant::now(),
            in_window: 0,
        }
    }

    fn allow(&mut self) -> bool {
        if self.seen < RELAY_LOG_BURST {
            self.seen += 1;
            return true;
        }
        let now = std::time::Instant::now();
        if now.duration_since(self.window_start) >= std::time::Duration::from_secs(1) {
            self.window_start = now;
            self.in_window = 0;
        }
        if self.in_window < RELAY_LOG_PER_SEC {
            self.in_window += 1;
            self.seen += 1;
            true
        } else {
            false
        }
    }
}

/// Short (first 4 bytes, hex) form of a peer UUID for packet logs.
fn short_uuid(uuid: &[u8; 16]) -> String {
    uuid[..4].iter().map(|b| format!("{b:02x}")).collect()
}

/// Clients in one room, keyed by their 16-byte peer UUID.
pub type RoomClients = HashMap<[u8; 16], RelayClient>;
/// Every live room, keyed by room code.
pub type Rooms = Arc<RwLock<HashMap<String, RoomClients>>>;

#[derive(Clone, Default)]
pub struct SharedRelayHub {
    pub rooms: Rooms,
}

impl SharedRelayHub {
    pub async fn prune_stale(&self, max_age: std::time::Duration) {
        let mut rooms = self.rooms.write().await;
        let now = std::time::Instant::now();
        rooms.retain(|room, clients| {
            clients.retain(|_uuid, client| now.duration_since(client.last_seen) < max_age);
            room == MAIN_ROOT_ROOM || !clients.is_empty()
        });
    }
}

/// Fallback UDP voice relay + RFC 5389 STUN responder:
/// - RFC 5389 STUN Binding Requests are answered with XOR-MAPPED-ADDRESS.
/// - Verio voice relay packets `[4 bytes room code][16 bytes sender UUID][payload]`
///   are routed to other peers registered in that room code.
async fn run_udp_relay(relay_hub: SharedRelayHub, port: u16) {
    let socket = match tokio::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], port))).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::error!("failed to bind UDP relay/STUN socket on port {port}: {e}");
            return;
        }
    };
    tracing::info!("Verio UDP relay + STUN responder listening on 0.0.0.0:{port}");

    let mut buf = [0u8; 4096];
    let mut pkt_log = PacketLogThrottle::new();

    loop {
        let (n, from) = match socket.recv_from(&mut buf).await {
            Ok(res) => res,
            Err(_) => continue,
        };

        // RFC 5389 STUN Binding Request responder: 20 bytes, type 0x0001, magic 0x2112A442
        if n == 20 && buf[0..2] == [0x00, 0x01] && buf[4..8] == [0x21, 0x12, 0xa4, 0x42] {
            let mut resp = Vec::with_capacity(32);
            resp.extend_from_slice(&[0x01, 0x01]); // Binding Success Response
            resp.extend_from_slice(&12u16.to_be_bytes()); // Message length (12 bytes for 1 attribute)
            resp.extend_from_slice(&[0x21, 0x12, 0xa4, 0x42]); // Magic cookie
            resp.extend_from_slice(&buf[8..20]); // Transaction ID

            // XOR-MAPPED-ADDRESS attribute (0x0020)
            resp.extend_from_slice(&0x0020u16.to_be_bytes());
            resp.extend_from_slice(&8u16.to_be_bytes()); // attr length
            resp.push(0x00); // reserved
            resp.push(0x01); // IPv4 family
            let port_xor = from.port() ^ 0x2112;
            resp.extend_from_slice(&port_xor.to_be_bytes());
            match from.ip() {
                std::net::IpAddr::V4(ipv4) => {
                    let ip_u32 = u32::from(ipv4) ^ 0x2112A442;
                    resp.extend_from_slice(&ip_u32.to_be_bytes());
                }
                std::net::IpAddr::V6(_) => continue,
            }
            let _ = socket.send_to(&resp, from).await;
            continue;
        }

        // Minimum packet: 4 bytes room code + 16 bytes UUID
        if n < 20 {
            continue;
        }

        // Session 10: clients on networks that drop unrecognised outbound UDP wrap
        // relay datagrams in a 20-byte STUN Binding-Request-shaped envelope. The
        // genuine STUN responder branch above only matches bare 20-byte requests, so
        // a longer packet carrying that signature is an envelope to strip here.
        let payload: &[u8] = if n > 20
            && buf[0..2] == [0x00, 0x01]
            && buf[4..8] == [0x21, 0x12, 0xa4, 0x42]
        {
            &buf[20..n]
        } else {
            &buf[..n]
        };

        if payload.len() < 20 {
            continue;
        }

        let room_code = match std::str::from_utf8(&payload[0..4]) {
            Ok(s) => s.to_string(),
            Err(_) => continue,
        };

        let mut sender_uuid = [0u8; 16];
        sender_uuid.copy_from_slice(&payload[4..20]);

        let uuid_short = short_uuid(&sender_uuid);

        // Register client in shared relay hub
        {
            let mut rooms = relay_hub.rooms.write().await;
            let clients = rooms.entry(room_code.clone()).or_default();
            let is_new = !clients.contains_key(&sender_uuid);
            clients.insert(
                sender_uuid,
                RelayClient {
                    addr: from,
                    last_seen: std::time::Instant::now(),
                    socket: Arc::clone(&socket),
                },
            );
            if is_new {
                tracing::info!(
                    room = %room_code,
                    client = %from,
                    port,
                    "registered new UDP relay client in room"
                );
            }
            if pkt_log.allow() {
                tracing::debug!(
                    room = %room_code,
                    uuid = %uuid_short,
                    addr = %from,
                    "registered"
                );
            }
        }

        // Relay packet to all other peers in the room (inner payload, unwrapped:
        // the server -> client direction is not filtered, so no re-wrapping).
        if payload.len() > 20 {
            let rooms = relay_hub.rooms.read().await;
            match rooms.get(&room_code) {
                Some(clients) => {
                    let peers_in_room = clients.len();
                    if pkt_log.allow() {
                        tracing::debug!(
                            room = %room_code,
                            from_uuid = %uuid_short,
                            from_addr = %from,
                            len = payload.len(),
                            peers_in_room,
                            "relay_in"
                        );
                    }
                    let mut forwarded = 0usize;
                    for (&uuid, client) in clients.iter() {
                        if uuid != sender_uuid && client.addr != from {
                            let _ = client.socket.send_to(payload, client.addr).await;
                            forwarded += 1;
                            if pkt_log.allow() {
                                tracing::debug!(
                                    room = %room_code,
                                    to_uuid = %short_uuid(&uuid),
                                    to_addr = %client.addr,
                                    len = payload.len(),
                                    "relay_out"
                                );
                            }
                        }
                    }
                    if forwarded == 0 && pkt_log.allow() {
                        tracing::debug!(
                            room = %room_code,
                            reason = "no_peers",
                            from_uuid = %uuid_short,
                            "drop"
                        );
                    }
                }
                None => {
                    if pkt_log.allow() {
                        tracing::debug!(
                            room = %room_code,
                            reason = "unknown_room",
                            from_uuid = %uuid_short,
                            "drop"
                        );
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_defaults() {
        let cfg = parse_args_from(Vec::<&str>::new()).unwrap().unwrap();
        assert_eq!(cfg.tcp_port, DEFAULT_TCP_PORT);
        assert_eq!(cfg.udp_port, DEFAULT_UDP_PORT);
    }

    #[test]
    fn test_parse_args_short_port() {
        let cfg = parse_args_from(["-p", "9000"]).unwrap().unwrap();
        assert_eq!(cfg.tcp_port, 9000);
        assert_eq!(cfg.udp_port, 9001);
    }

    #[test]
    fn test_parse_args_long_port() {
        let cfg = parse_args_from(["--port", "9000"]).unwrap().unwrap();
        assert_eq!(cfg.tcp_port, 9000);
        assert_eq!(cfg.udp_port, 9001);
    }

    #[test]
    fn test_parse_args_equals_port() {
        let cfg = parse_args_from(["--port=9000"]).unwrap().unwrap();
        assert_eq!(cfg.tcp_port, 9000);
        assert_eq!(cfg.udp_port, 9001);
    }

    #[test]
    fn test_parse_args_both_ports() {
        let cfg = parse_args_from(["-p", "7000", "-u", "7050"]).unwrap().unwrap();
        assert_eq!(cfg.tcp_port, 7000);
        assert_eq!(cfg.udp_port, 7050);
    }

    #[test]
    fn test_parse_args_invalid_port() {
        assert!(parse_args_from(["-p", "not_a_number"]).is_err());
        assert!(parse_args_from(["-p"]).is_err());
        assert!(parse_args_from(["--unknown-flag"]).is_err());
    }

    #[test]
    fn test_parse_args_main_root() {
        let cfg = parse_args_from(["-mr", "active"]).unwrap().unwrap();
        assert!(cfg.main_root);

        let cfg = parse_args_from(["--main-root", "active"]).unwrap().unwrap();
        assert!(cfg.main_root);

        let cfg = parse_args_from(["-mr=active"]).unwrap().unwrap();
        assert!(cfg.main_root);

        let cfg = parse_args_from(["--main-root=active"]).unwrap().unwrap();
        assert!(cfg.main_root);

        let cfg = parse_args_from(["-mr", "inactive"]).unwrap().unwrap();
        assert!(!cfg.main_root);

        let cfg = parse_args_from(["--main-root=inactive"]).unwrap().unwrap();
        assert!(!cfg.main_root);

        assert!(parse_args_from(["-mr", "unknown"]).is_err());
        assert!(parse_args_from(["--main-root=foo"]).is_err());
    }

    #[tokio::test]
    async fn test_main_root_retained_when_empty() {
        let state = AppState::new();
        {
            let mut rooms = state.rooms.write().await;
            rooms.insert(
                MAIN_ROOT_ROOM.to_string(),
                Room {
                    peers: HashMap::new(),
                },
            );
        }

        remove_peer_from_room(&state, MAIN_ROOT_ROOM, "peer-1", 1).await;

        let rooms = state.rooms.read().await;
        assert!(
            rooms.contains_key(MAIN_ROOT_ROOM),
            "1111 must be kept alive even when empty"
        );
    }

    #[tokio::test]
    async fn test_stale_conn_id_cannot_evict_newer_session_for_same_peer() {
        let state = AppState::new();
        state.rooms.write().await.insert(
            MAIN_ROOT_ROOM.to_string(),
            Room {
                peers: HashMap::new(),
            },
        );
        let id = PeerIdentity {
            uuid: "peer-uuid-1".into(),
            user_id: "user-1".into(),
            is_bot: false,
            force_stop: false,
            name: "Alice".into(),
            version: "0.1.0".into(),
        };

        let (tx1, _rx1) = mpsc::unbounded_channel();
        let notify1 = Arc::new(Notify::new());
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let notify2 = Arc::new(Notify::new());

        assert!(join_room_session(&state, MAIN_ROOT_ROOM, id.clone(), 1, tx1, notify1).await);
        assert!(join_room_session(&state, MAIN_ROOT_ROOM, id.clone(), 2, tx2, notify2).await);

        // Old conn_id 1 finishes closing late: must NOT remove conn_id 2!
        remove_peer_from_room(&state, MAIN_ROOT_ROOM, "peer-uuid-1", 1).await;

        {
            let rooms = state.rooms.read().await;
            let r = rooms.get(MAIN_ROOT_ROOM).unwrap();
            assert_eq!(r.peers.len(), 1);
            assert_eq!(r.peers.get("peer-uuid-1").unwrap().conn_id, 2);
        }

        // Joining message for conn_id 2 must NOT contain self as an existing peer.
        let joined_msg = rx2.recv().await.unwrap();
        let Message::Text(joined_text) = joined_msg else {
            panic!("expected text Joined frame");
        };
        let parsed: ServerMessage = serde_json::from_str(&joined_text).unwrap();
        match parsed {
            ServerMessage::Joined { peers, .. } => {
                assert!(peers.is_empty(), "self must never appear in Joined peers");
            }
            other => panic!("unexpected server message: {other:?}"),
        }

        // Current conn_id 2 leaving DOES remove the peer cleanly.
        remove_peer_from_room(&state, MAIN_ROOT_ROOM, "peer-uuid-1", 2).await;
        let rooms = state.rooms.read().await;
        assert!(rooms.get(MAIN_ROOT_ROOM).unwrap().peers.is_empty());
    }

    #[tokio::test]
    async fn test_join_prunes_dead_senders_and_empty_1111_rejoin_succeeds() {
        let state = AppState::new();
        state.rooms.write().await.insert(
            MAIN_ROOT_ROOM.to_string(),
            Room {
                peers: HashMap::new(),
            },
        );
        let ghost_id = PeerIdentity {
            uuid: "ghost-bot".into(),
            user_id: "user-1".into(),
            is_bot: true,
            force_stop: true,
            name: "Music".into(),
            version: "0.1.0".into(),
        };
        let (ghost_tx, ghost_rx) = mpsc::unbounded_channel();
        assert!(
            join_room_session(
                &state,
                MAIN_ROOT_ROOM,
                ghost_id,
                1,
                ghost_tx,
                Arc::new(Notify::new())
            )
            .await
        );
        // Simulate abrupt WebSocket death without Leave: receiver dropped.
        drop(ghost_rx);

        let alice = PeerIdentity {
            uuid: "alice-1".into(),
            user_id: "user-alice".into(),
            is_bot: false,
            force_stop: false,
            name: "Alice".into(),
            version: "0.1.0".into(),
        };
        let (tx_a1, mut rx_a1) = mpsc::unbounded_channel();
        assert!(
            join_room_session(
                &state,
                MAIN_ROOT_ROOM,
                alice.clone(),
                2,
                tx_a1,
                Arc::new(Notify::new())
            )
            .await
        );
        let msg = rx_a1.recv().await.unwrap();
        let Message::Text(txt) = msg else {
            panic!("expected text");
        };
        let ServerMessage::Joined { peers, .. } = serde_json::from_str(&txt).unwrap() else {
            panic!("expected Joined");
        };
        assert!(
            peers.is_empty(),
            "dead ghost channel must be pruned before building Joined.peers"
        );

        // Leave empty 1111 and rejoin repeatedly.
        remove_peer_from_room(&state, MAIN_ROOT_ROOM, "alice-1", 2).await;
        drop(rx_a1);

        let (tx_a2, mut rx_a2) = mpsc::unbounded_channel();
        assert!(
            join_room_session(
                &state,
                MAIN_ROOT_ROOM,
                alice,
                3,
                tx_a2,
                Arc::new(Notify::new())
            )
            .await
        );
        let msg2 = rx_a2.recv().await.unwrap();
        let Message::Text(txt2) = msg2 else {
            panic!("expected text");
        };
        let ServerMessage::Joined { room, peers } = serde_json::from_str(&txt2).unwrap() else {
            panic!("expected Joined");
        };
        assert_eq!(room, MAIN_ROOT_ROOM);
        assert!(peers.is_empty());
    }

    #[tokio::test]
    async fn test_shared_relay_hub_prune_stale_sweeps_1111_and_removes_empty_rooms() {
        let hub = SharedRelayHub::default();
        let sock = Arc::new(tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        let now = std::time::Instant::now();
        {
            let mut rooms = hub.rooms.write().await;
            let main_clients = rooms.entry(MAIN_ROOT_ROOM.to_string()).or_default();
            main_clients.insert(
                [1u8; 16],
                RelayClient {
                    addr,
                    last_seen: now - std::time::Duration::from_secs(45),
                    socket: Arc::clone(&sock),
                },
            );
            main_clients.insert(
                [2u8; 16],
                RelayClient {
                    addr,
                    last_seen: now - std::time::Duration::from_secs(5),
                    socket: Arc::clone(&sock),
                },
            );
            let old_room = rooms.entry("9999".to_string()).or_default();
            old_room.insert(
                [3u8; 16],
                RelayClient {
                    addr,
                    last_seen: now - std::time::Duration::from_secs(45),
                    socket: Arc::clone(&sock),
                },
            );
        }

        hub.prune_stale(std::time::Duration::from_secs(30)).await;

        let rooms = hub.rooms.read().await;
        assert!(!rooms.contains_key("9999"), "empty non-1111 room must be removed");
        let main_clients = rooms.get(MAIN_ROOT_ROOM).unwrap();
        assert!(!main_clients.contains_key(&[1u8; 16]), "stale client in 1111 must be pruned");
        assert!(main_clients.contains_key(&[2u8; 16]), "fresh client in 1111 must remain");
    }
}

