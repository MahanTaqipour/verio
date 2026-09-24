/**
 * Realistic fixtures for the Verio dev mock harness.
 *
 * Rules this file follows:
 *  1. Every field exists in the real Rust types. Nothing is invented.
 *  2. Sample content is plausible for Verio (2-10 person voice rooms on
 *     Windows), not lorem ipsum.
 *  3. Error strings are shaped after the real ones in the Rust crates, so the
 *     UI's error presentation is exercised against realistic text lengths.
 *  4. No capability is fabricated. There is no jitter, no loss percentage, no
 *     per-peer level, no playlist, no encryption name, no identity keypair,
 *     no DSP load — because the application does not produce them.
 */

import type {
  AudioState,
  DeviceList,
  MusicBotState,
  MusicStatus,
  PeerInfo,
  QueueItem,
  RoomSnapshot,
  Settings,
  TransportDiagnostics,
} from "./types";

// ---------------------------------------------------------------------------
// Sample content
// ---------------------------------------------------------------------------

/** Local user's nickname. Kept out of the remote roster. */
const LOCAL_NICKNAME = "Kaveh";

/** Room code used by the in-call scenarios. */
const ROOM_CODE = "4821";

/** Default relay from `Settings::default()` in crates/verio-app/src/settings.rs. */
const DEFAULT_VPS = "ws://127.0.0.1:8443";

/** 9 remote peers → 10 total participants with the local user. Max supported. */
const ROSTER_NAMES = [
  "Dan",
  "Arman",
  "Nika",
  "Kian",
  "Sora",
  "Elena",
  "Reza",
  "Tara",
  "Sina",
] as const;

/**
 * Deterministic uuid per roster index. The app sorts peers by uuid, so the
 * leading digits keep the rendered order stable across scenarios.
 */
function peerUuid(index: number): string {
  const n = String(index + 1).padStart(2, "0");
  return `${n}1a2b3c-4d5e-4f60-8a90-00000000${n}00`;
}

function peer(index: number, over: Partial<PeerInfo> = {}): PeerInfo {
  return {
    uuid: peerUuid(index),
    // Session 29: a persistent id per roster member. Distinct from the uuid, as
    // in the real app — a fixture that reused the uuid would make the mute tests
    // pass for the wrong reason.
    user_id: `user-${(ROSTER_NAMES[index] ?? `peer${index + 1}`).toLowerCase()}`,
    name: ROSTER_NAMES[index] ?? `Peer ${index + 1}`,
    speaking: false,
    mute: false,
    deafen: false,
    music: false,
    music_title: "",
    // Session 29b: a person unless a fixture says otherwise. The bot peer in the
    // roster is built from this same helper with `is_bot: true`, because a bot
    // differs from a person in exactly one field.
    is_bot: false,
    // Session 29b Force Stop: a person never accepts a remote stop. The bot peer
    // overrides this from the bot's own start, so the value on a bot fixture is the
    // one the app would really have announced rather than a fixture's opinion.
    force_stop: false,
    ...over,
  };
}

/** First `count` roster members, with optional per-index overrides. */
function roster(
  count: number,
  overrides: Record<number, Partial<PeerInfo>> = {},
): PeerInfo[] {
  return Array.from({ length: count }, (_, i) => peer(i, overrides[i] ?? {}));
}

/**
 * Session 29b Force Stop: a bot peer as somebody *else* sees it — a music session
 * in the room that this client does not own.
 *
 * Needed because `music_bot_start` can only ever create *our* bot, and a bot in the
 * roster is otherwise always ours. That makes the listener's half of Force Stop —
 * the Stop control, and its disabled state — unreachable in the harness, which is
 * exactly the half the plan describes for "other clients".
 *
 * `hostIndex` names the roster member the bot belongs to, so the Host chip has a
 * voice card to land on: the two sessions share a persistent id and that is the only
 * link between them.
 */
function remoteBotPeer(hostIndex: number, forceStop: boolean): PeerInfo {
  const host = ROSTER_NAMES[hostIndex] ?? `peer${hostIndex + 1}`;
  return peer(hostIndex + 1, {
    is_bot: true,
    name: "Music",
    user_id: `user-${host.toLowerCase()}`,
    force_stop: forceStop,
    music: true,
    music_title: MUSIC_TRACK.title,
  });
}

// ---------------------------------------------------------------------------
// Device list (names are plausible Windows WASAPI endpoints)
// ---------------------------------------------------------------------------

const DEVICES: DeviceList = {
  inputs: [
    { name: "Shure SM7B (Elgato Wave XLR)", is_default: false },
    { name: "Microphone (Realtek(R) Audio)", is_default: true },
    { name: "Headset Microphone (Schiit Hel 2E)", is_default: false },
  ],
  outputs: [
    { name: "Schiit Hel 2E DAC/Amp", is_default: true },
    { name: "Speakers (Realtek(R) Audio)", is_default: false },
    { name: "Sony WH-1000XM4 (Hands-Free AG Audio)", is_default: false },
  ],
};

const NO_DEVICES: DeviceList = { inputs: [], outputs: [] };

// ---------------------------------------------------------------------------
// Base state
// ---------------------------------------------------------------------------

export interface MockState {
  settings: Settings;
  devices: DeviceList;
  audio: AudioState;
  room: RoomSnapshot;
  music: MusicStatus;
  /**
   * Session 20: the play queue. Optional — a scenario that omits it starts with
   * an empty queue, which is also what exercises the EmptyState in the panel.
   */
  queue?: QueueItem[];
  /**
   * Session 29b: the music bot session, when a scenario has one running.
   *
   * Optional like `queue`, and for the same reason: almost every screen in the
   * harness is not hosting, and making this required would mean editing thirty
   * scenarios to say "no". A scenario that omits it is idle, which is what the
   * real `music_bot_state` reports for a client that has not pressed Play.
   */
  bot?: MusicBotState;
}

export interface MockBehavior {
  /** Artificial IPC latency, ms. Reads should feel instant; writes a little slower. */
  latency: { read: number; write: number };
  /** Commands that reject with the given message, keyed by command name. */
  failures: Record<string, string>;
  /** Stream a synthetic mic level into the app's imperative meter. */
  micLevel: { enabled: boolean; db: number };
  /** Emit `audio_error` once after boot (real event, real payload shape). */
  audioError?: string;
  /**
   * Emit `room_error` once after boot (real event, real payload shape).
   *
   * Session 24: added because `room-connection-lost`'s note claimed it already
   * did this and it did not — the only emitter was the harness panel's button.
   * Mirrors `audioError` exactly.
   */
  roomError?: string;
  /**
   * REMOVED in Session 24: `serverReachable: boolean`.
   *
   * It was set in 26 scenarios and read by nothing — no code ever consulted it,
   * so it controlled no behaviour. Every scenario that set it `false` already
   * produced its state through a mechanism that *is* read: `relay-unreachable`
   * through `failures`, `room-connection-lost` through `diagOffline()`. It was
   * vestigial config that merely looked like it controlled reachability.
   */
}

export interface Scenario {
  id: string;
  label: string;
  /** Why this scenario exists — shown in the harness panel. */
  note: string;
  state: MockState;
  behavior: MockBehavior;
}

function settings(over: Partial<Settings> = {}): Settings {
  return {
    nickname: LOCAL_NICKNAME,
    input_device: null,
    output_device: null,
    input_gain_db: 6,
    noise_suppression: true,
    loopback: false,
    debug_wavs: false,
    hotkeys: { mute: "Ctrl+Shift+M", deafen: "Ctrl+Shift+D", ptt: "" },
    signaling_port: 49860,
    vps_address: DEFAULT_VPS,
    transport_mode: "auto",
    host_port: 8444,
    voice_bitrate_kbps: 32,
    music_bitrate_kbps: 96,
    // Session 29b Force Stop: `Settings::default()` has this on, so the fixture
    // does too — a mock defaulting to `false` would hide the feature in every
    // screenshot and every browser check.
    music_force_stop: true,
    ...over,
  };
}

function audio(over: Partial<AudioState> = {}): AudioState {
  return {
    muted: false,
    deafened: false,
    ptt_mode: false,
    ptt_held: false,
    speaking: false,
    loopback: false,
    ...over,
  };
}

/**
 * Session 29: the local user's persistent id, matching `LOCAL_NICKNAME`. It is
 * what the UI compares a room mute's `target_user_id` against to decide whether
 * it owes an explanation for being silent.
 */
const SELF_USER_ID = "user-kaveh";

function room(over: Partial<RoomSnapshot> = {}): RoomSnapshot {
  return {
    state: "idle",
    peer_name: null,
    peers: [],
    room_code: null,
    direct_codes: [],
    signaling_port: 49860,
    vps_address: DEFAULT_VPS,
    transport_mode: "auto",
    diagnostics: null,
    mutes: [],
    self_user_id: SELF_USER_ID,
    ...over,
  };
}

function music(over: Partial<MusicStatus> = {}): MusicStatus {
  return {
    active: false,
    playing: false,
    path: null,
    title: null,
    duration_ms: 0,
    position_ms: 0,
    volume: 1,
    loop: false,
    monitor: false,
    queue_index: null,
    queue_len: 0,
    queue_looping: false,
    queue_finished: false,
    ...over,
  };
}

/** A queue entry. Metadata mirrors what `describe_track` resolves from a file. */
function queueItem(over: Partial<QueueItem> = {}): QueueItem {
  return {
    path: "D:\\Music\\Tycho - Awake\\01 Awake.flac",
    title: "Awake",
    duration_ms: 272_000,
    ...over,
  };
}

/**
 * Connected, healthy P2P session. Mirrors the shape a real in-call
 * `get_transport_diagnostics` returns: an active mode, a remote endpoint, and
 * monotonically growing counters. No jitter/loss fields exist to populate.
 */
function diag(over: Partial<TransportDiagnostics> = {}): TransportDiagnostics {
  return {
    mode: "direct_p2p",
    remote_endpoint: "85.198.44.17:51844",
    local_addr: "192.168.1.24:51844",
    packets_sent: 18422,
    packets_recv: 18390,
    bytes_sent: 2643640,
    bytes_recv: 2635808,
    last_recv_ms_ago: 18,
    rtt_ms: 14,
    ice_state: "connected",
    status: "connected",
    last_error: null,
    relay_packets_sent: 0,
    relay_packets_recv: 0,
    relay_bytes_sent: 0,
    relay_bytes_recv: 0,
    relay_mode: "inactive",
    ...over,
  };
}

/** Relay is down: no endpoint, no RTT, a real error string, zeroed counters. */
function diagOffline(): TransportDiagnostics {
  return {
    mode: "cloud_relay",
    remote_endpoint: null,
    local_addr: null,
    packets_sent: 0,
    packets_recv: 0,
    bytes_sent: 0,
    bytes_recv: 0,
    last_recv_ms_ago: null,
    rtt_ms: null,
    ice_state: null,
    status: "offline",
    last_error: "failed to resolve VPS relay address for relay.verio.internal",
    relay_packets_sent: 0,
    relay_packets_recv: 0,
    relay_bytes_sent: 0,
    relay_bytes_recv: 0,
    relay_mode: "offline",
  };
}

const MUSIC_TRACK = {
  path: "D:\\Music\\Tycho - Awake\\01 Awake.flac",
  title: "Awake",
  duration_ms: 272_000,
};

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

export const SCENARIOS: Scenario[] = [
  // ---- Lobby -------------------------------------------------------------
  {
    id: "lobby-fresh",
    label: "Lobby — fresh install",
    note: "Identity unset, idle room. Exercises the empty/disabled state: create and join must not be reachable-looking.",
    state: {
      settings: settings({ nickname: "", input_device: null, output_device: null }),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "lobby-ready",
    label: "Lobby — ready to connect",
    note: "Normal pre-call state with a nickname set and devices enumerated.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -42 },
    },
  },
  {
    id: "lobby-no-devices",
    label: "Lobby — no audio devices",
    note: "Device enumeration returned nothing. Exercises the empty picker state.",
    state: {
      settings: settings({ input_device: null, output_device: null }),
      devices: NO_DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "lobby-invalid-code",
    label: "Lobby — join fails (bad code)",
    note: "Typing any code and pressing Join rejects with a relay-side error. Drives the error presentation.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: { join_room: `Room #${ROOM_CODE} not found on relay.` },
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "lobby-creating",
    label: "Lobby — creating room (slow)",
    note: "create_room takes 2.5 s, so the in-progress state is observable. Also drives the double-submit guard.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 2500 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "relay-unreachable",
    label: "Lobby — relay unreachable",
    note: "Both create and join fail with the real unreachable-relay error string.",
    state: {
      settings: settings({ vps_address: "wss://relay.verio.internal:7443" }),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 1200 },
      failures: {
        create_room:
          "failed to resolve VPS relay address for relay.verio.internal",
        join_room:
          "failed to resolve VPS relay address for relay.verio.internal",
      },
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "settings-slow",
    label: "Loading — settings not yet loaded",
    note: "get_settings takes 1.5 s, so the app renders its loading/empty shell first.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 1500, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },

  // ---- Room --------------------------------------------------------------
  {
    id: "joining",
    label: "Room — joining handshake",
    note: "room.state = connecting with a code already known. The handshake/in-progress presentation.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connecting",
        room_code: ROOM_CODE,
        peers: [],
        diagnostics: diag({
          mode: "auto",
          remote_endpoint: null,
          ice_state: "checking",
          status: "connecting",
          rtt_ms: null,
          last_recv_ms_ago: null,
          packets_sent: 0,
          packets_recv: 0,
          bytes_sent: 0,
          bytes_recv: 0,
        }),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "room-1",
    label: "Room — 1 participant (you)",
    note: "Smallest supported room. Nothing else should be implied.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio({ speaking: false }),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: [],
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -34 },
    },
  },
  {
    id: "room-3",
    label: "Room — 3 participants",
    note: "Typical small group. Baseline density check.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2, { 0: { speaking: true } }),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -30 },
    },
  },
  {
    id: "room-remote-music",
    label: "Room — somebody else is hosting music",
    note: "A bot peer that is not ours, accepting a remote stop. The listener's Stop control is enabled here.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: [...roster(2), remoteBotPeer(0, true)],
        diagnostics: diag(),
      }),
      // Session 29c: the listener's OWN player, loaded and paused, with a queue of
      // its own — deliberately not idle. With a track loaded and three rows queued,
      // every transport control would be enabled, so D6's assertions that they are
      // *disabled* are about somebody else's music rather than about an empty
      // player, which would satisfy the same assertions for the wrong reason.
      //
      // The local track is "Montana" while the room's is "Awake", so a panel that
      // showed the wrong one is caught rather than coincidentally matching.
      music: music({
        active: false,
        playing: false,
        path: "D:\\Music\\Tycho - Awake\\02 Montana.flac",
        title: "Montana",
        duration_ms: 321_000,
        monitor: true,
        queue_index: 0,
        queue_len: 3,
      }),
      queue: [
        queueItem({
          path: "D:\\Music\\Tycho - Awake\\02 Montana.flac",
          title: "Montana",
          duration_ms: 321_000,
        }),
        queueItem({
          path: "D:\\Music\\Tycho - Awake\\03 L.flac",
          title: "L",
          duration_ms: 279_000,
        }),
        queueItem({
          path: "D:\\Music\\Tycho - Awake\\04 A Walk.flac",
          title: "A Walk",
          duration_ms: 305_000,
        }),
      ],
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -30 },
    },
  },
  {
    id: "room-remote-music-no-force",
    label: "Room — the host disabled force-stop",
    note: "The same room, with the bot refusing a remote stop. The Stop control must be present but disabled, with the host's reason on its tooltip.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: [...roster(2), remoteBotPeer(0, false)],
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -30 },
    },
  },
  {
    id: "room-6",
    label: "Room — 6 participants",
    note: "Mid-size group; the layout should still fit without a scrolling participant list.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(5, {
          0: { speaking: true },
          2: { mute: true },
        }),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -28 },
    },
  },
  {
    id: "room-10",
    label: "Room — 10 participants (max)",
    note: "Maximum supported group, every state present at once. The density and no-scroll requirement.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio({ speaking: true }),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(9, {
          0: { speaking: true },
          1: { mute: true },
          2: { deafen: true },
          3: { mute: true, deafen: true },
          4: { music: true, music_title: "Awake" },
          5: { speaking: true },
          6: { mute: true },
        }),
        diagnostics: diag({ mode: "cloud_relay", relay_mode: "active" }),
      }),
      music: music({
        active: true,
        playing: true,
        path: MUSIC_TRACK.path,
        title: MUSIC_TRACK.title,
        duration_ms: MUSIC_TRACK.duration_ms,
        position_ms: 96_000,
        monitor: true,
      }),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -24 },
    },
  },
  {
    id: "room-self-muted",
    label: "Room — you are muted + deafened",
    note: "Local mute/deafen presentation together with a healthy room.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio({ muted: true, deafened: true }),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(3, { 1: { speaking: true } }),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "room-room-muted-self",
    label: "Room — the room silenced you",
    note:
      "Session 29: a room-wide mute whose target is the local user. This is the one " +
      "combination that renders the banner, and it has to name the actor — the " +
      "user's own mic still works, so nothing else in the UI would explain the " +
      "silence.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(3),
        mutes: [{ target_user_id: SELF_USER_ID, by: "Dan" }],
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "room-ptt",
    label: "Room — push-to-talk held",
    note: "ptt_mode + ptt_held. A distinct self state that must be visually separable from mute.",
    state: {
      settings: settings({
        hotkeys: { mute: "Ctrl+Shift+M", deafen: "Ctrl+Shift+D", ptt: "Mouse4" },
      }),
      devices: DEVICES,
      audio: audio({ ptt_mode: true, ptt_held: true, speaking: true }),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -26 },
    },
  },
  {
    id: "room-degraded",
    label: "Room — relay path, no RTT yet",
    note: "Relay mode with an empty endpoint and no measurement. Exercises honest 'unknown' rendering instead of a fake number.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(4, { 0: { speaking: true } }),
        diagnostics: diag({
          mode: "cloud_relay",
          remote_endpoint: null,
          local_addr: "192.168.1.24:51844",
          rtt_ms: null,
          last_recv_ms_ago: null,
          relay_mode: "active",
          relay_packets_sent: 9210,
          relay_packets_recv: 9188,
          relay_bytes_sent: 1321720,
          relay_bytes_recv: 1318808,
          status: "relaying",
        }),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -38 },
    },
  },
  {
    id: "room-connection-lost",
    label: "Room — connection lost",
    note: "The real ICE-loss error string, emitted as a room_error event on boot.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(1),
        diagnostics: diagOffline(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
      // The real string, from crates/verio-transport/src/lib.rs:888. Before
      // Session 24 the note above claimed this was emitted on boot and it was
      // not — the harness panel's button was the only emitter, so the scenario
      // named "connection lost" showed no banner. Now it does.
      roomError: "ice connection lost (grace period expired)",
    },
  },
  {
    id: "edge-long-strings",
    label: "Edge — long name / long address",
    note: "Overflow behaviour: long peer name, long relay address, long device name, long music title.",
    state: {
      settings: settings({
        nickname: "Kaveh",
        input_device: "Shure SM7B (Elgato Wave XLR) — Channel 1/2 Stereo Pair",
        output_device: "Schiit Hel 2E DAC/Amp (Exclusive Mode, Low Gain)",
        vps_address:
          "wss://relay-eu-central-1.verio.internal:7443/signaling/websocket",
      }),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: [
          peer(0, { name: "Aleksandr_the_Unstoppable" }),
          peer(1, { name: "Bartholomew" }),
        ],
        diagnostics: diag({
          remote_endpoint: "2606:4700:4700::1111:51844",
          local_addr: "fe80::a4b2:c3d4:e5f6:7890%12:51844",
        }),
      }),
      music: music({
        active: true,
        playing: true,
        path: "D:\\Music\\Tycho - Awake (10th Anniversary Deluxe Edition)\\01 Awake (Extended Mix).flac",
        title: "Awake (Extended Mix)",
        duration_ms: 512_000,
        position_ms: 128_000,
      }),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -32 },
    },
  },

  // ---- Music -------------------------------------------------------------
  {
    id: "music-idle",
    label: "Music — no track loaded",
    note: "Idle bus. Must not show a queue, artwork, spectrum, or a fake buffered bar.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -44 },
    },
  },
  {
    id: "music-playing",
    label: "Music — playing into the room",
    note: "Playing, streaming, local monitor on. Volume at 140% to exercise the >100% range.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(3, { 2: { music: true, music_title: "Awake" } }),
        diagnostics: diag(),
      }),
      music: music({
        active: true,
        playing: true,
        path: MUSIC_TRACK.path,
        title: MUSIC_TRACK.title,
        duration_ms: MUSIC_TRACK.duration_ms,
        position_ms: 96_000,
        volume: 1.4,
        monitor: true,
        queue_index: 0,
        queue_len: 3,
        queue_looping: false,
      }),
      // Session 20: a real queue, so the panel's list and the current-entry
      // marker can actually be exercised.
      queue: [
        queueItem({
          path: MUSIC_TRACK.path,
          title: MUSIC_TRACK.title,
          duration_ms: MUSIC_TRACK.duration_ms,
        }),
        queueItem({
          path: "D:\\Music\\Tycho - Awake\\02 Montana.flac",
          title: "Montana",
          duration_ms: 321_000,
        }),
        queueItem({
          path: "D:\\Music\\Tycho - Awake\\03 L.flac",
          title: "L",
          duration_ms: 279_000,
        }),
      ],
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -46 },
    },
  },
  {
    id: "music-paused",
    label: "Music — paused",
    note: "Loaded but not playing. Monitor off.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2),
        diagnostics: diag(),
      }),
      music: music({
        active: true,
        playing: false,
        path: MUSIC_TRACK.path,
        title: MUSIC_TRACK.title,
        duration_ms: MUSIC_TRACK.duration_ms,
        position_ms: 96_000,
        loop: true,
      }),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -46 },
    },
  },
  {
    id: "music-opening",
    label: "Music — opening a file (slow)",
    note: "music_open takes 1.5 s. The real in-between state: active, not yet playing, no title or duration.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2),
        diagnostics: diag(),
      }),
      music: music({ active: true, playing: false }),
    },
    behavior: {
      latency: { read: 20, write: 1500 },
      failures: {},
      micLevel: { enabled: true, db: -46 },
    },
  },
  {
    id: "music-error",
    label: "Music — open fails",
    note: "music_open rejects with a decoder-shaped error, matching verio-dsp's real message format.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio(),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 400 },
      failures: {
        music_open: "no decoder for this codec: unsupported codec parameters",
      },
      micLevel: { enabled: true, db: -46 },
    },
  },
  {
    id: "audio-error",
    label: "Error — audio engine failed",
    note: "The real `audio_error` event (currently unhandled by the app) with a plausible pipeline message.",
    state: {
      settings: settings(),
      devices: DEVICES,
      audio: audio({ muted: true }),
      room: room({
        state: "connected",
        room_code: ROOM_CODE,
        peers: roster(2),
        diagnostics: diag(),
      }),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
      audioError: "spawn audio pump failed: no default output device",
    },
  },

  // ---- Settings ----------------------------------------------------------
  {
    id: "settings-normal",
    label: "Settings — normal",
    note: "Every real setting populated with plausible values.",
    state: {
      settings: settings({
        input_device: "Shure SM7B (Elgato Wave XLR)",
        output_device: "Schiit Hel 2E DAC/Amp",
        input_gain_db: 9,
        noise_suppression: true,
        loopback: false,
        debug_wavs: false,
      }),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music({ monitor: true }),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: true, db: -40 },
    },
  },
  {
    id: "settings-empty-nickname",
    label: "Settings — blank nickname",
    note: "set_nickname accepts an empty value in Rust. The UI is free to warn; it must not claim a failure that does not happen.",
    state: {
      settings: settings({ nickname: "" }),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 30 },
      failures: {},
      micLevel: { enabled: false, db: -60 },
    },
  },
  {
    id: "settings-validation-error",
    label: "Settings — save rejected",
    note: "set_vps_address rejects with the real Rust message. Drives inline validation presentation.",
    state: {
      settings: settings({ vps_address: DEFAULT_VPS }),
      devices: DEVICES,
      audio: audio(),
      room: room(),
      music: music(),
    },
    behavior: {
      latency: { read: 20, write: 300 },
      failures: { set_vps_address: "VPS address cannot be empty." },
      micLevel: { enabled: false, db: -60 },
    },
  },
];

export const DEFAULT_SCENARIO_ID = "lobby-ready";

export function getScenario(id: string | null | undefined): Scenario {
  return (
    SCENARIOS.find((s) => s.id === id) ??
    SCENARIOS.find((s) => s.id === DEFAULT_SCENARIO_ID) ??
    SCENARIOS[0]
  );
}

/** Structured clone so a scenario's pristine state can be restored on reset. */
export function cloneState(state: MockState): MockState {
  return JSON.parse(JSON.stringify(state)) as MockState;
}
