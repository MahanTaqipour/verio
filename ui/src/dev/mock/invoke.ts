/**
 * Mock Tauri transport for the Verio dev harness.
 *
 * How it works
 * ------------
 * `@tauri-apps/api/mocks#mockIPC` installs `window.__TAURI_INTERNALS__.invoke`,
 * which is exactly what the real `invoke()` in `@tauri-apps/api/core` calls. So
 * the application's own `invoke`/`listen` code paths are exercised unchanged —
 * only the transport behind them is fake. `App.tsx` is not modified.
 *
 * Why the event plugin is implemented here instead of using
 * `mockIPC(..., { shouldMockEvents: true })`:
 * Tauri's built-in event mock removes listeners by `args.id`, but the real
 * `_unlisten` sends `{ event, eventId }`. The ids therefore never match, and
 * every event fires a stale listener that logs
 * "Couldn't find callback id ...". At the 20 Hz `input_level` rate that floods
 * the console. This implementation removes by `eventId` and delivers the real
 * event object shape `{ event, id, payload }`.
 */

import { mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import {
  DEFAULT_SCENARIO_ID,
  cloneState,
  getScenario,
  type MockState,
  type Scenario,
} from "./fixtures";
import type { MusicBotState, QueueItem } from "./types";

// ---------------------------------------------------------------------------
// window.__TAURI_INTERNALS__ typing
// ---------------------------------------------------------------------------

interface TauriInternals {
  invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
  transformCallback?: (cb: (data: unknown) => void, once?: boolean) => number;
  unregisterCallback?: (id: number) => void;
  runCallback?: (id: number, data: unknown) => void;
  callbacks?: Map<number, (data: unknown) => void>;
  metadata?: unknown;
}

function internals(): TauriInternals {
  return (window as unknown as { __TAURI_INTERNALS__: TauriInternals })
    .__TAURI_INTERNALS__;
}

// ---------------------------------------------------------------------------
// Public surface used by MockHarness.tsx
// ---------------------------------------------------------------------------

export interface CommandLogEntry {
  cmd: string;
  args: Record<string, unknown> | undefined;
  at: number;
}

export interface MockTransport {
  /** The scenario currently installed. */
  readonly scenario: Scenario;
  /** Live mutable state. Command handlers read and write this. */
  readonly state: MockState;
  /** Most recent commands first. Capped. */
  readonly log: CommandLogEntry[];
  /** Push an event into the app exactly as Rust would. */
  emitEvent(event: string, payload?: unknown): void;
  /** Live, no-reload controls for driving state transitions. */
  addRandomPeer(): void;
  removeLastPeer(): void;
  toggleRandomPeerSpeaking(): void;
  emitRoomError(message: string): void;
  emitAudioError(message: string): void;
  setMusicPlaying(playing: boolean): void;
  /**
   * Session 29b: start or end the music bot session.
   *
   * Routed through the real command path rather than poking `state`, so the harness
   * exercises the refusals too — a control that bypassed them would let the harness
   * show a state the app can never reach.
   */
  toggleMusicBot(): void;
  /** Turn the synthetic mic level stream on or off at runtime. */
  setMicLevel(enabled: boolean): void;
  /** Emit a short input_level sweep to exercise the imperative meter. */
  pulseMicLevel(): void;
  /** Count of commands handled, for a quick sanity read. */
  readonly commandCount: number;
}

const MAX_LOG = 60;

/**
 * Session 29b: a client that is not hosting. Matches what `music_bot_state`
 * returns from Rust for a client that has never pressed Play.
 */
const IDLE_BOT: MusicBotState = {
  hosting: false,
  session_id: null,
  host_name: null,
};

/**
 * Session 29b: the bot session's wire id.
 *
 * Fixed rather than generated, so a scenario that wants to name the bot peer in a
 * fixture cannot drift from the one `music_bot_start` creates — and so the Music
 * tile keeps its identity when the harness re-clones state.
 */
const BOT_UUID = "b07b07b0-1234-4abc-8def-000000000b07";

// ---------------------------------------------------------------------------
// Install
// ---------------------------------------------------------------------------

export function installMockTransport(scenarioId: string): MockTransport {
  const scenario = getScenario(scenarioId);
  const state: MockState = cloneState(scenario.state);
  const failures: Record<string, string> = { ...scenario.behavior.failures };

  // Session 20: the play queue, held apart from `state.music`. The real backend
  // reports only the queue's *shape* in MusicStatus; the contents arrive from
  // `music_queue_list` and the `music_queue_changed` event. A scenario that omits
  // `queue` starts empty, which is what exercises the panel's EmptyState.
  const queue: QueueItem[] = state.queue ? structuredClone(state.queue) : [];

  /** Mirror the queue's shape into MusicStatus, as the real backend does. */
  function syncQueueShape(): void {
    state.music.queue_len = queue.length;
    const idx =
      state.music.path === null
        ? -1
        : queue.findIndex((q) => q.path === state.music.path);
    state.music.queue_index = idx >= 0 ? idx : null;
  }

  /**
   * Derive `active` the way the backend does, then emit.
   *
   * Rust computes this in one place — `active` is `decoder.is_some() &&
   * g.playing` in `MusicSource::status`, and `MusicSource::is_active` is the
   * same expression. The mock used to hand-set the field at each call site, and
   * three sites disagreed with the backend: `music_open` claimed `active: true`
   * while `open()` leaves `playing = false`; `music_play` never set it; and
   * `music_pause` left it true. A harness reporting a state the app cannot reach
   * makes a correct UI look broken — the music panel's chip reads `active` to
   * decide whether to claim it is sending to the room.
   *
   * `path !== null` stands in for "a decoder is loaded", which is what `open`
   * establishes and `stop` clears.
   */
  function emitMusicStatus(): void {
    state.music.active = state.music.path !== null && state.music.playing;
    void emit("music_status_changed", structuredClone(state.music));
  }

  function emitQueue(): void {
    void emit("music_queue_changed", {
      items: structuredClone(queue),
      index: state.music.queue_index,
    });
  }

  syncQueueShape();

  const log: CommandLogEntry[] = [];
  let commandCount = 0;

  // --- event plugin -------------------------------------------------------
  const listeners = new Map<string, number[]>();

  function dispatch(event: string, payload: unknown): void {
    const ids = listeners.get(event);
    if (!ids || ids.length === 0) return;
    const api = internals();
    for (const id of ids) {
      // Real Tauri delivers `{ event, id, payload }`.
      api.runCallback?.(id, { event, id, payload });
    }
  }

  function handleEventPlugin(
    cmd: string,
    args: Record<string, unknown>,
  ): unknown {
    switch (cmd) {
      case "plugin:event|listen": {
        const event = String(args.event);
        const id = Number(args.handler);
        const ids = listeners.get(event) ?? [];
        ids.push(id);
        listeners.set(event, ids);
        return id;
      }
      case "plugin:event|unlisten": {
        const event = String(args.event);
        const id = Number(args.eventId);
        const ids = listeners.get(event);
        if (ids) {
          const next = ids.filter((x) => x !== id);
          if (next.length) listeners.set(event, next);
          else listeners.delete(event);
        }
        return null;
      }
      case "plugin:event|emit":
      case "plugin:event|emit_to": {
        dispatch(String(args.event), args.payload);
        return null;
      }
      default:
        return null;
    }
  }

  // --- timers -------------------------------------------------------------
  const timers: number[] = [];
  let micTimer: number | null = null;
  let diagTimer: number | null = null;

  function startMicLevel(): void {
    stopMicLevel();
    const enabled = scenario.behavior.micLevel.enabled;
    let phase = 0;
    micTimer = window.setInterval(() => {
      phase += 1;
      let db: number;
      if (!enabled) {
        db = -60;
      } else {
        // Speech-like envelope: a slow syllable oscillation with small jitter.
        const base = scenario.behavior.micLevel.db;
        const syllable = Math.abs(Math.sin(phase / 7));
        const jitter = (Math.random() - 0.5) * 3;
        db = base + syllable * 14 + jitter;
      }
      void emit("input_level", db);
    }, 50);
    timers.push(micTimer);
  }

  function stopMicLevel(): void {
    if (micTimer !== null) {
      window.clearInterval(micTimer);
      micTimer = null;
    }
  }

  /**
   * Advance the diagnostics counters the way the real transport would: frames
   * arrive continuously, so packet and byte counters grow and the receive age
   * resets. Only fields that exist on TransportDiagnostics are touched.
   */
  function startDiagnosticsTicker(): void {
    diagTimer = window.setInterval(() => {
      const d = state.room.diagnostics;
      if (!d) return;
      if (d.status !== "connected" && d.status !== "relaying") {
        if (d.last_recv_ms_ago !== null) d.last_recv_ms_ago += 1000;
        return;
      }
      const viaRelay = d.relay_mode === "active";
      d.packets_sent += 100;
      d.packets_recv += 100;
      d.bytes_sent += viaRelay ? 12_400 : 5_200;
      d.bytes_recv += viaRelay ? 12_300 : 5_180;
      d.last_recv_ms_ago = 10 + Math.floor(Math.random() * 20);
      if (d.rtt_ms !== null) {
        const base = scenario.id === "room-degraded" ? 48 : 14;
        d.rtt_ms = Math.max(1, Math.round(base + (Math.random() - 0.5) * 4));
      }
      if (viaRelay) {
        d.relay_packets_sent += 100;
        d.relay_packets_recv += 100;
        d.relay_bytes_sent += 12_400;
        d.relay_bytes_recv += 12_300;
      }
    }, 1000);
    timers.push(diagTimer);
  }

  // --- command dispatch ---------------------------------------------------
  const sleep = (ms: number) => new Promise((r) => window.setTimeout(r, ms));

  /** Rust commands return `Err(String)`, which Tauri rejects with the raw string. */
  function fail(msg: string): never {
    throw msg;
  }

  async function handleCommand(
    cmd: string,
    args: Record<string, unknown> = {},
  ): Promise<unknown> {
    commandCount += 1;
    log.unshift({ cmd, args, at: Date.now() });
    if (log.length > MAX_LOG) log.pop();

    const forced = failures[cmd];
    const isWrite = cmd.startsWith("set_") || cmd.startsWith("music_") ||
      cmd === "toggle_mute" || cmd === "toggle_deafen" ||
      cmd === "create_room" || cmd === "join_room" || cmd === "leave_room" ||
      cmd === "connect_direct";
    await sleep(
      isWrite ? scenario.behavior.latency.write : scenario.behavior.latency.read,
    );
    if (forced) fail(forced);

    switch (cmd) {
      // --- reads ---
      case "get_settings":
        return structuredClone(state.settings);
      case "list_devices":
        return structuredClone(state.devices);
      case "get_audio_state":
        return structuredClone(state.audio);
      case "get_room_state":
        return {
          state: state.room.state,
          peer_name: state.room.peer_name,
          peers: structuredClone(state.room.peers),
          room_code: state.room.room_code,
          direct_codes: structuredClone(state.room.direct_codes),
          signaling_port: state.settings.signaling_port,
          vps_address: state.settings.vps_address,
          transport_mode: state.settings.transport_mode,
          diagnostics: state.room.diagnostics
            ? structuredClone(state.room.diagnostics)
            : null,
          mutes: structuredClone(state.room.mutes),
          self_user_id: state.room.self_user_id,
        };
      case "get_transport_diagnostics":
        return state.room.diagnostics
          ? structuredClone(state.room.diagnostics)
          : null;
      case "music_status":
        return structuredClone(state.music);
      case "ping":
        return {
          message: `Hello ${String(args.name ?? "")}! The Rust ↔ frontend bridge is working.`,
          rust_timestamp_ms: Date.now(),
          app: { name: "Verio", version: "0.1.0" },
        };

      // --- audio controls ---
      case "toggle_mute":
        state.audio.muted = !state.audio.muted;
        void emit("audio_state_changed", structuredClone(state.audio));
        return structuredClone(state.audio);
      case "toggle_deafen":
        state.audio.deafened = !state.audio.deafened;
        // Rust deafen also forces mute off/on in tandem; keep the two coherent.
        if (state.audio.deafened) state.audio.muted = true;
        void emit("audio_state_changed", structuredClone(state.audio));
        return structuredClone(state.audio);
      case "set_loopback":
        state.settings.loopback = Boolean(args.on);
        state.audio.loopback = Boolean(args.on);
        void emit("audio_state_changed", structuredClone(state.audio));
        return null;

      // --- settings writes ---
      case "set_nickname": {
        const raw = String(args.name ?? args.nickname ?? "");
        state.settings.nickname = raw.trim();
        return null;
      }
      case "set_input_device":
        state.settings.input_device = (args.name as string | null) ?? null;
        return null;
      case "set_output_device":
        state.settings.output_device = (args.name as string | null) ?? null;
        return null;
      case "set_input_gain":
        state.settings.input_gain_db = Math.min(
          12,
          Math.max(-12, Number(args.gainDb ?? 0)),
        );
        return null;
      case "set_noise_suppression":
        state.settings.noise_suppression = Boolean(args.on);
        return null;
      case "set_debug_wavs":
        state.settings.debug_wavs = Boolean(args.on);
        return null;
      case "set_hotkey": {
        const action = String(args.action ?? args.field ?? "");
        const combo = String(args.combo ?? "");
        if (action === "mute" || action === "deafen" || action === "ptt") {
          state.settings.hotkeys[action] = combo;
        }
        return null;
      }
      case "set_voice_quality":
        state.settings.voice_bitrate_kbps = Number(args.kbps ?? 48);
        return null;
      case "set_music_quality":
        state.settings.music_bitrate_kbps = Number(args.kbps ?? 128);
        return null;
      case "set_vps_address":
        state.settings.vps_address = String(args.address ?? "").trim();
        state.room.vps_address = state.settings.vps_address;
        return null;
      case "set_peer_volume":
        // The pipeline keeps per-peer gain internally and exposes no getter, so
        // there is nothing to mirror into the snapshot. Accepted and ignored.
        return null;
      case "set_peer_muted": {
        // Session 29. Room state, keyed on the persistent id — and the change is
        // echoed on `peer_muted` to everyone including the caller, which is what
        // the real room does. A handler that only mutated `state.room.mutes`
        // would leave the UI waiting for an event that never came.
        // `targetUserId`, not `target_user_id`: Tauri v2 renames command arguments
        // to camelCase on the JS side, which is why `set_input_gain(gain_db)` is
        // invoked as `{ gainDb }` elsewhere in this app.
        const target = String(args.targetUserId ?? "").trim();
        if (!target) {
          // The real command refuses this too (`set_room_mute`).
          throw new Error("cannot mute a peer whose persistent id is unknown");
        }
        const muted = Boolean(args.muted);
        const by = String(state.settings.nickname || "Player");
        const at = state.room.mutes.findIndex((m) => m.target_user_id === target);
        if (muted) {
          if (at >= 0) {
            state.room.mutes[at].by = by;
          } else {
            state.room.mutes.push({ target_user_id: target, by });
          }
        } else if (at >= 0) {
          state.room.mutes.splice(at, 1);
        }
        void emit("peer_muted", { target_user_id: target, muted, by });
        return null;
      }

      // --- room ---
      case "create_room": {
        state.room.state = "connecting";
        void emit("room_state", "connecting");
        await sleep(300);
        const code = "7391";
        state.room.state = "connected";
        state.room.room_code = code;
        state.room.diagnostics = structuredClone(
          getScenario("room-1").state.room.diagnostics,
        );
        void emit("room_created", { code });
        void emit("room_state", "connected");
        return code;
      }
      case "join_room": {
        const code = String(args.code ?? "");
        state.room.state = "connecting";
        state.room.room_code = code;
        void emit("room_state", "connecting");
        await sleep(600);
        state.room.state = "connected";
        state.room.peers = structuredClone(getScenario("room-3").state.room.peers);
        state.room.diagnostics = structuredClone(
          getScenario("room-3").state.room.diagnostics,
        );
        void emit("room_joined", { code });
        void emit("room_state", "connected");
        return null;
      }
      case "leave_room": {
        state.room.state = "idle";
        state.room.room_code = null;
        state.room.peers = [];
        state.room.diagnostics = null;
        void emit("room_state", "idle");
        return null;
      }
      case "connect_direct":
        // Not reachable from the shipped UI; kept so the command surface matches.
        return null;

      // --- music ---
      case "music_pick_file":
        return "D:\\Music\\Tycho - Awake\\01 Awake.flac";
      case "music_open": {
        const path = String(args.path ?? "");
        const title = path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? path;
        state.music = {
          ...state.music,
          playing: false,
          path,
          title,
          duration_ms: 272_000,
          position_ms: 0,
          // A directly-opened file is not a queue member. Mirrors
          // `MusicSource::open`, which clears the index for the same reason.
          queue_index: null,
          queue_len: queue.length,
        };
        emitMusicStatus();
        // Real MusicInfo also carries the decoded stream format.
        return {
          path,
          title,
          duration_ms: 272_000,
          sample_rate: 48_000,
          channels: 2,
        };
      }
      case "music_play":
        state.music.playing = true;
        emitMusicStatus();
        return null;
      case "music_pause":
        state.music.playing = false;
        emitMusicStatus();
        return null;
      case "music_stop":
        state.music = {
          ...state.music,
          playing: false,
          path: null,
          title: null,
          duration_ms: 0,
          position_ms: 0,
        };
        syncQueueShape();
        emitMusicStatus();
        return null;
      case "music_seek":
        state.music.position_ms = Number(args.ms ?? 0);
        emitMusicStatus();
        return null;
      case "music_set_volume":
        state.music.volume = Number(args.v ?? 1);
        emitMusicStatus();
        return null;
      case "music_set_loop":
        state.music.loop = Boolean(args.on);
        emitMusicStatus();
        return null;
      case "music_set_monitor":
        state.music.monitor = Boolean(args.on);
        emitMusicStatus();
        return null;

      // --- Session 29b Force Stop --------------------------------------------
      case "set_music_force_stop":
        // A setting, so this is the whole effect: it is read by `music_bot_start`
        // the next time this client becomes host, and by nothing else. In
        // particular it must not touch a bot that is already running, which is the
        // plan's "applied each time the user becomes host".
        state.settings.music_force_stop = Boolean(args.enabled);
        return null;

      case "request_music_stop": {
        // Mirrors `RoomManager::request_music_stop`, which fails only when there is
        // no live transport. It deliberately does *not* consult the host's
        // decision: the frame leaves either way, and whether it is honoured belongs
        // to whoever owns the bot. A mock that refused here would enforce a rule the
        // app does not have — and would hide the very asymmetry this feature is.
        if (state.room.state !== "connected") {
          fail("Join a room before stopping its music.");
        }
        // Nothing is stopped, and that is the honest outcome: this client owns no
        // bot, so the music keeps playing here until the *host* acts. The harness
        // asserts exactly that, so a UI that faked a local stop would fail.
        return null;
      }

      // --- music queue (Session 20) ------------------------------------------
      // Mirrors the Rust side: queue mutation only appends/removes/reorders.
      // It never loads a track, because `MusicSource::queue_append` does not
      // either — the panel issues an explicit `music_queue_jump` for that.
      case "music_queue_list":
        return structuredClone(queue);
      case "music_pick_files":
        return [
          "D:\\Music\\Tycho - Awake\\04 Daydream.flac",
          "D:\\Music\\Tycho - Awake\\05 Dawn To Dusk.flac",
        ];
      case "music_queue_add": {
        const paths = Array.isArray(args.paths) ? (args.paths as string[]) : [];
        let added = 0;
        const skipped: string[] = [];
        for (const p of paths) {
          // A path containing "missing" stands in for an unreadable file, so the
          // partial-success branch is actually reachable in the harness.
          if (p.toLowerCase().includes("missing")) {
            skipped.push(p);
            continue;
          }
          const title = p.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, "") ?? p;
          queue.push({ path: p, title, duration_ms: 240_000 });
          added += 1;
        }
        syncQueueShape();
        emitQueue();
        emitMusicStatus();
        return { added, skipped };
      }
      case "music_queue_remove": {
        const i = Number(args.index ?? -1);
        if (i >= 0 && i < queue.length) queue.splice(i, 1);
        syncQueueShape();
        emitQueue();
        emitMusicStatus();
        return null;
      }
      case "music_queue_clear":
        queue.length = 0;
        syncQueueShape();
        emitQueue();
        emitMusicStatus();
        return null;
      case "music_queue_move": {
        const from = Number(args.from ?? -1);
        const to = Number(args.to ?? -1);
        if (from >= 0 && to >= 0 && from < queue.length && to < queue.length && from !== to) {
          const [item] = queue.splice(from, 1);
          queue.splice(to, 0, item);
        }
        syncQueueShape();
        emitQueue();
        return null;
      }
      case "music_queue_set_loop":
        state.music.queue_looping = Boolean(args.on);
        emitMusicStatus();
        return null;
      case "music_queue_jump": {
        const i = Number(args.index ?? -1);
        if (i < 0 || i >= queue.length) throw new Error("queue index out of range");
        const item = queue[i];
        const play = Boolean(args.play);
        state.music = {
          ...state.music,
          playing: play,
          path: item.path,
          title: item.title,
          duration_ms: item.duration_ms,
          position_ms: 0,
        };
        syncQueueShape();
        emitMusicStatus();
        return null;
      }
      case "music_next": {
        const cur = state.music.queue_index;
        if (cur !== null) {
          const next =
            cur + 1 < queue.length
              ? cur + 1
              : state.music.queue_looping && queue.length > 0
                ? 0
                : null;
          if (next === null) {
            state.music = { ...state.music, playing: false };
          } else {
            const item = queue[next];
            state.music = {
              ...state.music,
              playing: true,
              path: item.path,
              title: item.title,
              duration_ms: item.duration_ms,
              position_ms: 0,
            };
          }
        }
        syncQueueShape();
        emitMusicStatus();
        return null;
      }
      case "music_prev": {
        const cur = state.music.queue_index;
        // Mirrors the Rust rule: past 3 s, previous restarts the track.
        if (cur !== null && state.music.position_ms > 3_000) {
          state.music.position_ms = 0;
        } else if (cur !== null && cur > 0) {
          const item = queue[cur - 1];
          state.music = {
            ...state.music,
            playing: true,
            path: item.path,
            title: item.title,
            duration_ms: item.duration_ms,
            position_ms: 0,
          };
        }
        syncQueueShape();
        emitMusicStatus();
        return null;
      }

      // --- Session 29b: the music bot session ---
      case "music_bot_state":
        return structuredClone(state.bot ?? IDLE_BOT);

      case "music_bot_start": {
        if (!state.bot?.hosting) {
          // The same two refusals, in the same order, as `music_bot_start` in
          // ui/src-tauri/src/lib.rs. Duplicating them is the point: the mock has to
          // be able to *fail* the way the real command fails, or the toast it
          // drives is untestable.
          //
          // `fail`, not `new Error`: Rust returns `Err(String)`, so the rejection
          // value is a bare string. An Error here would make the same refusal read
          // "Error: Music is already playing…" in the harness and
          // "Music is already playing…" in the app.
          if (state.room.peers.some((p) => p.is_bot)) {
            fail("Music is already playing in this room.");
          }
          if (!state.music.active) {
            fail("Press play before sending music to the room.");
          }
          state.room.peers.push({
            uuid: BOT_UUID,
            // The bot shares its host's persistent id. That is the only link
            // between the two sessions, and it is what lets the UI put the Host
            // badge on the right voice card.
            user_id: state.room.self_user_id,
            name: "Music",
            speaking: false,
            mute: false,
            deafen: false,
            music: true,
            music_title: state.music.title ?? "",
            is_bot: true,
            // Session 29b Force Stop: read from settings at the moment the user
            // becomes host, exactly as `music_bot_start` in the Rust does. It cannot
            // come from the fixture: a listener's Stop control is drawn from this
            // value, so a mock that invented it would test a decision the app never
            // makes.
            force_stop: state.settings.music_force_stop,
          });
          state.bot = {
            hosting: true,
            session_id: BOT_UUID,
            host_name: state.settings.nickname,
          };
          void emit("peer_connected", {
            name: "Music",
            uuid: BOT_UUID,
            user_id: state.room.self_user_id,
            is_bot: true,
            force_stop: state.settings.music_force_stop,
            arrived: true,
          });
          void emit("music_bot_changed", structuredClone(state.bot));
        }
        return null;
      }

      case "music_bot_stop": {
        if (state.bot?.hosting) {
          const uuid = state.bot.session_id;
          state.bot = structuredClone(IDLE_BOT);
          state.room.peers = state.room.peers.filter((p) => p.uuid !== uuid);
          void emit("peer_disconnected", {
            reason: "peer left room",
            peer_id: uuid,
          });
          void emit("music_bot_changed", structuredClone(state.bot));
        }
        return null;
      }

      case "debug_log":
        // eslint-disable-next-line no-console
        console.debug("[verio mock] debug_log:", args.msg);
        return null;

      default:
        // eslint-disable-next-line no-console
        console.warn(`[verio mock] unhandled command: ${cmd}`);
        return null;
    }
  }

  // --- install ------------------------------------------------------------
  mockIPC(
    (cmd, args) => {
      if (cmd.startsWith("plugin:event|")) {
        return handleEventPlugin(cmd, (args ?? {}) as Record<string, unknown>);
      }
      return handleCommand(cmd, (args ?? {}) as Record<string, unknown>);
    },
    { shouldMockEvents: false },
  );

  // Window label so `getCurrentWindow()`/`getCurrentWebview()` can resolve.
  const api = internals();
  api.metadata = {
    currentWindow: { label: "main" },
    currentWebview: { windowLabel: "main", label: "main" },
  };

  startMicLevel();
  startDiagnosticsTicker();

  if (scenario.behavior.audioError) {
    // The app does not listen for this yet; emitting it proves the event is
    // real and gives B8 something honest to surface.
    window.setTimeout(() => {
      void emit("audio_error", scenario.behavior.audioError);
    }, 400);
  }

  if (scenario.behavior.roomError) {
    // Session 24: mirrors audioError. Before this, the only way to surface a
    // room_error was the harness panel's button, so the scenario literally
    // named "connection lost" rendered no banner.
    window.setTimeout(() => {
      void emit("room_error", scenario.behavior.roomError);
    }, 400);
  }

  // --- live controls ------------------------------------------------------
  const transport: MockTransport = {
    scenario,
    state,
    log,
    get commandCount() {
      return commandCount;
    },
    emitEvent(event, payload) {
      dispatch(event, payload);
    },
    addRandomPeer() {
      const pool = ["Dan", "Arman", "Nika", "Kian", "Sora", "Elena", "Reza", "Tara", "Sina"];
      const used = new Set(state.room.peers.map((p) => p.name));
      const name = pool.find((n) => !used.has(n)) ?? `Peer ${state.room.peers.length + 1}`;
      const uuid = `${String(state.room.peers.length + 1).padStart(2, "0")}1a2b3c-4d5e-4f60-8a90-0000000000ff`;
      state.room.peers.push({
        uuid,
        user_id: `user-${name.toLowerCase()}`,
        name,
        speaking: false,
        mute: false,
        deafen: false,
        music: false,
        music_title: "",
        // Session 29b: a simulated person, never a bot. The bot peer is created
        // only by `music_bot_start`, which is the one path that declares it.
        is_bot: false,
        // And a person never accepts a remote stop.
        force_stop: false,
      });
      // `arrived: true` because this control simulates a peer actually joining,
      // which is the only thing that should announce itself or play a cue. The
      // scenario rosters are seeded into `state.room.peers` directly and emit
      // nothing, which is what the real app does for the peers already present
      // when you join.
      void emit("peer_connected", { name, uuid, user_id: `user-${name.toLowerCase()}`, arrived: true });
    },
    removeLastPeer() {
      const last = state.room.peers.pop();
      if (last) {
        void emit("peer_disconnected", { reason: "peer timed out", peer_id: last.uuid });
      }
    },
    toggleRandomPeerSpeaking() {
      if (!state.room.peers.length) return;
      const p = state.room.peers[Math.floor(Math.random() * state.room.peers.length)];
      p.speaking = !p.speaking;
      void emit("peer_state", {
        peer_id: p.uuid,
        speaking: p.speaking,
        mute: p.mute,
        deafen: p.deafen,
        music: p.music,
        music_title: p.music_title,
      });
    },
    emitRoomError(message) {
      void emit("room_error", message);
    },
    emitAudioError(message) {
      void emit("audio_error", message);
    },
    setMusicPlaying(playing) {
      state.music.playing = playing;
      if (!state.music.active) {
        state.music.active = true;
        state.music.path = "D:\\Music\\Tycho - Awake\\01 Awake.flac";
        state.music.title = "Awake";
        state.music.duration_ms = 272_000;
        state.music.position_ms = 96_000;
      }
      emitMusicStatus();
    },
    toggleMusicBot() {
      const cmd = state.bot?.hosting ? "music_bot_stop" : "music_bot_start";
      void handleCommand(cmd).catch((e) => {
        // A refusal is the expected answer here — "press play first" is what the
        // harness gets if it is clicked with no track loaded — so it is reported
        // rather than swallowed, and never rethrown into the click handler.
        // eslint-disable-next-line no-console
        console.warn(`[verio mock] ${cmd}: ${String(e)}`);
      });
    },
    setMicLevel(enabled) {
      scenario.behavior.micLevel.enabled = enabled;
      if (!enabled) void emit("input_level", -60);
    },
    pulseMicLevel() {
      let n = 0;
      const id = window.setInterval(() => {
        n += 1;
        void emit("input_level", -46 + Math.abs(Math.sin(n / 4)) * 26);
        if (n >= 24) window.clearInterval(id);
      }, 50);
      timers.push(id);
    },
  };

  // Expose for tooling/screenshots. Purely a marker + handle; nothing reads it
  // in application code.
  (window as unknown as { __VERIO_MOCK__?: unknown }).__VERIO_MOCK__ = transport;

  return transport;
}

export { DEFAULT_SCENARIO_ID };
