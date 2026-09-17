//! Verio transport: one WebRTC session per remote peer (str0m).
//!
//! Decision-tree outcome (see PROGRESS.md): the libdatachannel `datachannel`
//! bindings are UNBUILDABLE on this machine (bindgen has no libclang; no system
//! OpenSSL; vendored OpenSSL needs Perl) → branch 3: **str0m**, `wincrypto`
//! feature (Windows CNG/SChannel — encryption entirely library-provided).
//!
//! Audio rides an UNRELIABLE UNORDERED DataChannel (`max_retransmits = 0`) with
//! the branch-2 header, one Opus 20 ms frame per message:
//!
//! ```text
//! byte 0      magic      = 0x56 ('V')
//! byte 1      version    = 1
//! bytes 2..6  seq        = u32 LE (loss/late accounting)
//! bytes 6..12 capture_ts = u48 LE (unix ms at encode time → one-way latency)
//! bytes 12..  opus frame
//! ```
//!
//! Control JSON rides a second, RELIABLE ORDERED DataChannel:
//! `hello{name,version}`, `state{speaking,mute}`, `ping{t}` / `pong{t}` every 2 s
//! → RTT shown in both UIs.
//!
//! Deterministic role: the peer with the LARGER uuid is the offerer (enforced in
//! verio-discovery; the offerer declares both channels).
//!
//! Engineering rules: this driver runs on its own thread (never a cpal callback);
//! Result everywhere; tracing only off the real-time path.

use std::io::ErrorKind;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use str0m::change::{SdpAnswer, SdpOffer};
pub use str0m::change::SdpPendingOffer;
use str0m::net::{Protocol, Receive};
use str0m::channel::{ChannelConfig, ChannelData, ChannelId, Reliability};
use str0m::{Candidate, Event, IceConnectionState, Input, Output, Rtc, RtcError};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const AUDIO_MAGIC: u8 = 0x56; // 'V'
pub const AUDIO_VERSION: u8 = 1;
pub const AUDIO_CHANNEL_LABEL: &str = "verio-audio";
pub const CONTROL_CHANNEL_LABEL: &str = "verio-control";
/// Spec: 2-byte Opus silence frame every 400 ms while VAD-silent (decoder warm +
/// NAT mapping alive).
pub const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(400);
/// Spec: control ping/pong every 2 s.
pub const PING_INTERVAL: Duration = Duration::from_secs(2);
/// STUN servers for internet-mode connections (not needed for same-LAN direct).
pub const STUN_SERVERS: &[&str] = &[
    "stun.l.google.com:19302",
    "stun1.l.google.com:19302",
    "stun2.l.google.com:19302",
    "stun.cloudflare.com:3478",
    "stun.miwifi.com:3478",
];

#[cfg(windows)]
fn disable_connection_reset(socket: &UdpSocket) {
    use std::os::windows::io::AsRawSocket;
    const SIO_UDP_CONNRESET: u32 = 0x9800000c;
    let handle = socket.as_raw_socket();
    let mut bytes_returned: u32 = 0;
    let mut enable: u32 = 0; // FALSE to disable WSAECONNRESET on UDP
    #[link(name = "ws2_32")]
    extern "system" {
        fn WSAIoctl(
            s: usize,
            dwIoControlCode: u32,
            lpvInBuffer: *const std::ffi::c_void,
            cbInBuffer: u32,
            lpvOutBuffer: *mut std::ffi::c_void,
            cbOutBuffer: u32,
            lpcbBytesReturned: *mut u32,
            lpOverlapped: *mut std::ffi::c_void,
            lpCompletionRoutine: *mut std::ffi::c_void,
        ) -> i32;
    }
    unsafe {
        let _ = WSAIoctl(
            handle as usize,
            SIO_UDP_CONNRESET,
            &mut enable as *mut _ as *const _,
            std::mem::size_of::<u32>() as u32,
            std::ptr::null_mut(),
            0,
            &mut bytes_returned,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        );
    }
}

#[cfg(not(windows))]
fn disable_connection_reset(_socket: &UdpSocket) {}

const AUDIO_HEADER_LEN: usize = 12;
const BUF_SIZE: usize = 2000;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct TransportError(pub String);

impl std::fmt::Display for TransportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "transport error: {}", self.0)
    }
}

impl std::error::Error for TransportError {}

impl From<RtcError> for TransportError {
    fn from(e: RtcError) -> Self {
        TransportError(format!("str0m: {e}"))
    }
}

impl From<std::io::Error> for TransportError {
    fn from(e: std::io::Error) -> Self {
        TransportError(format!("io: {e}"))
    }
}

// ---------------------------------------------------------------------------
// Audio packet codec (branch-2 12-byte header)
// ---------------------------------------------------------------------------

/// Unix milliseconds right now.
pub fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// One Opus frame + branch-2 header (magic, version, seq u32, capture_ts u48).
pub fn encode_audio_packet(seq: u32, capture_ts_ms: u64, opus: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(AUDIO_HEADER_LEN + opus.len());
    out.push(AUDIO_MAGIC);
    out.push(AUDIO_VERSION);
    out.extend_from_slice(&seq.to_le_bytes());
    // u48 little-endian.
    let ts = capture_ts_ms.to_le_bytes();
    out.extend_from_slice(&ts[..6]);
    out.extend_from_slice(opus);
    out
}

/// Parse an audio packet → `(seq, capture_ts_ms, opus_payload)`. The payload
/// borrows the input buffer.
pub fn parse_audio_packet(buf: &[u8]) -> Result<(u32, u64, &[u8]), TransportError> {
    if buf.len() < AUDIO_HEADER_LEN {
        return Err(TransportError(format!("audio packet too short: {}", buf.len())));
    }
    if buf[0] != AUDIO_MAGIC {
        return Err(TransportError(format!("bad magic: {:#04x}", buf[0])));
    }
    if buf[1] != AUDIO_VERSION {
        return Err(TransportError(format!("bad version: {}", buf[1])));
    }
    let seq = u32::from_le_bytes([buf[2], buf[3], buf[4], buf[5]]);
    let mut ts_bytes = [0u8; 8];
    ts_bytes[..6].copy_from_slice(&buf[6..12]);
    let capture_ts_ms = u64::from_le_bytes(ts_bytes);
    Ok((seq, capture_ts_ms, &buf[12..]))
}

// ---------------------------------------------------------------------------
// Control messages (JSON over the reliable DataChannel)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControlMessage {
    Hello { name: String, version: String },
    State {
        speaking: bool,
        mute: bool,
        /// Session 11: announced so peers can show a "deafened" badge.
        #[serde(default)]
        deafen: bool,
        /// Session 14: this peer is playing music into the room (and its title).
        #[serde(default)]
        music: bool,
        #[serde(default)]
        music_title: String,
    },
    Ping { t: u64 },
    Pong { t: u64 },
}

// ---------------------------------------------------------------------------
// Transport events / commands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// ICE + DTLS established.
    Connected,
    Disconnected { reason: String },
    /// One Opus frame from a remote peer (audio channel). `peer` is the sender's
    /// UUID when the transport can attribute it (the relay fans out to every peer
    /// in the room); it is empty for single-peer transports, where the room already
    /// knows who the peer is.
    Audio {
        peer: String,
        seq: u32,
        capture_ts_ms: u64,
        opus: Vec<u8>,
    },
    /// Control message from a remote peer (attributed for the same reason).
    Control {
        peer: String,
        message: ControlMessage,
    },
    /// RTT measured from our own ping/pong (clock-skew immune).
    Rtt { ms: f64 },
}

pub enum PeerCommand {
    Audio { capture_ts_ms: u64, opus: Vec<u8> },
    Control(ControlMessage),
    Disconnect,
}

/// Live transport diagnostics and network telemetry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportDiagnostics {
    pub mode: String,
    pub endpoint: String,
    pub local_addr: String,
    pub packets_sent: u64,
    pub packets_recv: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
    pub last_recv_ms_ago: Option<u64>,
    pub rtt_ms: Option<f64>,
    pub ice_state: Option<String>,
    pub status: String,
    pub error: Option<String>,
    // --- Session 7 relay instrumentation -------------------------------------
    /// Relay-path counters (voice datagrams only reach these when the relay path is live).
    pub relay_packets_sent: u64,
    pub relay_packets_recv: u64,
    pub relay_bytes_sent: u64,
    pub relay_bytes_recv: u64,
    /// `forced` | `auto-upgraded` | `direct`.
    pub relay_mode: String,
}

impl Default for TransportDiagnostics {
    fn default() -> Self {
        Self {
            mode: "unknown".into(),
            endpoint: "none".into(),
            local_addr: "none".into(),
            packets_sent: 0,
            packets_recv: 0,
            bytes_sent: 0,
            bytes_recv: 0,
            last_recv_ms_ago: None,
            rtt_ms: None,
            ice_state: None,
            status: "Initializing".into(),
            error: None,
            relay_packets_sent: 0,
            relay_packets_recv: 0,
            relay_bytes_sent: 0,
            relay_bytes_recv: 0,
            relay_mode: "direct".into(),
        }
    }
}

// ---------------------------------------------------------------------------
// Session 7: relay packet-level logging (throttled) + counters
// ---------------------------------------------------------------------------

/// Log the first `RELAY_LOG_BURST` packets of a direction unconditionally so call
/// startup timing is visible, then at most `RELAY_LOG_PER_SEC` lines per second
/// so a ~50 pps audio stream cannot flood the log.
const RELAY_LOG_BURST: u32 = 20;
const RELAY_LOG_PER_SEC: u32 = 10;

struct PacketLogThrottle {
    seen: u32,
    window_start: Instant,
    in_window: u32,
}

impl PacketLogThrottle {
    fn new() -> Self {
        Self {
            seen: 0,
            window_start: Instant::now(),
            in_window: 0,
        }
    }

    fn allow(&mut self) -> bool {
        if self.seen < RELAY_LOG_BURST {
            self.seen += 1;
            return true;
        }
        let now = Instant::now();
        if now.duration_since(self.window_start) >= Duration::from_secs(1) {
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

/// Classify one relay datagram: `audio` | `control` | `stun` | `unknown`.
fn relay_packet_kind(buf: &[u8]) -> &'static str {
    if buf.len() >= 8 && buf[0..2] == [0x00, 0x01] && buf[4..8] == [0x21, 0x12, 0xa4, 0x42] {
        return "stun";
    }
    if buf.len() >= 21 {
        match buf[20] {
            RELAY_TAG_AUDIO => "audio",
            RELAY_TAG_CONTROL => "control",
            _ => "unknown",
        }
    } else {
        "unknown"
    }
}

/// One line per relay datagram, no batching.
fn log_relay_packet(
    direction: &'static str,
    room: &str,
    uuid_short: &str,
    addr: SocketAddr,
    len: usize,
    kind: &str,
    throttle: &mut PacketLogThrottle,
) {
    if !throttle.allow() {
        return;
    }
    if direction == "send" {
        tracing::debug!(
            direction,
            room,
            peer_uuid = uuid_short,
            dst = %addr,
            len,
            kind,
            "relay packet"
        );
    } else {
        tracing::debug!(
            direction,
            room,
            peer_uuid = uuid_short,
            src = %addr,
            len,
            kind,
            "relay packet"
        );
    }
}

fn bump_relay_sent(diag: &Arc<std::sync::RwLock<TransportDiagnostics>>, len: usize) {
    if let Ok(mut d) = diag.write() {
        d.packets_sent = d.packets_sent.saturating_add(1);
        d.bytes_sent = d.bytes_sent.saturating_add(len as u64);
        d.relay_packets_sent = d.relay_packets_sent.saturating_add(1);
        d.relay_bytes_sent = d.relay_bytes_sent.saturating_add(len as u64);
    }
}

fn bump_relay_recv(diag: &Arc<std::sync::RwLock<TransportDiagnostics>>, len: usize) {
    if let Ok(mut d) = diag.write() {
        d.packets_recv = d.packets_recv.saturating_add(1);
        d.bytes_recv = d.bytes_recv.saturating_add(len as u64);
        d.relay_packets_recv = d.relay_packets_recv.saturating_add(1);
        d.relay_bytes_recv = d.relay_bytes_recv.saturating_add(len as u64);
    }
}

/// Session 10: wrap a relay datagram in a STUN Binding-Request-shaped envelope.
///
/// Some ISP paths silently drop outbound UDP that is not a recognised protocol,
/// while passing anything behind a valid STUN header (type 0x0001 + magic cookie
/// 0x2112A442). Wrapping makes the relay survive such a path. `verio-server`
/// strips this envelope before parsing; the return direction needs no wrapper.
fn wrap_stun_envelope(inner: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(20 + inner.len());
    out.extend_from_slice(&0x0001u16.to_be_bytes()); // Binding Request
    out.extend_from_slice(&(inner.len() as u16).to_be_bytes()); // message length
    out.extend_from_slice(&[0x21, 0x12, 0xa4, 0x42]); // magic cookie
    out.extend_from_slice(&[0u8; 12]); // transaction id (unused by the server)
    out.extend_from_slice(inner);
    out
}

/// Cloneable handle to a running session's driver thread.
#[derive(Clone)]
pub struct PeerHandle {
    tx: Sender<PeerCommand>,
    diagnostics: Arc<std::sync::RwLock<TransportDiagnostics>>,
}

impl PeerHandle {
    pub fn send_audio(&self, capture_ts_ms: u64, opus: Vec<u8>) {
        let _ = self.tx.send(PeerCommand::Audio { capture_ts_ms, opus });
    }

    pub fn send_control(&self, msg: ControlMessage) {
        let _ = self.tx.send(PeerCommand::Control(msg));
    }

    pub fn disconnect(&self) {
        let _ = self.tx.send(PeerCommand::Disconnect);
    }

    pub fn diagnostics(&self) -> TransportDiagnostics {
        self.diagnostics.read().map(|g| g.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------------------
// Session setup (Rtc + UDP socket + candidates)
// ---------------------------------------------------------------------------

/// A half-built session: the `Rtc` and its UDP socket exist and have local
/// candidates, but no signaling has happened yet. The SDP exchange (via
/// verio-discovery) decides who offers.
pub struct SessionSetup {
    pub rtc: Rtc,
    pub socket: UdpSocket,
    pub base_addr: SocketAddr,
}

/// Checks whether an IP address is non-global (private, loopback, link-local, or CGNAT RFC 6598).
pub fn is_non_global_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let octets = v4.octets();
            // Loopback (127.0.0.0/8)
            octets[0] == 127
            // Private (10.0.0.0/8)
            || octets[0] == 10
            // Private (172.16.0.0/12)
            || (octets[0] == 172 && (16..=31).contains(&octets[1]))
            // Private (192.168.0.0/16)
            || (octets[0] == 192 && octets[1] == 168)
            // CGNAT RFC 6598 (100.64.0.0/10: 100.64.0.0 - 100.127.255.255)
            || (octets[0] == 100 && (64..=127).contains(&octets[1]))
            // Link-local (169.254.0.0/16)
            || (octets[0] == 169 && octets[1] == 254)
            // Unspecified (0.0.0.0)
            || octets == [0, 0, 0, 0]
        }
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// Attempts automatic UPnP / IGD port mapping on the local gateway router.
/// Maps `local_port` on the LAN IPv4 to `local_port` on the external WAN interface for 3600 seconds.
pub fn try_upnp_port_mapping(local_port: u16, lan_ip: std::net::Ipv4Addr) -> Option<SocketAddr> {
    let options = igd_next::SearchOptions {
        timeout: Some(Duration::from_millis(1500)),
        ..Default::default()
    };

    tracing::info!("UPnP: searching for IGD gateway router (timeout 1.5s)...");
    let gateway = match igd_next::search_gateway(options) {
        Ok(gw) => gw,
        Err(e) => {
            tracing::info!("UPnP: no gateway found or unsupported: {e}");
            return None;
        }
    };

    let ext_ip = match gateway.get_external_ip() {
        Ok(ip) => ip,
        Err(e) => {
            tracing::warn!("UPnP: failed to get external IP from gateway: {e}");
            return None;
        }
    };

    if is_non_global_ip(ext_ip) {
        tracing::warn!(external_ip = %ext_ip, "UPnP: gateway reported non-global/CGNAT address; discarding candidate");
        return None;
    }

    let local_socket = SocketAddr::V4(std::net::SocketAddrV4::new(lan_ip, local_port));
    match gateway.add_port(
        igd_next::PortMappingProtocol::UDP,
        local_port,
        local_socket,
        3600,
        "Verio WebRTC Voice",
    ) {
        Ok(()) => {
            let mapped_addr = SocketAddr::new(ext_ip, local_port);
            tracing::info!(mapped = %mapped_addr, "UPnP: successfully mapped external UDP port on router");
            Some(mapped_addr)
        }
        Err(e) => {
            tracing::warn!("UPnP: failed to add port mapping on gateway: {e}");
            None
        }
    }
}

impl SessionSetup {
    /// Bind the UDP socket and add local candidates. `internet_mode` adds a
    /// server-reflexive candidate via a minimal STUN binding query (host
    /// candidates alone can't cross NAT; no TURN in v1).
    pub fn new(internet_mode: bool) -> Result<Self, TransportError> {
        Self::with_custom_stuns(internet_mode, &[])
    }

    /// Extended setup accepting an optional private VPS STUN address for DPI-proof discovery.
    pub fn with_custom_stun(
        internet_mode: bool,
        custom_stun: Option<SocketAddr>,
    ) -> Result<Self, TransportError> {
        let stuns: Vec<SocketAddr> = custom_stun.into_iter().collect();
        Self::with_custom_stuns(internet_mode, &stuns)
    }

    /// Extended setup accepting a list of private VPS STUN addresses (e.g. port 3478 preferred, custom UDP fallback).
    pub fn with_custom_stuns(
        internet_mode: bool,
        custom_stuns: &[SocketAddr],
    ) -> Result<Self, TransportError> {
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        disable_connection_reset(&socket);
        let port = socket.local_addr()?.port();

        let mut rtc = Rtc::new(Instant::now());

        // Primary LAN candidate (cross-machine direct connect).
        let lan_addr = verio_discovery::primary_lan_addr(port);
        let base = match lan_addr {
            Some(lan) => {
                let lan_cand = Candidate::host(lan, "udp")
                    .map_err(|e| TransportError(format!("LAN candidate: {e}")))?;
                if rtc.add_local_candidate(lan_cand).is_none() {
                    tracing::warn!("LAN candidate rejected (duplicate?)");
                }
                lan
            }
            None => {
                // Fallback to loopback only if no LAN adapter was found
                let lo = SocketAddr::from(([127, 0, 0, 1], port));
                let lo_cand = Candidate::host(lo, "udp")
                    .map_err(|e| TransportError(format!("loopback candidate: {e}")))?;
                let _ = rtc.add_local_candidate(lo_cand);
                lo
            }
        };

        if internet_mode {
            // 1. Try automatic UPnP port mapping on home router (turns symmetric/restricted NAT into Full-Cone)
            if let Some(SocketAddr::V4(v4)) = lan_addr {
                if let Some(upnp_addr) = try_upnp_port_mapping(port, *v4.ip()) {
                    match Candidate::server_reflexive(upnp_addr, base, "udp") {
                        Ok(upnp_cand) => {
                            tracing::info!(upnp = %upnp_addr, "UPnP candidate added to WebRTC");
                            let _ = rtc.add_local_candidate(upnp_cand);
                        }
                        Err(e) => tracing::warn!("UPnP candidate creation failed: {e}"),
                    }
                }
            }

            // 2. Try private VPS STUN servers in order (e.g. 3478 VoIP port, then dynamic UDP port)
            let mut stun_discovered = false;
            for &stun_addr in custom_stuns {
                let target = stun_addr.to_string();
                tracing::info!(%target, "querying private VPS STUN server");
                match stun_binding_query(&socket, &target) {
                    Ok(srflx) => {
                        tracing::info!(srflx = %srflx, target = %target, "private VPS STUN succeeded");
                        match Candidate::server_reflexive(srflx, base, "udp") {
                            Ok(cand) => {
                                let _ = rtc.add_local_candidate(cand);
                                stun_discovered = true;
                                break;
                            }
                            Err(e) => tracing::warn!("srflx candidate failed: {e}"),
                        }
                    }
                    Err(e) => tracing::warn!(target = %target, "private VPS STUN query failed: {e}"),
                }
            }

            // 3. Fallback to public STUN servers if VPS STUN was not used or failed
            if !stun_discovered {
                for server in STUN_SERVERS {
                    match stun_binding_query(&socket, server) {
                        Ok(srflx) => {
                            tracing::info!(server, srflx = %srflx, "public STUN server-reflexive candidate");
                            match Candidate::server_reflexive(srflx, base, "udp") {
                                Ok(srflx_cand) => {
                                    let _ = rtc.add_local_candidate(srflx_cand);
                                }
                                Err(e) => tracing::warn!("srflx candidate creation failed: {e}"),
                            }
                            break;
                        }
                        Err(e) => tracing::warn!(server, "STUN binding failed: {e}"),
                    }
                }
            }
        }

        Ok(Self { rtc, socket, base_addr: base })
    }

    /// OFFERER ONLY: declare the audio (unreliable) and control (reliable)
    /// channels and produce the offer (JSON-serialized str0m SdpOffer — the
    /// string form is what travels inside direct frames and invite blobs), plus
    /// the pending handle needed for [`Self::accept_answer`]. str0m is
    /// non-trickle: candidates are already gathered and ride inside the offer,
    /// which is exactly what invite blobs need.
    pub fn make_offer(&mut self) -> Result<(String, SdpPendingOffer), TransportError> {
        let audio = ChannelConfig {
            label: AUDIO_CHANNEL_LABEL.to_string(),
            ordered: false,
            reliability: Reliability::MaxRetransmits { retransmits: 0 },
            ..ChannelConfig::default()
        };
        let control = ChannelConfig {
            label: CONTROL_CHANNEL_LABEL.to_string(),
            ..ChannelConfig::default()
        };
        let mut sdp = self.rtc.sdp_api();
        sdp.add_channel_with_config(audio);
        sdp.add_channel_with_config(control);
        let (offer, pending) = sdp
            .apply()
            .ok_or_else(|| TransportError("no pending offer after apply".into()))?;
        let json = serde_json::to_string(&offer)
            .map_err(|e| TransportError(format!("serialize offer: {e}")))?;
        Ok((json, pending))
    }

    /// OFFERER ONLY: apply the remote answer (JSON string as returned by the
    /// signaling layer).
    pub fn accept_answer(
        &mut self,
        pending: SdpPendingOffer,
        answer_json: &str,
    ) -> Result<(), TransportError> {
        let answer: SdpAnswer = serde_json::from_str(answer_json)
            .map_err(|e| TransportError(format!("parse answer: {e}")))?;
        self.rtc.sdp_api().accept_answer(pending, answer)?;
        Ok(())
    }

    /// ANSWERER ONLY: apply the remote offer, produce the answer JSON string.
    pub fn accept_offer(&mut self, offer_json: &str) -> Result<String, TransportError> {
        let offer: SdpOffer = serde_json::from_str(offer_json)
            .map_err(|e| TransportError(format!("parse offer: {e}")))?;
        let answer = self.rtc.sdp_api().accept_offer(offer)?;
        let json = serde_json::to_string(&answer)
            .map_err(|e| TransportError(format!("serialize answer: {e}")))?;
        Ok(json)
    }
}

// ---------------------------------------------------------------------------
// Minimal STUN client (RFC 5389 binding request — protocol, not crypto)
// ---------------------------------------------------------------------------

/// Hand-rolled STUN binding request. Justification: str0m does not perform STUN
/// gathering itself, and a STUN crate for one request/response would violate the
/// dependency allowance. ~60 lines, no crypto involved.
fn stun_binding_query(socket: &UdpSocket, server: &str) -> Result<SocketAddr, TransportError> {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    let addr: SocketAddr = server
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| TransportError(format!("STUN server {server:?} does not resolve")))?;

    // 96-bit transaction id from the OS-random hasher seed.
    let mut h = RandomState::new().build_hasher();
    let mut txid = [0u8; 12];
    for chunk in txid.chunks_mut(8) {
        h.write_u64(unix_ms());
        let v = h.finish();
        for (i, b) in chunk.iter_mut().enumerate() {
            *b = (v >> (i * 8)) as u8;
        }
    }

    const RFC5389_MAGIC: u32 = 0x2112A442;

    // Binding request: type 0x0001, length 0, magic cookie (0x2112A442), txid.
    let mut req = Vec::with_capacity(20);
    req.extend_from_slice(&0x0001u16.to_be_bytes());
    req.extend_from_slice(&0u16.to_be_bytes());
    req.extend_from_slice(&RFC5389_MAGIC.to_be_bytes());
    req.extend_from_slice(&txid);
    socket.send_to(&req, addr)?;

    let mut buf = [0u8; 256];
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(TransportError(format!("STUN {server}: timeout")));
        }
        socket.set_read_timeout(Some(remaining))?;
        let (n, from) = match socket.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionReset
                ) || matches!(e.raw_os_error(), Some(10054 | 10051 | 10065 | 10060 | 10035)) =>
            {
                continue;
            }
            Err(e) => return Err(e.into()),
        };
        if from != addr || n < 20 {
            continue;
        }
        // Success response: type 0x0101, txid must match.
        if buf[..2] != [0x01, 0x01] || buf[8..20] != txid {
            continue;
        }
        // Walk attributes looking for XOR-MAPPED-ADDRESS (0x0020) or MAPPED-ADDRESS (0x0001).
        let mut i = 20;
        while i + 4 <= n {
            let attr = u16::from_be_bytes([buf[i], buf[i + 1]]);
            let len = u16::from_be_bytes([buf[i + 2], buf[i + 3]]) as usize;
            if attr == 0x0020 && len >= 8 && i + 4 + len <= n {
                let port = u16::from_be_bytes([buf[i + 6], buf[i + 7]]) ^ (RFC5389_MAGIC >> 16) as u16;
                let xor_ip = u32::from_be_bytes([buf[i + 8], buf[i + 9], buf[i + 10], buf[i + 11]]);
                let ip = std::net::Ipv4Addr::from(xor_ip ^ RFC5389_MAGIC);
                return Ok(SocketAddr::from((ip, port)));
            } else if attr == 0x0001 && len >= 8 && i + 4 + len <= n {
                let port = u16::from_be_bytes([buf[i + 6], buf[i + 7]]);
                let ip = std::net::Ipv4Addr::new(buf[i + 8], buf[i + 9], buf[i + 10], buf[i + 11]);
                return Ok(SocketAddr::from((ip, port)));
            }
            i += 4 + len.next_multiple_of(4);
        }
    }
}

// ---------------------------------------------------------------------------
// Driver thread
// ---------------------------------------------------------------------------

/// Take over a completed session on a dedicated driver thread. The driver owns
/// the `Rtc` and socket; other threads talk to it exclusively via the returned
/// [`PeerHandle`] (lock-free mpsc) and observe it via `events`.
pub fn spawn_session(
    setup: SessionSetup,
    identity: verio_discovery::Identity,
    silence_frame: Arc<Vec<u8>>,
    events: Sender<TransportEvent>,
) -> Result<PeerHandle, TransportError> {
    let (cmd_tx, cmd_rx) = mpsc::channel::<PeerCommand>();
    let SessionSetup { rtc, socket, base_addr } = setup;

    let local_addr = socket.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| "0.0.0.0".into());
    let diagnostics = Arc::new(std::sync::RwLock::new(TransportDiagnostics {
        mode: "direct_p2p".into(),
        endpoint: base_addr.to_string(),
        local_addr,
        status: "Connecting".into(),
        ..Default::default()
    }));
    let diag_clone = Arc::clone(&diagnostics);

    std::thread::Builder::new()
        .name("verio-transport".into())
        .spawn(move || {
            let mut driver = Driver {
                rtc,
                socket,
                base_addr,
                identity,
                silence_frame,
                events,
                cmds: cmd_rx,
                audio_cid: None,
                control_cid: None,
                connected: false,
                seq: 0,
                last_audio_send: None,
                last_ping: None,
                outbox: Vec::new(),
                disconnecting: false,
                disconnected_since: None,
                diagnostics: diag_clone,
                last_recv_instant: None,
            };
            let reason = driver.run();
            tracing::info!(%reason, "transport driver exited");
            let _ = driver.events.send(TransportEvent::Disconnected { reason });
        })
        .map_err(|e| TransportError(format!("spawn driver thread: {e}")))?;

    Ok(PeerHandle {
        tx: cmd_tx,
        diagnostics,
    })
}

struct Driver {
    rtc: Rtc,
    socket: UdpSocket,
    base_addr: SocketAddr,
    identity: verio_discovery::Identity,
    /// Cached Opus silence frame (encoded once at startup) for the 400 ms
    /// audio-path keep-alive.
    silence_frame: Arc<Vec<u8>>,
    events: Sender<TransportEvent>,
    cmds: Receiver<PeerCommand>,
    audio_cid: Option<ChannelId>,
    control_cid: Option<ChannelId>,
    connected: bool,
    seq: u32,
    last_audio_send: Option<Instant>,
    last_ping: Option<Instant>,
    /// Control messages waiting for the reliable channel to accept them again.
    outbox: Vec<ControlMessage>,
    disconnecting: bool,
    disconnected_since: Option<Instant>,
    diagnostics: Arc<std::sync::RwLock<TransportDiagnostics>>,
    last_recv_instant: Option<Instant>,
}

impl Driver {
    /// The canonical str0m loop: one mutation, then drain `poll_output` until
    /// `Output::Timeout`, then wait for socket input / the deadline.
    fn run(&mut self) -> String {
        let mut buf = Vec::new();
        loop {
            if let Err(reason) = self.apply_commands() {
                return reason;
            }
            let str0m_wait = match self.drain_output() {
                Ok(w) => w,
                Err(reason) => return reason,
            };
            if self.disconnecting {
                return "disconnected".into();
            }
            if let Err(reason) = self.periodic_sends() {
                return reason;
            }
            let str0m_wait2 = match self.drain_output() {
                Ok(w) => w,
                Err(reason) => return reason,
            };
            if self.disconnecting {
                return "disconnected".into();
            }

            // Check if ICE disconnected grace period expired (30 seconds)
            if let Some(since) = self.disconnected_since {
                if since.elapsed() >= Duration::from_secs(30) {
                    tracing::error!("ICE disconnected grace period expired (30s)");
                    self.rtc.disconnect();
                    self.disconnecting = true;
                    return "ice connection lost (grace period expired)".into();
                }
            }

            let now = Instant::now();
            // str0m's requested timeout (pacing/SACK/DTLS timers) combined with
            // our own keep-alive/ping schedule — honoring it keeps one-way
            // latency free of up-to-250 ms artificial driver delay.
            let wait = self
                .wait_duration(now)
                .min(str0m_wait)
                .min(str0m_wait2);
            if wait.is_zero() {
                if let Err(e) = self.rtc.handle_input(Input::Timeout(Instant::now())) {
                    tracing::debug!("str0m timeout handling error: {e}");
                }
                continue;
            }

            // Cap the blocking read at 10 ms: the audio pump hands frames to the
            // cmd channel asynchronously, and without this cap the driver would
            // only pick them up when the next network packet happens to arrive.
            let sock_wait = wait.min(Duration::from_millis(10));
            if let Err(e) = self.socket.set_read_timeout(Some(sock_wait)) {
                return format!("socket: {e}");
            }
            buf.resize(BUF_SIZE, 0);
            let input = match self.socket.recv_from(&mut buf) {
                Ok((n, source)) => {
                    buf.truncate(n);
                    self.last_recv_instant = Some(Instant::now());
                    if let Ok(mut d) = self.diagnostics.write() {
                        d.packets_recv = d.packets_recv.saturating_add(1);
                        d.bytes_recv = d.bytes_recv.saturating_add(n as u64);
                        d.last_recv_ms_ago = Some(0);
                        d.endpoint = source.to_string();
                    }
                    let port = match self.socket.local_addr() {
                        Ok(a) => a.port(),
                        Err(e) => return format!("socket: {e}"),
                    };
                    let destination = if source.ip().is_loopback() {
                        SocketAddr::from(([127, 0, 0, 1], port))
                    } else {
                        self.base_addr
                    };
                    let contents = match buf.as_slice().try_into() {
                        Ok(c) => c,
                        Err(_) => {
                            tracing::debug!(len = n, "ignoring unclassifiable or empty UDP datagram");
                            continue;
                        }
                    };
                    Input::Receive(
                        Instant::now(),
                        Receive {
                            proto: Protocol::Udp,
                            source,
                            destination,
                            contents,
                        },
                    )
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::ConnectionReset
                    ) || matches!(e.raw_os_error(), Some(10054 | 10051 | 10065 | 10060 | 10035)) =>
                {
                    Input::Timeout(Instant::now())
                }
                Err(e) => return format!("socket: {e}"),
            };
            if let Err(e) = self.rtc.handle_input(input) {
                tracing::debug!("str0m input handling non-fatal error: {e}");
            }
        }
    }

    /// Consume commands from the app side and apply them as mutations.
    fn apply_commands(&mut self) -> Result<(), String> {
        while let Ok(cmd) = self.cmds.try_recv() {
            match cmd {
                PeerCommand::Audio { capture_ts_ms, opus } => {
                    tracing::debug!(
                        delay_ms = unix_ms().saturating_sub(capture_ts_ms),
                        "audio cmd picked up (feed→driver delay)"
                    );
                    self.send_audio_frame(&opus, capture_ts_ms);
                }
                PeerCommand::Control(msg) => {
                    self.queue_control(msg);
                }
                PeerCommand::Disconnect => {
                    if !self.disconnecting {
                        tracing::info!("disconnect requested");
                        self.rtc.disconnect();
                        self.disconnecting = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// Send one audio frame on the unreliable channel (best-effort: a rejected
    /// write means the SCTP buffer is full — drop the frame, latency beats loss).
    fn send_audio_frame(&mut self, opus: &[u8], capture_ts_ms: u64) {
        self.seq = self.seq.wrapping_add(1);
        let Some(cid) = self.audio_cid else { return };
        let packet = encode_audio_packet(self.seq, capture_ts_ms, opus);
        if let Some(mut ch) = self.rtc.channel(cid) {
            match ch.write(true, &packet) {
                Ok(true) => self.last_audio_send = Some(Instant::now()),
                Ok(false) => {
                    // Buffer full — dropped on purpose (keep latency bounded).
                }
                Err(e) => tracing::warn!("audio write failed: {e}"),
            }
        }
    }

    /// Queue a control message (reliable channel; retried from the outbox when
    /// the buffer was full).
    fn queue_control(&mut self, msg: ControlMessage) {
        self.outbox.push(msg);
        self.flush_outbox();
    }

    fn flush_outbox(&mut self) {
        let Some(cid) = self.control_cid else { return };
        let mut still_pending = Vec::new();
        for msg in self.outbox.drain(..) {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!("control serialize: {e}");
                    continue;
                }
            };
            let accepted = match self.rtc.channel(cid) {
                Some(mut ch) => ch.write(false, json.as_bytes()).unwrap_or(false),
                None => false,
            };
            if !accepted {
                still_pending.push(msg);
            }
        }
        self.outbox = still_pending;
    }

    /// Keep-alives: 400 ms Opus silence on the audio path, 2 s ping on control.
    fn periodic_sends(&mut self) -> Result<(), String> {
        if !self.connected {
            return Ok(());
        }
        if let Some(cid) = self.audio_cid {
            let due = match self.last_audio_send {
                None => true,
                Some(t) => t.elapsed() >= KEEPALIVE_INTERVAL,
            };
            if due {
                // Spec: 2-byte Opus silence frame keeps the decoder warm (and,
                // in the internet test, the NAT mapping alive).
                let ts = unix_ms();
                let frame = Arc::clone(&self.silence_frame);
                self.seq = self.seq.wrapping_add(1);
                let packet = encode_audio_packet(self.seq, ts, &frame);
                if let Some(mut ch) = self.rtc.channel(cid) {
                    match ch.write(true, &packet) {
                        Ok(true) => self.last_audio_send = Some(Instant::now()),
                        Ok(false) => {}
                        Err(e) => tracing::warn!("keep-alive write failed: {e}"),
                    }
                }
            }
        }
        let ping_due = match self.last_ping {
            None => true,
            Some(t) => t.elapsed() >= PING_INTERVAL,
        };
        if ping_due {
            self.last_ping = Some(Instant::now());
            self.queue_control(ControlMessage::Ping { t: unix_ms() });
        }
        Ok(())
    }

    /// How long to block on the socket: the soonest of the keep-alive and the
    /// next ping (never longer than 250 ms — str0m's own timeouts are honored
    /// via `Input::Timeout` on the next loop pass).
    fn wait_duration(&self, now: Instant) -> Duration {
        let mut wait = Duration::from_millis(250);
        if let Some(t) = self.last_audio_send {
            let since = now.saturating_duration_since(t);
            wait = wait.min(KEEPALIVE_INTERVAL.saturating_sub(since));
        }
        if let Some(t) = self.last_ping {
            let since = now.saturating_duration_since(t);
            wait = wait.min(PING_INTERVAL.saturating_sub(since));
        }
        wait
    }

    /// Drain all pending output; returns on the next timeout marker.
    /// Drain all pending output; returns how long str0m wants before its next
    /// `Input::Timeout` (capped at 250 ms so keep-alives/pings stay punctual).
    /// The caller combines this with its own keep-alive/ping schedule —
    /// ignoring str0m's timeout adds up-to-250 ms artificial latency because
    /// paced transmissions, SACK timers and DTLS retransmits all fire late.
    fn drain_output(&mut self) -> Result<Duration, String> {
        loop {
            match self.rtc.poll_output() {
                Ok(Output::Timeout(t)) => {
                    let now = Instant::now();
                    return Ok(t.saturating_duration_since(now).min(Duration::from_millis(250)));
                }
                Ok(Output::Transmit(t)) => {
                    if let Ok(mut d) = self.diagnostics.write() {
                        d.packets_sent = d.packets_sent.saturating_add(1);
                        d.bytes_sent = d.bytes_sent.saturating_add(t.contents.len() as u64);
                    }
                    if let Err(e) = self.socket.send_to(&t.contents, t.destination) {
                        // Transient ICMP/port errors are survivable.
                        tracing::debug!("udp send_to failed: {e}");
                    }
                }
                Ok(Output::Event(e)) => {
                    self.handle_event(e)?;
                    if self.disconnecting {
                        return Ok(Duration::from_millis(250));
                    }
                }
                Err(e) => return Err(format!("str0m: {e}")),
            }
        }
    }

    fn handle_event(&mut self, event: Event) -> Result<(), String> {
        match event {
            Event::Connected if !self.connected => {
                tracing::info!("peer connection established (ICE + DTLS)");
                self.connected = true;
                self.disconnected_since = None;
                if let Ok(mut d) = self.diagnostics.write() {
                    d.status = "Connected".into();
                }
                let _ = self.events.send(TransportEvent::Connected);
            }
            Event::Connected => {
                self.disconnected_since = None;
                if let Ok(mut d) = self.diagnostics.write() {
                    d.status = "Connected".into();
                }
            }
            Event::IceConnectionStateChange(s) => {
                if let Ok(mut d) = self.diagnostics.write() {
                    d.ice_state = Some(format!("{s:?}"));
                }
                match s {
                    IceConnectionState::Connected | IceConnectionState::Completed => {
                        tracing::info!("ICE connection state: {s:?}");
                        self.disconnected_since = None;
                    }
                    IceConnectionState::Disconnected => {
                        tracing::warn!("ICE connection entered Disconnected state (transient, grace period active)");
                        if self.disconnected_since.is_none() {
                            self.disconnected_since = Some(Instant::now());
                        }
                    }
                    _ => {}
                }
            }
            Event::Closed => {
                self.disconnecting = true;
                return Err("session closed".into());
            }
            Event::ChannelOpen(cid, label) => {
                match label.as_str() {
                    AUDIO_CHANNEL_LABEL => {
                        tracing::info!(?cid, "audio channel open (unreliable)");
                        self.audio_cid = Some(cid);
                    }
                    CONTROL_CHANNEL_LABEL => {
                        tracing::info!(?cid, "control channel open (reliable)");
                        self.control_cid = Some(cid);
                        // Spec: hello{name,version} on the control channel.
                        self.queue_control(ControlMessage::Hello {
                            name: self.identity.name.clone(),
                            version: self.identity.version.clone(),
                        });
                    }
                    other => tracing::debug!(label = other, "unexpected channel open"),
                }
            }
            Event::ChannelClose(cid)
                if Some(cid) == self.audio_cid || Some(cid) == self.control_cid =>
            {
                self.rtc.disconnect();
                self.disconnecting = true;
                return Err("data channel closed by remote".into());
            }
            Event::ChannelClose(_) => {}
            Event::ChannelData(data) => self.handle_channel_data(&data),
            _ => {}
        }
        Ok(())
    }

    fn handle_channel_data(&mut self, data: &ChannelData) {
        if Some(data.id) == self.audio_cid {
            match parse_audio_packet(&data.data) {
                Ok((seq, capture_ts_ms, opus)) => {
                    tracing::debug!(
                        seq,
                        delay_ms = unix_ms().saturating_sub(capture_ts_ms),
                        "audio received (capture→driver delay)"
                    );
                    let _ = self.events.send(TransportEvent::Audio {
                        peer: String::new(),
                        seq,
                        capture_ts_ms,
                        opus: opus.to_vec(),
                    });
                }
                Err(e) => tracing::warn!("bad audio packet: {e}"),
            }
        } else if Some(data.id) == self.control_cid {
            let text = String::from_utf8_lossy(&data.data);
            match serde_json::from_str::<ControlMessage>(&text) {
                Ok(ControlMessage::Ping { t }) => {
                    self.queue_control(ControlMessage::Pong { t });
                }
                Ok(ControlMessage::Pong { t }) => {
                    let ms = unix_ms().saturating_sub(t) as f64;
                    if let Ok(mut d) = self.diagnostics.write() {
                        d.rtt_ms = Some(ms);
                    }
                    let _ = self.events.send(TransportEvent::Rtt { ms });
                }
                Ok(msg) => {
                    let _ = self.events.send(TransportEvent::Control {
                        peer: String::new(),
                        message: msg,
                    });
                }
                Err(e) => tracing::warn!("bad control message: {e}"),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Embedded Voice Relay Host (Mode 2: "Host on My PC")
// ---------------------------------------------------------------------------

/// Embedded UDP voice relay host for Mode 2 ("Host on My PC"):
/// Runs a lightweight UDP voice switchboard directly inside the room creator's client.
/// Binds a UDP socket on a configurable or OS-assigned ephemeral port.
/// Listens for datagrams from peers and fans them out to all other peers in the room.
pub struct EmbeddedVoiceHost {
    local_port: u16,
    stop_tx: std::sync::mpsc::Sender<()>,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl EmbeddedVoiceHost {
    pub fn start(port: u16) -> Result<Self, TransportError> {
        let socket = UdpSocket::bind(format!("0.0.0.0:{port}"))
            .map_err(|e| TransportError(format!("bind embedded host socket on port {port}: {e}")))?;
        disable_connection_reset(&socket);
        let local_port = socket.local_addr()?.port();
        socket.set_read_timeout(Some(Duration::from_millis(50)))?;

        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let join_handle = std::thread::Builder::new()
            .name("verio-voice-host".into())
            .spawn(move || {
                tracing::info!(local_port, "embedded voice host started");
                let mut buf = [0u8; 4096];
                let mut rooms: std::collections::HashMap<String, std::collections::HashMap<[u8; 16], (SocketAddr, Instant)>> = std::collections::HashMap::new();

                while stop_rx.try_recv().is_err() {
                    let (n, from) = match socket.recv_from(&mut buf) {
                        Ok(res) => res,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                            continue;
                        }
                        Err(_) => continue,
                    };

                    // RFC 5389 STUN Binding Request responder
                    if n == 20 && buf[0..2] == [0x00, 0x01] && buf[4..8] == [0x21, 0x12, 0xa4, 0x42] {
                        let mut resp = Vec::with_capacity(32);
                        resp.extend_from_slice(&[0x01, 0x01]);
                        resp.extend_from_slice(&12u16.to_be_bytes());
                        resp.extend_from_slice(&[0x21, 0x12, 0xa4, 0x42]);
                        resp.extend_from_slice(&buf[8..20]);
                        resp.extend_from_slice(&0x0020u16.to_be_bytes());
                        resp.extend_from_slice(&8u16.to_be_bytes());
                        resp.push(0x00);
                        resp.push(0x01);
                        let port_xor = from.port() ^ 0x2112;
                        resp.extend_from_slice(&port_xor.to_be_bytes());
                        match from.ip() {
                            std::net::IpAddr::V4(ipv4) => {
                                let ip_u32 = u32::from(ipv4) ^ 0x2112A442;
                                resp.extend_from_slice(&ip_u32.to_be_bytes());
                            }
                            std::net::IpAddr::V6(_) => continue,
                        }
                        let _ = socket.send_to(&resp, from);
                        continue;
                    }

                    if n < 20 {
                        continue;
                    }

                    // Session 10: clients frame relay datagrams in a STUN-shaped
                    // envelope (see wrap_stun_envelope); strip it before parsing.
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

                    let now = Instant::now();
                    let room_map = rooms.entry(room_code).or_default();
                    room_map.insert(sender_uuid, (from, now));

                    // Prune peers inactive > 60s
                    room_map.retain(|_, (_, last_seen)| now.duration_since(*last_seen) < Duration::from_secs(60));

                    // Fan-out to all other peers in the room (inner payload)
                    if payload.len() > 20 {
                        for (&uuid, &(target, _)) in room_map.iter() {
                            if uuid != sender_uuid && target != from {
                                let _ = socket.send_to(payload, target);
                            }
                        }
                    }
                }
                tracing::info!("embedded voice host stopped");
            })
            .map_err(|e| TransportError(format!("spawn embedded host thread: {e}")))?;

        Ok(Self {
            local_port,
            stop_tx,
            join_handle: Some(join_handle),
        })
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    pub fn stop(&mut self) {
        let _ = self.stop_tx.send(());
        if let Some(h) = self.join_handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for EmbeddedVoiceHost {
    fn drop(&mut self) {
        self.stop();
    }
}

// ---------------------------------------------------------------------------
// Fallback UDP Relay Session
// ---------------------------------------------------------------------------

pub const RELAY_TAG_HEARTBEAT: u8 = 0x00;
pub const RELAY_TAG_AUDIO: u8 = 0x01;
pub const RELAY_TAG_CONTROL: u8 = 0x02;

pub fn format_room_code_bytes(code: &str) -> [u8; 4] {
    let mut b = [b'0'; 4];
    let bytes = code.as_bytes();
    let len = bytes.len().min(4);
    b[4 - len..].copy_from_slice(&bytes[..len]);
    b
}

/// Spawn a voice session over the VPS fallback UDP relay.
pub fn spawn_relay_session(
    relay_addr: SocketAddr,
    room_code: String,
    local_identity: verio_discovery::Identity,
    remote_identity: verio_discovery::Identity,
    silence_frame: Arc<Vec<u8>>,
    events: Sender<TransportEvent>,
) -> Result<PeerHandle, TransportError> {
    spawn_relay_session_with_mode(
        relay_addr,
        room_code,
        local_identity,
        remote_identity,
        silence_frame,
        events,
        "cloud_relay",
    )
}

/// Spawn a voice session over a UDP relay or peer-host with a specified mode label.
pub fn spawn_relay_session_with_mode(
    relay_addr: SocketAddr,
    room_code: String,
    local_identity: verio_discovery::Identity,
    remote_identity: verio_discovery::Identity,
    silence_frame: Arc<Vec<u8>>,
    events: Sender<TransportEvent>,
    mode: &'static str,
) -> Result<PeerHandle, TransportError> {
    spawn_relay_session_with_candidates(
        relay_addr,
        Vec::new(),
        room_code,
        local_identity,
        remote_identity,
        silence_frame,
        events,
        mode,
    )
}

/// Spawn a voice session over a UDP relay or peer-host with candidate endpoints (probes all until response, then locks on).
pub fn spawn_relay_session_with_candidates(
    primary_addr: SocketAddr,
    fallback_addrs: Vec<SocketAddr>,
    room_code: String,
    local_identity: verio_discovery::Identity,
    _remote_identity: verio_discovery::Identity,
    _silence_frame: Arc<Vec<u8>>,
    events: Sender<TransportEvent>,
    mode: &'static str,
) -> Result<PeerHandle, TransportError> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    disable_connection_reset(&socket);
    socket.set_read_timeout(Some(Duration::from_millis(10)))?;

    let mut candidate_addrs = vec![primary_addr];
    for a in fallback_addrs {
        if !candidate_addrs.contains(&a) {
            candidate_addrs.push(a);
        }
    }

    let local_addr = socket.local_addr().map(|a| a.to_string()).unwrap_or_else(|_| "0.0.0.0".into());
    let diagnostics = Arc::new(std::sync::RwLock::new(TransportDiagnostics {
        mode: mode.into(),
        endpoint: primary_addr.to_string(),
        local_addr,
        status: "Connected".into(),
        relay_mode: if mode == "forced_relay" {
            "forced".into()
        } else {
            "auto-upgraded".into()
        },
        ..Default::default()
    }));
    let diag_clone = Arc::clone(&diagnostics);

    let room_bytes = format_room_code_bytes(&room_code);
    let local_uuid_bytes = *local_identity.uuid.as_bytes();
    // Session 7 instrumentation: packet-log context + per-direction throttles.
    let local_uuid_short: String = local_identity.uuid.to_string().chars().take(8).collect();
    let room_log = room_code.clone();
    let mut tx_log = PacketLogThrottle::new();
    let mut rx_log = PacketLogThrottle::new();

    let mut prefix = [0u8; 20];
    prefix[0..4].copy_from_slice(&room_bytes);
    prefix[4..20].copy_from_slice(&local_uuid_bytes);

    // Send initial registration packet to all candidate relay endpoints.
    // Session 10: every relay datagram is framed in a STUN-shaped envelope.
    let framed_prefix = wrap_stun_envelope(&prefix);
    for target in &candidate_addrs {
        let _ = socket.send_to(&framed_prefix, *target);
        log_relay_packet(
            "send",
            &room_log,
            &local_uuid_short,
            *target,
            framed_prefix.len(),
            "control",
            &mut tx_log,
        );
        bump_relay_sent(&diag_clone, framed_prefix.len());
    }

    // Send initial Hello control message
    let hello = ControlMessage::Hello {
        name: local_identity.name.clone(),
        version: local_identity.version.clone(),
    };
    if let Ok(json) = serde_json::to_string(&hello) {
        let mut pkt = Vec::with_capacity(21 + json.len());
        pkt.extend_from_slice(&prefix);
        pkt.push(RELAY_TAG_CONTROL);
        pkt.extend_from_slice(json.as_bytes());
        let framed = wrap_stun_envelope(&pkt);
        for target in &candidate_addrs {
            let _ = socket.send_to(&framed, *target);
            log_relay_packet(
                "send",
                &room_log,
                &local_uuid_short,
                *target,
                framed.len(),
                "control",
                &mut tx_log,
            );
            bump_relay_sent(&diag_clone, framed.len());
        }
    }

    // Immediately signal Connected
    let _ = events.send(TransportEvent::Connected);

    let (cmd_tx, cmd_rx) = mpsc::channel::<PeerCommand>();

    std::thread::Builder::new()
        .name("verio-relay-driver".into())
        .spawn(move || {
            let mut seq: u32 = 0;
            let mut last_heartbeat = Instant::now();
            let mut last_ping = Instant::now();
            // Session 12: with several peers in the room every peer echoes our
            // ping, so remember which ping is ours and ignore the rest.
            let mut last_ping_t: u64 = 0;
            let mut last_recv_time: Option<Instant> = None;
            let mut active_relay_addr = primary_addr;
            let mut recv_buf = [0u8; 2048];

            let reason = loop {
                // 1. Drain commands from audio pump / app
                let mut disconnect = false;
                while let Ok(cmd) = cmd_rx.try_recv() {
                    match cmd {
                        PeerCommand::Audio { capture_ts_ms, opus } => {
                            seq = seq.wrapping_add(1);
                            let audio_data = encode_audio_packet(seq, capture_ts_ms, &opus);
                            let mut pkt = Vec::with_capacity(21 + audio_data.len());
                            pkt.extend_from_slice(&prefix);
                            pkt.push(RELAY_TAG_AUDIO);
                            pkt.extend_from_slice(&audio_data);
                            let framed = wrap_stun_envelope(&pkt);
                            let _ = socket.send_to(&framed, active_relay_addr);
                            log_relay_packet(
                                "send",
                                &room_log,
                                &local_uuid_short,
                                active_relay_addr,
                                framed.len(),
                                "audio",
                                &mut tx_log,
                            );
                            bump_relay_sent(&diag_clone, framed.len());
                        }
                        PeerCommand::Control(msg) => {
                            if let Ok(json) = serde_json::to_string(&msg) {
                                let mut pkt = Vec::with_capacity(21 + json.len());
                                pkt.extend_from_slice(&prefix);
                                pkt.push(RELAY_TAG_CONTROL);
                                pkt.extend_from_slice(json.as_bytes());
                                let framed = wrap_stun_envelope(&pkt);
                                let _ = socket.send_to(&framed, active_relay_addr);
                                log_relay_packet(
                                    "send",
                                    &room_log,
                                    &local_uuid_short,
                                    active_relay_addr,
                                    framed.len(),
                                    "control",
                                    &mut tx_log,
                                );
                                bump_relay_sent(&diag_clone, framed.len());
                            }
                        }
                        PeerCommand::Disconnect => {
                            disconnect = true;
                            break;
                        }
                    }
                }
                if disconnect {
                    break "user disconnect".to_string();
                }

                // 2. Periodic timers
                let now = Instant::now();
                // 1s heartbeat keeps NAT open
                if now.duration_since(last_heartbeat) >= Duration::from_secs(1) {
                    if last_recv_time.is_none() {
                        // Probe all candidates until inbound traffic locks onto one
                        for target in &candidate_addrs {
                            let _ = socket.send_to(&framed_prefix, *target);
                            log_relay_packet(
                                "send",
                                &room_log,
                                &local_uuid_short,
                                *target,
                                framed_prefix.len(),
                                "control",
                                &mut tx_log,
                            );
                            bump_relay_sent(&diag_clone, framed_prefix.len());
                        }
                    } else {
                        let _ = socket.send_to(&framed_prefix, active_relay_addr);
                        log_relay_packet(
                            "send",
                            &room_log,
                            &local_uuid_short,
                            active_relay_addr,
                            framed_prefix.len(),
                            "control",
                            &mut tx_log,
                        );
                        bump_relay_sent(&diag_clone, framed_prefix.len());
                    }
                    last_heartbeat = now;
                }
                // 2s ping for RTT measurement
                if now.duration_since(last_ping) >= PING_INTERVAL {
                    let ping_t = unix_ms();
                    last_ping_t = ping_t;
                    let ping = ControlMessage::Ping { t: ping_t };
                    if let Ok(json) = serde_json::to_string(&ping) {
                        let mut pkt = Vec::with_capacity(21 + json.len());
                        pkt.extend_from_slice(&prefix);
                        pkt.push(RELAY_TAG_CONTROL);
                        pkt.extend_from_slice(json.as_bytes());
                        let framed = wrap_stun_envelope(&pkt);
                        let _ = socket.send_to(&framed, active_relay_addr);
                        log_relay_packet(
                            "send",
                            &room_log,
                            &local_uuid_short,
                            active_relay_addr,
                            framed.len(),
                            "control",
                            &mut tx_log,
                        );
                        bump_relay_sent(&diag_clone, framed.len());
                    }
                    last_ping = now;
                }

                if let Some(recv_time) = last_recv_time {
                    if let Ok(mut d) = diag_clone.write() {
                        d.last_recv_ms_ago = Some(recv_time.elapsed().as_millis() as u64);
                    }
                }

                // 3. Receive packets from UDP relay
                let mut process_packet = |n: usize, from: SocketAddr, buf: &[u8]| {
                    log_relay_packet(
                        "recv",
                        &room_log,
                        &local_uuid_short,
                        from,
                        n,
                        relay_packet_kind(&buf[..n]),
                        &mut rx_log,
                    );
                    let is_candidate = candidate_addrs.iter().any(|c| c.ip() == from.ip());
                    if !is_candidate || n < 20 {
                        return;
                    }
                    // Check room code
                    if &buf[0..4] != &room_bytes {
                        return;
                    }
                    // Ignore our own echo
                    if &buf[4..20] == &local_uuid_bytes {
                        return;
                    }
                    if n == 20 {
                        return;
                    }
                    // Session 12: attribute the datagram to its sender so the room and
                    // the UI can keep one stream per peer.
                    let sender_uuid = verio_discovery::Uuid::from_bytes(
                        <[u8; 16]>::try_from(&buf[4..20]).expect("uuid slice"),
                    )
                    .to_string();

                    if active_relay_addr != from {
                        active_relay_addr = from;
                        if let Ok(mut d) = diag_clone.write() {
                            d.endpoint = from.to_string();
                        }
                    }

                    last_recv_time = Some(Instant::now());
                    bump_relay_recv(&diag_clone, n);
                    if let Ok(mut d) = diag_clone.write() {
                        d.last_recv_ms_ago = Some(0);
                    }

                    let tag = buf[20];
                    let payload = &buf[21..n];
                    match tag {
                        RELAY_TAG_AUDIO => {
                            match parse_audio_packet(payload) {
                                Ok((seq, capture_ts_ms, opus)) => {
                                    let _ = events.send(TransportEvent::Audio {
                                        peer: sender_uuid.clone(),
                                        seq,
                                        capture_ts_ms,
                                        opus: opus.to_vec(),
                                    });
                                }
                                Err(e) => tracing::warn!("relay bad audio packet: {e}"),
                            }
                        }
                        RELAY_TAG_CONTROL => {
                            let text = String::from_utf8_lossy(payload);
                            match serde_json::from_str::<ControlMessage>(&text) {
                                Ok(ControlMessage::Ping { t }) => {
                                    let pong = ControlMessage::Pong { t };
                                    if let Ok(json) = serde_json::to_string(&pong) {
                                        let mut pkt = Vec::with_capacity(21 + json.len());
                                        pkt.extend_from_slice(&prefix);
                                        pkt.push(RELAY_TAG_CONTROL);
                                        pkt.extend_from_slice(json.as_bytes());
                                        let framed = wrap_stun_envelope(&pkt);
                                        let _ = socket.send_to(&framed, active_relay_addr);
                                        log_relay_packet(
                                            "send",
                                            &room_log,
                                            &local_uuid_short,
                                            active_relay_addr,
                                            framed.len(),
                                            "control",
                                            &mut tx_log,
                                        );
                                        bump_relay_sent(&diag_clone, framed.len());
                                    }
                                }
                                Ok(ControlMessage::Pong { t }) => {
                                    // Only our own ping is worth timing (see last_ping_t).
                                    if t != last_ping_t {
                                        return;
                                    }
                                    let ms = unix_ms().saturating_sub(t) as f64;
                                    if let Ok(mut d) = diag_clone.write() {
                                        d.rtt_ms = Some(ms);
                                    }
                                    let _ = events.send(TransportEvent::Rtt { ms });
                                }
                                Ok(msg) => {
                                    let _ = events.send(TransportEvent::Control {
                                        peer: sender_uuid.clone(),
                                        message: msg,
                                    });
                                }
                                Err(e) => tracing::warn!("relay bad control packet: {e}"),
                            }
                        }
                        _ => {}
                    }
                };

                match socket.recv_from(&mut recv_buf) {
                    Ok((n, from)) => {
                        process_packet(n, from, &recv_buf[..n]);
                        let _ = socket.set_read_timeout(Some(Duration::from_micros(1)));
                        while let Ok((n, from)) = socket.recv_from(&mut recv_buf) {
                            process_packet(n, from, &recv_buf[..n]);
                        }
                        let _ = socket.set_read_timeout(Some(Duration::from_millis(10)));
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock || e.kind() == ErrorKind::TimedOut => {}
                    Err(e) => {
                        #[cfg(windows)]
                        if e.raw_os_error() == Some(10054) {
                            continue;
                        }
                        tracing::warn!("relay socket recv error: {e}");
                    }
                }
            };

            tracing::info!(%reason, "relay session driver exited");
            if let Ok(mut d) = diag_clone.write() {
                d.status = "Disconnected".into();
                d.error = Some(reason.clone());
            }
            let _ = events.send(TransportEvent::Disconnected { reason });
        })
        .map_err(|e| TransportError(format!("spawn relay driver thread: {e}")))?;

    Ok(PeerHandle {
        tx: cmd_tx,
        diagnostics,
    })
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn audio_packet_roundtrip() {
        let opus = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let pkt = encode_audio_packet(42, 1_700_000_000_123, &opus);
        assert_eq!(pkt[0], AUDIO_MAGIC);
        assert_eq!(pkt[1], AUDIO_VERSION);
        let (seq, ts, payload) = parse_audio_packet(&pkt).expect("parse");
        assert_eq!(seq, 42);
        assert_eq!(ts, 1_700_000_000_123);
        assert_eq!(payload, &opus[..]);
        // Header is exactly 12 bytes (branch-2 spec).
        assert_eq!(pkt.len(), 12 + opus.len());
    }

    #[test]
    fn audio_packet_rejects_bad_magic_and_version() {
        let mut pkt = encode_audio_packet(1, 2, &[9]);
        pkt[0] = 0x00;
        assert!(parse_audio_packet(&pkt).is_err(), "wrong magic must fail");
        let mut pkt = encode_audio_packet(1, 2, &[9]);
        pkt[1] = 9;
        assert!(parse_audio_packet(&pkt).is_err(), "wrong version must fail");
        let short = [0u8; 5];
        assert!(parse_audio_packet(&short).is_err(), "short packet must fail");
    }

    #[test]
    fn audio_packet_u48_timestamp() {
        // Max u48 value round-trips.
        let pkt = encode_audio_packet(u32::MAX, (1 << 48) - 1, &[]);
        let (seq, ts, payload) = parse_audio_packet(&pkt).expect("parse");
        assert_eq!(seq, u32::MAX);
        assert_eq!(ts, (1 << 48) - 1);
        assert!(payload.is_empty());
    }

    #[test]
    fn control_message_serde_roundtrip() {
        for msg in [
            ControlMessage::Hello { name: "Mahan".into(), version: "0.1.0".into() },
            ControlMessage::State {
                speaking: true,
                mute: false,
                deafen: false,
                music: false,
                music_title: String::new(),
            },
            ControlMessage::Ping { t: 123_456 },
            ControlMessage::Pong { t: 123_456 },
        ] {
            let json = serde_json::to_string(&msg).expect("serialize");
            let back: ControlMessage = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, msg);
        }
        // Wire format uses snake_case type tags.
        assert!(serde_json::to_string(&ControlMessage::Ping { t: 1 })
            .unwrap()
            .contains(r#""type":"ping""#));
    }

    #[test]
    fn silence_frame_is_small_and_valid() {
        // The keep-alive frame the app encodes once: 20 ms of digital silence
        // must produce a tiny Opus packet (DTX collapses it to ~2 bytes).
        let frame = verio_app_test_silence_frame();
        assert!(!frame.is_empty());
        assert!(frame.len() <= 4, "silence frame should be ≤4 bytes, got {}", frame.len());
        // It must survive the packet codec.
        let pkt = encode_audio_packet(7, 9, &frame);
        let (seq, ts, payload) = parse_audio_packet(&pkt).expect("parse");
        assert_eq!((seq, ts), (7, 9));
        assert_eq!(payload, &frame[..]);
    }

    // Test-only bridge to verio-dsp (dev-dependency, no cycle): the same
    // helper the room uses for the 400 ms keep-alive.
    fn verio_app_test_silence_frame() -> Vec<u8> {
        verio_dsp::codec::OpusEncoderWrapper::opus_silence_frame().expect("silence frame")
    }

    #[test]
    fn test_format_room_code_bytes() {
        assert_eq!(format_room_code_bytes("8842"), *b"8842");
        assert_eq!(format_room_code_bytes("42"), *b"0042");
        assert_eq!(format_room_code_bytes("12345"), *b"1234");
    }

    #[test]
    fn test_relay_session_loopback() {
        let relay_socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        let relay_addr = relay_socket.local_addr().unwrap();

        let relay_sock_clone = relay_socket.try_clone().unwrap();
        let stop_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop_flag);
        let relay_handle = std::thread::spawn(move || {
            let mut buf = [0u8; 2048];
            let mut clients: std::collections::HashMap<String, std::collections::HashMap<[u8; 16], SocketAddr>> = std::collections::HashMap::new();
            relay_sock_clone.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
            while !stop_clone.load(std::sync::atomic::Ordering::Relaxed) {
                if let Ok((n, from)) = relay_sock_clone.recv_from(&mut buf) {
                    if n >= 20 {
                        // Session 10: the real relay server strips the client's
                        // STUN-shaped envelope before routing; mirror that here.
                        let inner: &[u8] = if n > 20
                            && buf[0..2] == [0x00, 0x01]
                            && buf[4..8] == [0x21, 0x12, 0xa4, 0x42]
                        {
                            &buf[20..n]
                        } else {
                            &buf[..n]
                        };
                        if inner.len() >= 20 {
                            let room = String::from_utf8_lossy(&inner[0..4]).to_string();
                            let mut sender_uuid = [0u8; 16];
                            sender_uuid.copy_from_slice(&inner[4..20]);
                            let room_map = clients.entry(room).or_default();
                            room_map.insert(sender_uuid, from);
                            if inner.len() > 20 {
                                for (&uuid, &target) in room_map.iter() {
                                    if uuid != sender_uuid && target != from {
                                        let _ = relay_sock_clone.send_to(inner, target);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        let id_a = verio_discovery::Identity {
            uuid: verio_discovery::Uuid::from_u128(1),
            name: "Alice".into(),
            version: "0.1.0".into(),
        };
        let id_b = verio_discovery::Identity {
            uuid: verio_discovery::Uuid::from_u128(2),
            name: "Bob".into(),
            version: "0.1.0".into(),
        };
        let silence = Arc::new(vec![0u8; 2]);
        let (tx_a, rx_a) = mpsc::channel();
        let (tx_b, rx_b) = mpsc::channel();

        let handle_a = spawn_relay_session(relay_addr, "9999".into(), id_a.clone(), id_b.clone(), silence.clone(), tx_a).unwrap();
        let handle_b = spawn_relay_session(relay_addr, "9999".into(), id_b.clone(), id_a.clone(), silence.clone(), tx_b).unwrap();

        assert!(matches!(rx_a.recv_timeout(Duration::from_secs(1)).unwrap(), TransportEvent::Connected));
        assert!(matches!(rx_b.recv_timeout(Duration::from_secs(1)).unwrap(), TransportEvent::Connected));

        std::thread::sleep(Duration::from_millis(50));

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut got_audio = false;
        while Instant::now() < deadline {
            handle_a.send_audio(1234567, vec![10, 20, 30, 40]);
            if let Ok(ev) = rx_b.recv_timeout(Duration::from_millis(50)) {
                if let TransportEvent::Audio { capture_ts_ms, opus, .. } = ev {
                    assert_eq!(capture_ts_ms, 1234567);
                    assert_eq!(opus, vec![10, 20, 30, 40]);
                    got_audio = true;
                    break;
                }
            }
        }
        assert!(got_audio, "Bob should receive audio from Alice via relay");

        stop_flag.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = relay_handle.join();
        handle_a.disconnect();
        handle_b.disconnect();
    }

    #[test]
    fn test_embedded_voice_host() {
        let mut host = EmbeddedVoiceHost::start(0).expect("start embedded host");
        let port = host.local_port();
        assert!(port > 0);
        let relay_addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();

        let id_a = verio_discovery::Identity {
            uuid: verio_discovery::Uuid::from_u128(10),
            name: "HostUser".into(),
            version: "0.1.0".into(),
        };
        let id_b = verio_discovery::Identity {
            uuid: verio_discovery::Uuid::from_u128(20),
            name: "GuestUser".into(),
            version: "0.1.0".into(),
        };
        let silence = Arc::new(vec![0u8; 2]);
        let (tx_a, rx_a) = mpsc::channel();
        let (tx_b, rx_b) = mpsc::channel();

        let handle_a = spawn_relay_session_with_mode(relay_addr, "8888".into(), id_a.clone(), id_b.clone(), silence.clone(), tx_a, "peer_host").unwrap();
        let handle_b = spawn_relay_session_with_mode(relay_addr, "8888".into(), id_b.clone(), id_a.clone(), silence.clone(), tx_b, "peer_host").unwrap();

        assert!(matches!(rx_a.recv_timeout(Duration::from_secs(1)).unwrap(), TransportEvent::Connected));
        assert!(matches!(rx_b.recv_timeout(Duration::from_secs(1)).unwrap(), TransportEvent::Connected));

        std::thread::sleep(Duration::from_millis(50));

        let deadline = Instant::now() + Duration::from_secs(3);
        let mut received = false;
        while Instant::now() < deadline {
            handle_a.send_audio(99999, vec![1, 2, 3, 4]);
            if let Ok(TransportEvent::Audio { capture_ts_ms, opus, .. }) = rx_b.recv_timeout(Duration::from_millis(50)) {
                assert_eq!(capture_ts_ms, 99999);
                assert_eq!(opus, vec![1, 2, 3, 4]);
                received = true;
                break;
            }
        }
        assert!(received, "Guest should receive audio from Host via EmbeddedVoiceHost");

        let diag_a = handle_a.diagnostics();
        assert_eq!(diag_a.mode, "peer_host");
        assert!(diag_a.packets_sent > 0);

        handle_a.disconnect();
        handle_b.disconnect();
        host.stop();
    }
}









