//! Session 29c: what a **listener's** roster learns about music it does not own.
//!
//! The panel's "Room music: <track> — hosted by <name>" line is assembled from two
//! facts that arrive over the wire, and until now neither had been checked against a
//! real second session:
//!
//!   1. **The declaration.** The music session says `is_bot`. That is the only thing
//!      separating a music session from a person, and what makes the listener render
//!      a room-music line instead of its own idle player.
//!   2. **The track.** The bot announces `music_title` in a control-channel `State`,
//!      on every track change and every 3 s, which the roster stores per peer.
//!
//! Both were previously covered only by the dev mock — which is a *description* of
//! the wire, not the wire. These tests run two real `RoomManager` sessions over the
//! real loopback signalling and transport path and assert what the second one
//! actually received. The pair is deliberate: one test proves a music session is
//! flagged, the other proves a person is not, so neither can pass by the flag being
//! constant.
//!
//! # A DEFECT THESE TESTS FOUND, AND WHY THEY DO NOT ASSERT `name` OR `user_id`
//!
//! Instrumenting `upsert_peer` and the `Hello` arm showed that in the **direct P2P
//! path** each session receives its *own* `Hello` back as an incoming frame, and
//! every direct-path control frame arrives with an **empty sender id**, so
//! `resolve_peer` falls back to the single known remote. The result, observed:
//!
//! ```text
//! room=Listener  frame_sender=  resolved=a0..01(bot)  name=Music     <- the bot's Hello, correct
//! room=Listener  frame_sender=  resolved=a0..01(bot)  name=Listener  <- its OWN Hello, echoed
//! room=Music     frame_sender=  resolved=b0..02(list) name=Listener  <- the listener's Hello, correct
//! room=Music     frame_sender=  resolved=b0..02(list) name=Music     <- its OWN Hello, echoed
//! ```
//!
//! So each room overwrites the *remote* peer's roster entry with its own `name` and
//! `user_id`. Asserting either here would be asserting a race: true at the handshake,
//! false a moment later. That is a real defect with real consequences — a peer card
//! showing your own name, and Session 29's room-wide mute being keyed on `user_id` —
//! but it is in the transport/roster core and outside this session's scope, so it is
//! reported rather than fixed here. `is_bot` and `music_title` survive it: the former
//! because the one-way ratchet only ever sets it *true*, and the latter because no
//! `Hello` touches it.
//!
//! The relay path — the one the app's rooms actually use — is not affected: the relay
//! does not echo a sender's own datagrams, and there the roster is seeded from the
//! server's `PeerIdentity`. That is inferred from those two documented behaviours,
//! not measured here.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use verio_app::db::Db;
use verio_app::room::{PeerInfo, RoomEvent, RoomManager, RoomState};
use verio_discovery::{DirectListener, Identity};

/// How long to wait for a real handshake. Loopback ICE + DTLS settles in well under
/// a second; the budget is for a loaded machine, not for a slow path.
const DEADLINE: Duration = Duration::from_secs(30);

/// The track the bot announces, and the title the panel must render.
const TRACK: &str = "Awake";

/// A private, empty local store per call.
///
/// The name is unique per call rather than per test because the room persists its
/// mutes through it, and two tests sharing a directory delete each other's database
/// out from under an open connection ("database is locked").
fn temp_db(name: &str) -> Db {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("verio-music-peer-{name}-{n}"));
    let _ = std::fs::remove_dir_all(&dir);
    Db::open(&dir).expect("open test database")
}

/// The music session: a bot, carrying its host's persistent id.
fn bot_identity() -> Identity {
    Identity {
        uuid: verio_discovery::Uuid::from_u128(0xA000_0000_0000_0000_0000_0000_0000_0001),
        // The host's id, shared on purpose: the bot's own name is always "Music", so
        // this is the only thing that says whose music it is.
        user_id: "user-host-kaveh".to_string(),
        is_bot: true,
        force_stop: true,
        name: "Music".to_string(),
        version: String::new(),
    }
}

/// An ordinary person. `n` orders the uuid, and the larger uuid offers.
///
/// The base is high on purpose: the bot's uuid starts at `0xA0…`, so a person built
/// from `n` alone would sort *below* it and silently swap the offerer role.
fn person_identity(name: &str, n: u128) -> Identity {
    Identity {
        uuid: verio_discovery::Uuid::from_u128(
            0xB000_0000_0000_0000_0000_0000_0000_0000 + n,
        ),
        user_id: format!("user-{name}"),
        is_bot: false,
        force_stop: false,
        name: name.to_string(),
        version: String::new(),
    }
}

/// The remote peers one session currently believes are in the room.
fn peers_of(room: &Arc<Mutex<RoomManager>>) -> Vec<PeerInfo> {
    room.lock().expect("room lock").peers()
}

/// Poll until `f` yields, or fail with `what` after [`DEADLINE`].
///
/// A bare `sleep(2)` would make these tests pass on a machine where the handshake
/// takes 2.1 s and the assertion then reads a roster that is legitimately empty, so
/// the wait is on the *condition* rather than on the clock.
fn wait_for<T>(what: &str, mut f: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Some(v) = f() {
            return v;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Two real sessions connected over the loopback signalling and transport path.
///
/// `a` listens and `b` dials, which is incidental — the direct path decides the
/// offerer by uuid order, not by who dials — so `b` must carry the larger uuid.
fn connected_pair(a: Identity, b: Identity) -> (Arc<Mutex<RoomManager>>, Arc<Mutex<RoomManager>>) {
    assert!(
        b.uuid > a.uuid,
        "the dialing side must carry the larger uuid or the offerer role flips"
    );
    let (a_tx, _a_rx) = mpsc::channel::<RoomEvent>();
    let (b_tx, _b_rx) = mpsc::channel::<RoomEvent>();
    let room_a = Arc::new(Mutex::new(RoomManager::new(a, a_tx, temp_db("a"))));
    let room_b = Arc::new(Mutex::new(RoomManager::new(b, b_tx, temp_db("b"))));

    let signaling = Arc::new(DirectListener::bind(0).expect("bind loopback signalling"));
    let addr = format!("127.0.0.1:{}", signaling.port());
    {
        let room = Arc::clone(&room_a);
        let l = Arc::clone(&signaling);
        std::thread::spawn(move || {
            if let Ok(session) = l.accept() {
                RoomManager::run_accept_direct(&room, session);
            }
        });
    }
    {
        let room = Arc::clone(&room_b);
        std::thread::spawn(move || RoomManager::run_connect_direct(&room, addr));
    }

    let connected = |r: &Arc<Mutex<RoomManager>>| {
        matches!(r.lock().expect("room lock").state(), RoomState::Connected)
    };
    wait_for("session A to connect", || connected(&room_a).then_some(()));
    wait_for("session B to connect", || connected(&room_b).then_some(()));
    // The roster is built from `Hello` frames, not from the handshake, so a session
    // that never announces stays invisible. Every session gossips every 3 s in the
    // running app; here it is explicit.
    room_a.lock().expect("room lock").send_hello();
    room_b.lock().expect("room lock").send_hello();
    (room_a, room_b)
}

#[test]
fn a_listener_learns_the_room_track_from_the_music_peer() {
    let (bot_room, listener_room) = connected_pair(bot_identity(), person_identity("Listener", 2));

    // What the bot announces: exactly what `music_bot.rs` sends on spawn, and again on
    // every track change. Sent from the *bot's* session, so the listener has to learn
    // it off the wire rather than from a shared struct.
    {
        let mut m = bot_room.lock().expect("room lock");
        m.send_state(false, false, false, true, TRACK.to_string());
    }

    // 1. The declaration. This is what makes the listener's panel show a room-music
    //    line instead of its own idle player.
    let peer = wait_for("the music peer to reach the listener's roster", || {
        peers_of(&listener_room).into_iter().find(|p| p.is_bot)
    });
    assert!(
        peer.is_bot,
        "the listener must be able to tell a music session from a person"
    );
    assert_eq!(
        peers_of(&listener_room).len(),
        1,
        "the room holds exactly one remote peer, so this is that peer and not a stray"
    );

    // 2. The track: the title the panel renders. Waited for separately, because the
    //    peer appears at the handshake while the title only arrives with the `State`.
    let peer = wait_for("the announced track to reach the listener", || {
        peers_of(&listener_room)
            .into_iter()
            .find(|p| p.is_bot && !p.music_title.is_empty())
    });
    assert_eq!(
        peer.music_title, TRACK,
        "the listener's roster must carry the track the bot announced"
    );
    assert!(
        peer.music,
        "a playing bot announces music presence, which is what makes its tile a music tile"
    );
}

/// The matched pair for the test above: without this, `is_bot` could be `true` of
/// every peer and the first test would still pass.
#[test]
fn a_person_is_not_flagged_as_a_music_peer() {
    let (a_room, b_room) = connected_pair(person_identity("Ada", 1), person_identity("Bo", 2));

    let peer = wait_for("the other person to reach the roster", || {
        peers_of(&b_room).into_iter().next()
    });
    assert!(
        !peer.is_bot,
        "a person must never be flagged as a music session, or every tile would offer to stop music"
    );
    assert_eq!(
        peer.music_title, "",
        "a person who is not playing must not claim a track"
    );
    assert!(!peer.music, "and must not claim music presence");

    // Both directions, because the flag is written per roster entry and a one-sided
    // check would miss a bug in the other side's write.
    let peer = wait_for("Ada to reach Bo's roster", || {
        peers_of(&a_room).into_iter().next()
    });
    assert!(!peer.is_bot);
}
