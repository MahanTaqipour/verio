//! Phase 2 agent-side verification harness: two full RoomManager instances
//! (identity + signaling + str0m transport + decode) connect over loopback
//! TCP signaling and exchange real Opus frames in BOTH directions through the
//! same path the app uses (audio pump → DataChannel → transport pump → RX).
//!
//! Evidence printed (HONESTY RULES: real measured output only):
//! - per-side decoded-packet growth every 5 s (late/lost/backlog)
//! - one-way transport latency (capture_ts_ms → arrival) p50/p95 per 5 s
//! - control-channel RTT p50/p95 (ping/pong every 2 s)
//! - speaking-state (hello/state JSON) propagation counts both directions
//!
//! Usage: `room_soak [seconds]` (default 600 = the 10-minute soak).
//! Exit code 0 iff every AC check below passes; each check is printed.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use verio_app::pipeline::{NetAudioIn, NetAudioOut};
use verio_app::room::{RoomEvent, RoomManager, RoomState};
use verio_discovery::{DirectListener, Identity};
use verio_dsp::codec::{OpusDecoderWrapper, OpusEncoderWrapper};
use verio_transport::unix_ms;

const FRAME_MS: u64 = 20;
const SAMPLES_PER_FRAME: usize = 960; // 20 ms @ 48 kHz

/// Rolling per-side RX statistics.
#[derive(Default)]
struct RxStats {
    decoded: u64,
    late: u64,
    lost: u64,
    oneway_ms: Vec<u64>,
    /// Every one-way sample ever seen (never drained) — final AC summary.
    oneway_all: Vec<u64>,
    backlog_max: usize,
}

/// Percentile of an ARBITRARY-ORDER slice (sorts a copy internally so call
/// sites can never feed it unsorted data by accident).
fn percentile(values: &[u64], p: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = (((sorted.len() as f64) - 1.0) * p).round() as usize;
    sorted[idx]
}

fn pcts_f(v: &[f64]) -> (f64, f64, f64) {
    if v.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pick = |p: f64| s[(((s.len() as f64) - 1.0) * p).round() as usize];
    (pick(0.50), pick(0.95), s[s.len() - 1])
}

fn check(name: &str, ok: bool) -> bool {
    println!("  [{}] {name}", if ok { "PASS" } else { "FAIL" });
    ok
}

fn print_window(side: &str, t: u64, stats: &Arc<Mutex<RxStats>>) {
    let mut s = stats.lock().unwrap();
    let sorted = std::mem::take(&mut s.oneway_ms);
    let (p50, p95) = (percentile(&sorted, 0.50), percentile(&sorted, 0.95));
    println!(
        "[{side}] t=+{t}s decoded={} late={} lost={} backlog_max={} oneway p50={p50}ms p95={p95}ms",
        s.decoded, s.late, s.lost, s.backlog_max
    );
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,str0m=debug,is=debug")),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    let soak_secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(600);
    println!("=== verio Phase 2 two-peer loopback soak: {soak_secs}s ===");

    // Identities: B has the larger uuid → B is the deterministic offerer.
    let id_a = Identity {
        uuid: uuid::Uuid::from_u128(0xA000_0000_0000_0000_0000_0000_0000_0001),
        name: "InstanceA".into(),
        version: verio_app::APP_VERSION.into(),
    };
    let id_b = Identity {
        uuid: uuid::Uuid::from_u128(0xB000_0000_0000_0000_0000_0000_0000_0002),
        name: "InstanceB".into(),
        version: verio_app::APP_VERSION.into(),
    };

    let (ev_a, ev_a_rx) = mpsc::channel::<RoomEvent>();
    let (ev_b, ev_b_rx) = mpsc::channel::<RoomEvent>();
    let room_a = Arc::new(Mutex::new(RoomManager::new(id_a.clone(), ev_a)));
    let room_b = Arc::new(Mutex::new(RoomManager::new(id_b.clone(), ev_b)));
    // RX slots: tap (arrival timestamp, blocking recv) → decode (counters),
    // both writing into one shared stats struct per side.
    let stats_a = Arc::new(Mutex::new(RxStats::default()));
    let stats_b = Arc::new(Mutex::new(RxStats::default()));
    let (tap_a_tx, tap_a_rx) = mpsc::channel::<NetAudioIn>();
    let (tap_b_tx, tap_b_rx) = mpsc::channel::<NetAudioIn>();
    room_a.lock().unwrap().set_net_in(Some(tap_a_tx));
    room_b.lock().unwrap().set_net_in(Some(tap_b_tx));

    // Instance A listens on an ephemeral port (stand-in for 49860).
    let listener = Arc::new(DirectListener::bind(0).expect("bind signaling listener"));
    let addr = format!("127.0.0.1:{}", listener.port());
    println!("A listening on {addr}");
    {
        let ra = Arc::clone(&room_a);
        let l = Arc::clone(&listener);
        std::thread::Builder::new()
            .name("soak-accept".into())
            .spawn(move || loop {
                match l.accept() {
                    Ok(session) => RoomManager::run_accept_direct(&ra, session),
                    Err(e) => {
                        eprintln!("accept failed: {e}");
                        break;
                    }
                }
            })
            .expect("spawn accept");
    }

    // Instance B dials A's direct code (exactly what the UI command does).
    {
        let rb = Arc::clone(&room_b);
        std::thread::spawn(move || RoomManager::run_connect_direct(&rb, addr));
    }

    // Event collectors + shared result slots.
    let connected = Arc::new(Mutex::new([false; 2]));
    let states = Arc::new(Mutex::new([0u64; 4]));
    let rtts: [Arc<Mutex<Vec<f64>>>; 2] = [Default::default(), Default::default()];
    let errors: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    spawn_event_collector("A", ev_a_rx, 0, &connected, &states, &rtts[0], &errors);
    spawn_event_collector("B", ev_b_rx, 1, &connected, &states, &rtts[1], &errors);

    // Wait for both Connected (30 s budget).
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let c = *connected.lock().unwrap();
        if c[0] && c[1] {
            break;
        }
        if Instant::now() > deadline {
            eprintln!("FAIL: connect timeout (A connected: {}, B connected: {})", c[0], c[1]);
            for e in errors.lock().unwrap().iter() {
                eprintln!("  error: {e}");
            }
            std::process::exit(1);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("CONNECTED both sides (WebRTC ICE + DTLS + DataChannels)");

    // RX tap + decoder threads.
    let (dec_a_tx, dec_a_rx) = mpsc::channel::<NetAudioIn>();
    let (dec_b_tx, dec_b_rx) = mpsc::channel::<NetAudioIn>();
    spawn_net_tap("A", tap_a_rx, dec_a_tx, Arc::clone(&stats_a));
    spawn_net_tap("B", tap_b_rx, dec_b_tx, Arc::clone(&stats_b));
    spawn_rx_decoder("A", dec_a_rx, Arc::clone(&stats_a));
    spawn_rx_decoder("B", dec_b_rx, Arc::clone(&stats_b));

    // Audio feed threads: real Opus-encoded 20 ms frames into each side's
    // net_out channel (the same channel the pipeline's processing thread uses).
    let stop = Arc::new(AtomicBool::new(false));
    spawn_feed(&room_a, "A", Arc::clone(&stop));
    spawn_feed(&room_b, "B", Arc::clone(&stop));

    // Speaking-state toggle every 5 s (exercises state JSON on the control
    // channel → remote PeerState → UI speaking ring).
    {
        let ra = Arc::clone(&room_a);
        let rb = Arc::clone(&room_b);
        let stop2 = Arc::clone(&stop);
        std::thread::spawn(move || {
            let mut speaking = false;
            while !stop2.load(Ordering::Relaxed) {
                speaking = !speaking;
                ra.lock()
                    .unwrap()
                    .send_state(speaking, false, false, false, String::new());
                rb.lock()
                    .unwrap()
                    .send_state(!speaking, false, false, false, String::new());
                std::thread::sleep(Duration::from_secs(5));
            }
        });
    }

    // Soak window: print per-side stats every 5 s.
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(soak_secs) {
        std::thread::sleep(Duration::from_secs(5));
        let t = started.elapsed().as_secs();
        print_window("A", t, &stats_a);
        print_window("B", t, &stats_b);
    }
    stop.store(true, Ordering::Relaxed);
    std::thread::sleep(Duration::from_millis(300));

    // ---- final AC summary -------------------------------------------------
    println!(
        "=== SOAK COMPLETE after {}s — final stats ===",
        started.elapsed().as_secs()
    );
    let (sa, sb) = (stats_a.lock().unwrap(), stats_b.lock().unwrap());
    let (rt_a, rt_b) = (rtts[0].lock().unwrap(), rtts[1].lock().unwrap());
    println!(
        "A→B RX (decoded on B): decoded={} late={} lost={} backlog_max={}",
        sb.decoded, sb.late, sb.lost, sb.backlog_max
    );
    println!(
        "B→A RX (decoded on A): decoded={} late={} lost={} backlog_max={}",
        sa.decoded, sa.late, sa.lost, sa.backlog_max
    );
    let oneway_p95 = sa.p95().max(sb.p95());
    let rtt_all: Vec<f64> = rt_a.iter().chain(rt_b.iter()).copied().collect();
    let (rt_p50, rt_p95, rt_max) = pcts_f(&rtt_all);
    println!(
        "RTT: samples={} p50={rt_p50:.1}ms p95={rt_p95:.1}ms max={rt_max:.1}ms",
        rtt_all.len()
    );
    let err_list = errors.lock().unwrap().clone();
    let st = *states.lock().unwrap();
    println!(
        "Speaking-state events: A received from B: speaking={} mute={} | B received from A: speaking={} mute={}",
        st[0], st[1], st[2], st[3]
    );
    println!("Errors ({}):", err_list.len());
    for e in err_list.iter().take(10) {
        println!("  {e}");
    }

    let mut pass = true;
    pass &= check("both instances connected", true);
    pass &= check("decoded packets grew on BOTH sides", sa.decoded > 1000 && sb.decoded > 1000);
    pass &= check("RTT p95 < 10 ms (loopback)", rt_p95 < 10.0);
    pass &= check("one-way transport latency p95 < 40 ms", oneway_p95 < 40);
    pass &= check("speaking state propagated both ways", st[0] > 0 && st[2] > 0);
    pass &= check("no room errors / disconnects", err_list.is_empty());
    println!("=== SOAK RESULT: {} ===", if pass { "PASS" } else { "FAIL" });
    if !pass {
        std::process::exit(1);
    }
}
fn spawn_event_collector(
    side: &'static str,
    rx: Receiver<RoomEvent>,
    slot: usize,
    connected: &Arc<Mutex<[bool; 2]>>,
    states: &Arc<Mutex<[u64; 4]>>,
    rtt: &Arc<Mutex<Vec<f64>>>,
    errors: &Arc<Mutex<Vec<String>>>,
) {
    let connected = Arc::clone(connected);
    let states = Arc::clone(states);
    let rtt = Arc::clone(rtt);
    let errors = Arc::clone(errors);
    std::thread::spawn(move || {
        while let Ok(ev) = rx.recv() {
            match ev {
                RoomEvent::StateChanged { state } => {
                    if state == RoomState::Connected {
                        connected.lock().unwrap()[slot] = true;
                        println!("[{side}] room state → Connected");
                    }
                }
                RoomEvent::PeerConnected { name, .. } => println!("[{side}] peer hello: {name}"),
                RoomEvent::PeerState {
                    speaking,
                    mute: _,
                    peer_id: _,
                    deafen: _,
                    music: _,
                    music_title: _,
                } => {
                    let mut st = states.lock().unwrap();
                    let idx = match (slot, speaking) {
                        (0, true) => 0,
                        (0, false) => 1,
                        (_, true) => 2,
                        (_, false) => 3,
                    };
                    st[idx] += 1;
                }
                RoomEvent::Rtt { ms } => rtt.lock().unwrap().push(ms),
                RoomEvent::Error { message } => {
                    errors.lock().unwrap().push(format!("[{side}] {message}"))
                }
                RoomEvent::PeerDisconnected { reason, peer_id: _ } => {
                    errors
                        .lock()
                        .unwrap()
                        .push(format!("[{side}] disconnected: {reason}"));
                }
                RoomEvent::RoomCreated { .. }
                | RoomEvent::RoomJoined { .. }
                | RoomEvent::TransportModeChanged { .. } => {}
            }
        }
    });
}

/// Arrival tap: sits between the transport pump and the decode thread. Takes a
/// BLOCKING recv (no sleeps) so the one-way timestamp is taken the moment the
/// frame arrives from the transport — the decode thread's batch/sleep pattern
/// would otherwise add 20-60 ms of pure measurement artifact. Forwards the
/// frame to the decoder channel untouched.
fn spawn_net_tap(
    side: &'static str,
    rx: Receiver<NetAudioIn>,
    tx: Sender<NetAudioIn>,
    stats: Arc<Mutex<RxStats>>,
) {
    std::thread::Builder::new()
        .name(format!("soak-tap-{side}"))
        .spawn(move || {
            while let Ok(pkt) = rx.recv() {
                let oneway = unix_ms().saturating_sub(pkt.capture_ts_ms);
                {
                    let mut s = stats.lock().unwrap();
                    s.oneway_ms.push(oneway);
                    s.oneway_all.push(oneway);
                }
                if tx.send(pkt).is_err() {
                    return;
                }
            }
        })
        .expect("spawn net tap");
}

/// Decodes every incoming frame through the app's Opus decoder and records
/// late/lost (seq gaps) and backlog depth. One-way latency is recorded by the
/// tap upstream, not here.
fn spawn_rx_decoder(
    side: &'static str,
    rx: Receiver<NetAudioIn>,
    stats: Arc<Mutex<RxStats>>,
) {
    std::thread::Builder::new()
        .name(format!("soak-rx-{side}"))
        .spawn(move || {
            let mut decoder = OpusDecoderWrapper::new().expect("decoder");
            let mut last_seq: Option<u32> = None;
            loop {
                // Backlog proxy: frames queued up while we slept.
                let mut batch = 0usize;
                loop {
                    match rx.try_recv() {
                        Ok(pkt) => {
                            batch += 1;
                            {
                                let mut s = stats.lock().unwrap();
                                if let Some(last) = last_seq {
                                    let gap = pkt.seq.wrapping_sub(last);
                                    if gap == 0 || gap > u32::MAX / 2 {
                                        s.late += 1;
                                    } else if gap > 1 {
                                        s.lost += u64::from(gap - 1);
                                    }
                                }
                                last_seq = Some(pkt.seq);
                            }
                            let _ = decoder.decode(Some(&pkt.opus));
                            stats.lock().unwrap().decoded += 1;
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => return,
                    }
                }
                {
                    let mut s = stats.lock().unwrap();
                    s.backlog_max = s.backlog_max.max(batch);
                }
                std::thread::sleep(Duration::from_millis(FRAME_MS));
            }
        })
        .expect("spawn rx decoder");
}

impl RxStats {
    /// p95 over ALL one-way samples seen during the whole run.
    fn p95(&self) -> u64 {
        percentile(&self.oneway_all, 0.95)
    }
}

/// Encodes real Opus frames (sine tone, distinct per side) every 20 ms and
/// pushes them into the room's net_out channel — the exact channel the
/// pipeline's processing thread feeds in the app.
fn spawn_feed(
    room: &Arc<Mutex<RoomManager>>,
    side: &'static str,
    stop: Arc<AtomicBool>,
) {
    let tx = room.lock().unwrap().net_out_tx();
    std::thread::Builder::new()
        .name(format!("soak-feed-{side}"))
        .spawn(move || {
            let mut encoder = OpusEncoderWrapper::new().expect("encoder");
            let freq = if side == "A" { 440.0 } else { 550.0 };
            let mut phase = 0.0f32;
            // Deadline-based 20 ms cadence: Windows sleep granularity (~15.6 ms)
            // makes naive sleep(20) cluster frames; a hard deadline + short
            // final spin keeps the offer rate close to the real audio clock.
            let mut next = Instant::now();
            while !stop.load(Ordering::Relaxed) {
                next += Duration::from_millis(FRAME_MS);
                let mut samples = Vec::with_capacity(SAMPLES_PER_FRAME);
                for _ in 0..SAMPLES_PER_FRAME {
                    samples.push(phase.sin() * 0.2);
                    phase += 2.0 * std::f32::consts::PI * freq / 48000.0;
                }
                if let Ok(Some(opus)) = encoder.push(&samples) {
                    let _ = tx.send(NetAudioOut {
                        capture_ts_ms: unix_ms(),
                        opus,
                    });
                }
                let now = Instant::now();
                if next > now {
                    let remain = next - now;
                    if remain > Duration::from_millis(3) {
                        std::thread::sleep(remain - Duration::from_millis(2));
                    }
                    while Instant::now() < next {
                        std::hint::spin_loop();
                    }
                }
            }
        })
        .expect("spawn feed");
}
