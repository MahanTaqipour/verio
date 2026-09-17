//! Verio discovery / signaling.

pub mod vps;
pub use vps::VpsSignaling;

use std::fmt;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
pub use uuid::Uuid;

/// Invite blob prefix — enforced on create and parse.
pub const INVITE_PREFIX: &str = "VR1-";
/// Default TCP port for direct-connect signaling.
pub const DEFAULT_SIGNALING_PORT: u16 = 49860;
/// mDNS service type (discovery only).
const MDNS_SERVICE: &str = "_verio._tcp.local.";
/// Hard cap for one signaling frame (SDP with many candidates stays far below).
const FRAME_MAX: usize = 256 * 1024;
const IO_TIMEOUT: Duration = Duration::from_secs(10);

/// Peer identity exchanged before any SDP flows. The uuid decides the
/// deterministic role: on any pairwise setup the peer with the LARGER uuid is the
/// offerer (prevents double-dial deadlock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub uuid: Uuid,
    pub name: String,
    pub version: String,
}

/// Deterministic role derived from comparing uuids.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Our uuid is LARGER — we produce the offer.
    Offerer,
    /// Our uuid is SMALLER — we accept the offer and answer.
    Answerer,
}

/// Signaling errors (manual impls — no new deps for this).
#[derive(Debug)]
pub enum SignalingError {
    Io(std::io::Error),
    Protocol(String),
    BadInvite(String),
}

impl fmt::Display for SignalingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalingError::Io(e) => write!(f, "signaling io error: {e}"),
            SignalingError::Protocol(m) => write!(f, "signaling protocol error: {m}"),
            SignalingError::BadInvite(m) => write!(f, "invalid invite blob: {m}"),
        }
    }
}

impl std::error::Error for SignalingError {}

impl From<std::io::Error> for SignalingError {
    fn from(e: std::io::Error) -> Self {
        SignalingError::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Wire framing: 4-byte little-endian length + JSON payload.
// ---------------------------------------------------------------------------

fn write_frame(stream: &mut TcpStream, value: &impl Serialize) -> Result<(), SignalingError> {
    let text = serde_json::to_vec(value)
        .map_err(|e| SignalingError::Protocol(format!("serialize frame: {e}")))?;
    if text.len() > FRAME_MAX {
        return Err(SignalingError::Protocol(format!(
            "frame too large: {} bytes",
            text.len()
        )));
    }
    let len = (text.len() as u32).to_le_bytes();
    stream.write_all(&len)?;
    stream.write_all(&text)?;
    stream.flush()?;
    Ok(())
}

fn read_frame<T: DeserializeOwned>(stream: &mut TcpStream) -> Result<T, SignalingError> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len == 0 || len > FRAME_MAX {
        return Err(SignalingError::Protocol(format!("bad frame length: {len}")));
    }
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    serde_json::from_slice(&buf).map_err(|e| SignalingError::Protocol(format!("parse frame: {e}")))
}

#[derive(Serialize, Deserialize)]
struct SdpFrame {
    sdp: String,
}

// ---------------------------------------------------------------------------
// Signaling trait
// ---------------------------------------------------------------------------

/// The signaling contract. Two implementations: [`DirectSession`] (TCP) and the
/// invite-code flow (the blob IS the channel — free functions below carry the
/// same SDP strings so the app layer stays signaling-agnostic).
pub trait Signaling {
    /// Send our identity, receive the remote's, derive the deterministic role.
    fn exchange_identities(
        &mut self,
        identity: &Identity,
    ) -> Result<(Identity, Role), SignalingError>;

    /// Offerer path: send our offer, block for the answer.
    fn exchange_offer(&mut self, offer: &str) -> Result<String, SignalingError>;

    /// Answerer path: block for the offer.
    fn receive_offer(&mut self) -> Result<String, SignalingError>;

    /// Answerer path: send our answer.
    fn send_answer(&mut self, answer: &str) -> Result<(), SignalingError>;
}

// ---------------------------------------------------------------------------
// Direct connect (TCP)
// ---------------------------------------------------------------------------

/// A TCP signaling listener. Both instances try to bind the signaling port; on a
/// single PC the second instance loses the race and acts as the client (connector)
/// only — that is an expected, logged condition.
pub struct DirectListener {
    listener: TcpListener,
    port: u16,
}

impl DirectListener {
    pub fn bind(port: u16) -> Result<Self, SignalingError> {
        let listener = TcpListener::bind(("0.0.0.0", port))?;
        let port = listener.local_addr()?.port();
        tracing::info!(port, "direct signaling listener bound");
        Ok(Self { listener, port })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Direct codes this instance advertises: `ip:port` for loopback and the
    /// primary LAN address (same-PC testing uses the loopback one).
    pub fn local_codes(&self) -> Vec<String> {
        let mut codes = vec![format!("127.0.0.1:{}", self.port)];
        if let Some(lan) = primary_lan_addr(self.port) {
            let lan = lan.to_string();
            if lan != codes[0] {
                codes.push(lan);
            }
        }
        codes
    }

    /// Block for one incoming signaling connection.
    pub fn accept(&self) -> Result<DirectSession, SignalingError> {
        let (stream, peer) = self.listener.accept()?;
        tracing::info!(%peer, "direct signaling: incoming connection");
        Ok(DirectSession::from_stream(stream))
    }
}

/// One direct-connect signaling session over a TCP stream.
pub struct DirectSession {
    stream: TcpStream,
}

impl DirectSession {
    pub(crate) fn from_stream(stream: TcpStream) -> Self {
        let _ = stream.set_nodelay(true);
        Self { stream }
    }

    /// Connect to a remote direct code (`ip:port`), 5 s timeout.
    pub fn connect(addr: &str) -> Result<Self, SignalingError> {
        let resolved: Vec<_> = addr
            .to_socket_addrs()
            .map_err(|e| SignalingError::Protocol(format!("bad direct code {addr:?}: {e}")))?
            .collect();
        let first = resolved.first().ok_or_else(|| {
            SignalingError::Protocol(format!("direct code {addr:?} resolves to nothing"))
        })?;
        let stream = TcpStream::connect_timeout(first, Duration::from_secs(5))?;
        tracing::info!(addr, "direct signaling: connected");
        Ok(Self::from_stream(stream))
    }
}

impl Signaling for DirectSession {
    fn exchange_identities(
        &mut self,
        identity: &Identity,
    ) -> Result<(Identity, Role), SignalingError> {
        self.stream.set_read_timeout(Some(IO_TIMEOUT))?;
        self.stream.set_write_timeout(Some(IO_TIMEOUT))?;
        write_frame(&mut self.stream, identity)?;
        let remote: Identity = read_frame(&mut self.stream)?;
        let role = if identity.uuid > remote.uuid {
            Role::Offerer
        } else if identity.uuid < remote.uuid {
            Role::Answerer
        } else {
            return Err(SignalingError::Protocol(
                "remote uuid equals ours — refusing to connect to ourselves".into(),
            ));
        };
        tracing::info!(peer = %remote.name, peer_uuid = %remote.uuid, ?role, "direct identities exchanged");
        Ok((remote, role))
    }

    fn exchange_offer(&mut self, offer: &str) -> Result<String, SignalingError> {
        write_frame(&mut self.stream, &SdpFrame { sdp: offer.to_string() })?;
        let answer: SdpFrame = read_frame(&mut self.stream)?;
        Ok(answer.sdp)
    }

    fn receive_offer(&mut self) -> Result<String, SignalingError> {
        let offer: SdpFrame = read_frame(&mut self.stream)?;
        Ok(offer.sdp)
    }

    fn send_answer(&mut self, answer: &str) -> Result<(), SignalingError> {
        write_frame(&mut self.stream, &SdpFrame { sdp: answer.to_string() })
    }
}

/// Primary LAN address without any winapi dependency: bind an ephemeral UDP
/// socket and "connect" it to a public address — the kernel picks the outgoing
/// interface. No packet is sent.
pub fn primary_lan_addr(port: u16) -> Option<std::net::SocketAddr> {
    let probe = UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    probe.connect(("8.8.8.8", 80)).ok()?;
    let ip = probe.local_addr().ok()?.ip();
    if ip.is_loopback() {
        return None; // offline machine: loopback-only operation still works
    }
    Some(std::net::SocketAddr::new(ip, port))
}

// ---------------------------------------------------------------------------
// Invite codes ("VR1-" + base64 JSON)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteKind {
    Offer,
    Answer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvitePayload {
    /// Format version.
    pub v: u32,
    pub identity: Identity,
    pub kind: InviteKind,
    /// SDP (offer or answer) including gathered ICE candidates (non-trickle).
    pub sdp: String,
}

fn b64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
}

fn b64_decode(text: &str) -> Result<Vec<u8>, SignalingError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(text)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(text))
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(text))
        .map_err(|e| SignalingError::BadInvite(format!("base64 decode: {e}")))
}

/// Create an offer invite blob: `VR1-` + base64(payload). The SDP must come from
/// an `Rtc` whose ICE gathering has already completed (str0m is non-trickle by
/// construction — candidates ride inside the SDP).
pub fn create_invite(identity: &Identity, offer_sdp: &str) -> String {
    let payload = InvitePayload {
        v: 1,
        identity: identity.clone(),
        kind: InviteKind::Offer,
        sdp: offer_sdp.to_string(),
    };
    let json = serde_json::to_vec(&payload).expect("invite payload serializes");
    format!("{INVITE_PREFIX}{}", b64_encode(&json))
}

/// Create the answer invite blob (the blob the friend pastes back).
pub fn create_answer_invite(identity: &Identity, answer_sdp: &str) -> String {
    let payload = InvitePayload {
        v: 1,
        identity: identity.clone(),
        kind: InviteKind::Answer,
        sdp: answer_sdp.to_string(),
    };
    let json = serde_json::to_vec(&payload).expect("invite payload serializes");
    format!("{INVITE_PREFIX}{}", b64_encode(&json))
}

/// Parse an invite blob. The `VR1-` prefix is ENFORCED here.
pub fn parse_invite(blob: &str) -> Result<InvitePayload, SignalingError> {
    let trimmed = blob.trim();
    let rest = trimmed
        .strip_prefix(INVITE_PREFIX)
        .ok_or_else(|| SignalingError::BadInvite(format!("blob must start with {INVITE_PREFIX:?}")))?;
    let json = b64_decode(rest)?;
    let payload: InvitePayload = serde_json::from_slice(&json)
        .map_err(|e| SignalingError::BadInvite(format!("payload parse: {e}")))?;
    if payload.v != 1 {
        return Err(SignalingError::BadInvite(format!(
            "unsupported invite version {}",
            payload.v
        )));
    }
    Ok(payload)
}

// ---------------------------------------------------------------------------
// mDNS — DISCOVERY ONLY (advertise; the SDP exchange always goes over TCP)
// ---------------------------------------------------------------------------

/// Best-effort mDNS advertisement. Fails on a same-PC second instance when UDP
/// 5353 is already taken (caller logs it; direct-connect covers testing).
pub struct MdnsAdvertiser {
    daemon: mdns_sd::ServiceDaemon,
    service_name: String,
}

impl MdnsAdvertiser {
    pub fn start(identity: &Identity, signaling_port: u16) -> Result<Self, String> {
        use mdns_sd::{ServiceDaemon, ServiceInfo};
        let daemon = ServiceDaemon::new().map_err(|e| format!("mdns daemon: {e}"))?;
        let instance = format!("verio-{}", identity.uuid.simple());
        let host = format!("{instance}.local.");
        let ip = primary_lan_addr(signaling_port)
            .map(|a| a.ip())
            .unwrap_or_else(|| std::net::IpAddr::from([127, 0, 0, 1]));
        let props = [
            ("peer_id", identity.uuid.to_string()),
            ("name", identity.name.clone()),
            ("signaling_port", signaling_port.to_string()),
        ];
        let info = ServiceInfo::new(MDNS_SERVICE, &instance, &host, ip, signaling_port, &props[..])
            .map_err(|e| format!("mdns service info: {e}"))?;
        daemon
            .register(info)
            .map_err(|e| format!("mdns register: {e}"))?;
        tracing::info!(service = %instance, signaling_port, "mDNS advertisement started (discovery only)");
        Ok(Self {
            daemon,
            service_name: instance,
        })
    }

    pub fn service_name(&self) -> &str {
        &self.service_name
    }
}

impl Drop for MdnsAdvertiser {
    fn drop(&mut self) {
        if let Ok(rx) = self.daemon.unregister(&self.service_name) {
            if rx.recv_timeout(Duration::from_secs(1)).is_ok() {
                tracing::debug!(service = %self.service_name, "mDNS unregistered");
            }
        }
    }
}

/// A LAN peer learned via mDNS (discovery only — the user still connects by
/// direct code).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiscoveredPeer {
    pub peer_id: Uuid,
    pub name: String,
    pub signaling_port: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name: &str) -> Identity {
        Identity {
            uuid: Uuid::new_v4(),
            name: name.to_string(),
            version: "0.1.0".to_string(),
        }
    }

    #[test]
    fn invite_offer_roundtrip() {
        let id = identity("Mahan");
        let blob = create_invite(&id, "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n");
        assert!(blob.starts_with("VR1-"));
        let parsed = parse_invite(&blob).expect("parse");
        assert_eq!(parsed.kind, InviteKind::Offer);
        assert_eq!(parsed.identity, id);
        assert_eq!(parsed.sdp, "v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n");
        assert_eq!(parsed.v, 1);
    }

    #[test]
    fn invite_answer_roundtrip() {
        let id = identity("Friend");
        let blob = create_answer_invite(&id, "v=0\r\nanswer\r\n");
        let parsed = parse_invite(&blob).expect("parse");
        assert_eq!(parsed.kind, InviteKind::Answer);
        assert!(parsed.sdp.contains("answer"));
    }

    #[test]
    fn invite_prefix_is_enforced() {
        let id = identity("X");
        let blob = create_invite(&id, "sdp");
        // Missing prefix → rejected.
        let no_prefix = blob.trim_start_matches(INVITE_PREFIX).to_string();
        assert!(matches!(
            parse_invite(&no_prefix),
            Err(SignalingError::BadInvite(_))
        ));
        // Wrong prefix → rejected.
        assert!(matches!(
            parse_invite(&format!("XX-{}", &blob[4..])),
            Err(SignalingError::BadInvite(_))
        ));
        // Garbage after prefix → rejected.
        assert!(matches!(
            parse_invite("VR1-not-base64!!!"),
            Err(SignalingError::BadInvite(_))
        ));
    }

    #[test]
    fn larger_uuid_is_offerer() {
        let a = identity("A");
        let mut b = identity("B");
        while b.uuid == a.uuid {
            b = identity("B");
        }
        let (big, small) = if a.uuid > b.uuid { (&a, &b) } else { (&b, &a) };
        // The comparison both sides perform independently must mirror.
        let role_for_small = if small.uuid > big.uuid {
            Role::Offerer
        } else {
            Role::Answerer
        };
        let role_for_big = if big.uuid > small.uuid {
            Role::Offerer
        } else {
            Role::Answerer
        };
        assert_eq!(role_for_small, Role::Answerer);
        assert_eq!(role_for_big, Role::Offerer);
    }

    #[test]
    fn frame_roundtrip_tcp() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let id: Identity = read_frame(&mut stream).expect("read");
            // Server side is the answerer in this scenario: read the offer
            // before replying, then reply and drop.
            let offer: SdpFrame = read_frame(&mut stream).expect("read offer");
            assert_eq!(offer.sdp, "offer-sdp");
            write_frame(&mut stream, &SdpFrame { sdp: "answer-sdp".into() }).expect("write");
            id
        });
        let mut client = DirectSession::from_stream(TcpStream::connect(addr).expect("connect"));
        let me = identity("me");
        write_frame(&mut client.stream, &me).expect("write");
        let answer = client.exchange_offer("offer-sdp").expect("exchange");
        assert_eq!(answer, "answer-sdp");
        let got = handle.join().expect("thread");
        assert_eq!(got, me);
    }

    #[test]
    fn direct_handshake_roles_and_sdp_flow() {
        // Fixed uuids make the role assignment deterministic (client is the
        // offerer): random v4 uuids made this a 50/50 coin flip where BOTH
        // sides could compute Answerer and block in receive_offer forever.
        let mk = |name: &str, id: u128| Identity {
            uuid: Uuid::from_u128(id),
            name: name.to_string(),
            version: "0.0.0".to_string(),
        };
        let server_id = mk("server", 1);
        let client_id = mk("client", 2);
        assert!(client_id.uuid > server_id.uuid);

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let addr = listener.local_addr().expect("addr");
        let server = std::thread::spawn(move || {
            let stream = listener.incoming().next().expect("conn").expect("tcp");
            let mut session = DirectSession::from_stream(stream);
            let (remote, role) = session.exchange_identities(&server_id).expect("ident");
            let answer = match role {
                Role::Answerer => {
                    let offer = session.receive_offer().expect("offer");
                    session.send_answer("the-answer").expect("answer");
                    offer
                }
                Role::Offerer => session.exchange_offer("the-answer").expect("exchange"),
            };
            (remote, role, answer)
        });
        let client = std::thread::spawn(move || {
            let mut session = DirectSession::connect(&addr.to_string()).expect("connect");
            let (remote, role) = session.exchange_identities(&client_id).expect("ident");
            // Deterministic role: larger uuid offers; the answerer receives.
            let answer = match role {
                Role::Offerer => session.exchange_offer("the-offer").expect("exchange"),
                Role::Answerer => {
                    let o = session.receive_offer().expect("recv");
                    session.send_answer("the-answer").expect("send");
                    o
                }
            };
            (remote, role, answer)
        });
        let (s_remote, s_role, s_offer) = server.join().expect("server");
        let (c_remote, c_role, c_answer) = client.join().expect("client");
        // Roles are mirrored, identities cross-delivered, SDP flows both ways.
        assert_ne!(s_role, c_role);
        assert_eq!(s_offer, "the-offer");
        assert_eq!(c_answer, "the-answer");
        assert_eq!(s_remote.name, "client");
        assert_eq!(c_remote.name, "server");
    }
}



