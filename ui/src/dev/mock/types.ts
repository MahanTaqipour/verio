/**
 * Wire-shape mirrors of the Rust types exposed over Tauri IPC.
 *
 * These deliberately do NOT import from `App.tsx`. The harness must stay
 * decoupled from application internals so it can never influence them, and so
 * a change to the app's private types cannot silently change the fixtures.
 *
 * Source of truth for every field below:
 *   ui/src-tauri/src/lib.rs        (RoomSnapshot, AudioState, DeviceList, Settings)
 *   crates/verio-app/src/room.rs   (PeerInfo)
 *   crates/verio-app/src/settings.rs
 *   crates/verio-app/src/pipeline.rs (MusicStatus)
 *   crates/verio-transport         (TransportDiagnostics)
 */

export interface Hotkeys {
  mute: string;
  deafen: string;
  ptt: string;
}

export type TransportModeSetting =
  | "auto"
  | "cloud_relay"
  | "peer_host"
  | "direct_p2p";

export interface Settings {
  nickname: string;
  input_device: string | null;
  output_device: string | null;
  input_gain_db: number;
  noise_suppression: boolean;
  loopback: boolean;
  debug_wavs: boolean;
  hotkeys: Hotkeys;
  signaling_port: number;
  vps_address: string;
  transport_mode: TransportModeSetting;
  host_port: number;
  voice_bitrate_kbps: number;
  music_bitrate_kbps: number;
  /**
   * Session 29b Force Stop: may other peers stop the music this user hosts?
   * Defaults to `true` — `Settings::default()` in the Rust — and is applied when
   * the user becomes host, not while music is already playing.
   */
  music_force_stop: boolean;
}

export interface DeviceEntry {
  name: string;
  is_default: boolean;
}

export interface DeviceList {
  inputs: DeviceEntry[];
  outputs: DeviceEntry[];
}

export interface AudioState {
  muted: boolean;
  deafened: boolean;
  ptt_mode: boolean;
  ptt_held: boolean;
  speaking: boolean;
  loopback: boolean;
}

export type RoomState = "idle" | "connecting" | "connected" | "closed";

export interface PeerInfo {
  uuid: string;
  /**
   * Session 29: the peer's PERSISTENT id. A room-wide mute is keyed on this, not
   * on `uuid` — a uuid is minted per process, so a mute against it would stop
   * matching the same person the next time they launched. Empty until their
   * `Hello` reaches us.
   */
  user_id: string;
  name: string;
  speaking: boolean;
  mute: boolean;
  deafen: boolean;
  /** Session 14: this peer is playing music into the room. */
  music: boolean;
  music_title: string;
  /**
   * Session 29b: this peer is a music bot session, not a person. Declared by the
   * peer itself; never inferred, and never withdrawn.
   */
  is_bot: boolean;
  /**
   * Session 29b Force Stop: this bot accepts a remote stop. Meaningful only when
   * `is_bot`, and only ever `true` for a peer that declared it on the wire — which
   * is why the mock sets it from the bot's own start, not from a fixture default.
   */
  force_stop: boolean;
}

export interface MusicStatus {
  active: boolean;
  playing: boolean;
  path: string | null;
  title: string | null;
  duration_ms: number;
  position_ms: number;
  volume: number;
  loop: boolean;
  monitor: boolean;
  /** Session 20: queue shape. The contents come from `music_queue_list`. */
  queue_index: number | null;
  queue_len: number;
  queue_looping: boolean;
  /**
   * Session 29b: the queue ran out, as opposed to being paused. Both leave
   * `playing` false, and the bot session survives one of them and not the other.
   * Carried so this mirror stays a mirror; nothing in the UI reads it.
   */
  queue_finished: boolean;
}

/** Session 29b: mirrors `BotState` in ui/src-tauri/src/lib.rs. */
export interface MusicBotState {
  hosting: boolean;
  session_id: string | null;
  host_name: string | null;
}

/** Mirrors `verio_app::pipeline::QueueItem`. */
export interface QueueItem {
  path: string;
  title: string;
  duration_ms: number;
}

export interface TransportDiagnostics {
  mode: string;
  endpoint: string | null;
  local_addr: string | null;
  packets_sent: number;
  packets_recv: number;
  bytes_sent: number;
  bytes_recv: number;
  last_recv_ms_ago: number | null;
  rtt_ms: number | null;
  ice_state: string | null;
  status: string;
  error: string | null;
  relay_packets_sent: number;
  relay_packets_recv: number;
  relay_bytes_sent: number;
  relay_bytes_recv: number;
  relay_mode: string;
}

/** Session 29: one room-wide mute. Mirrors `verio_app::room::MuteInfo`. */
export interface MuteInfo {
  /** Match against `PeerInfo.user_id`, never against `PeerInfo.uuid`. */
  target_user_id: string;
  /** Who set it. Empty when unknown, e.g. restored from disk. */
  by: string;
}

export interface RoomSnapshot {
  state: RoomState;
  peer_name: string | null;
  peers: PeerInfo[];
  room_code: string | null;
  direct_codes: string[];
  signaling_port: number;
  vps_address: string;
  transport_mode: string | null;
  diagnostics: TransportDiagnostics | null;
  /** Session 29: the room-wide mutes, so a reload does not lose them. */
  mutes: MuteInfo[];
  /** Session 29: this user's own persistent id. */
  self_user_id: string;
}
