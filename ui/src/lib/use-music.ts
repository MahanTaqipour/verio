/**
 * Music player state and actions.
 *
 * Extracted from App.tsx during the Phase 4 rebuild so the presentation can be
 * replaced without touching the behaviour. Everything the old inline version did
 * is preserved exactly, including the two subtleties that are easy to lose:
 *
 *   1. **Volume is debounced (100 ms).** Dragging the fader would otherwise fire
 *      `music_set_volume` on every pointer move, and each call writes through to
 *      the settings store on the Rust side.
 *   2. **`seekDraft` shadows the reported position while dragging.** Status ticks
 *      arrive every 250 ms; without the draft the handle would jump back to the
 *      last reported position mid-drag.
 *
 * The queue list is fetched separately from `MusicStatus` on purpose: status is
 * emitted on every fader movement, and re-serializing a long queue each time
 * would be pure waste. The list arrives from `music_queue_list` and the
 * `music_queue_changed` event.
 *
 * Nothing here auto-plays. Adding files to an empty queue loads the first track
 * *paused*, because music is broadcast to the room and starting playback is the
 * user's decision, not a side effect of picking files.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Mirrors `verio_app::pipeline::MusicStatus`. */
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
  /** Index into the queue of the loaded track, or null if it is not a member. */
  queue_index: number | null;
  queue_len: number;
  queue_looping: boolean;
}

/** Mirrors `verio_app::pipeline::QueueItem`. */
export interface QueueItem {
  path: string;
  title: string;
  duration_ms: number;
}

/** Mirrors `QueueAddResult` in ui/src-tauri/src/lib.rs. */
interface QueueAddResult {
  added: number;
  skipped: string[];
}

/** Payload of the `music_queue_changed` event. */
interface QueueEvent {
  items: QueueItem[];
  index: number | null;
}

export interface UseMusicOptions {
  /** Reports a human-readable line to the app status message. */
  onStatus?: (text: string, isError?: boolean) => void;
}

export interface MusicController {
  status: MusicStatus | null;
  queue: QueueItem[];
  /** True when a track is loaded (playing or paused). */
  hasTrack: boolean;
  /** Playback position in ms, shadowed by the drag draft. */
  positionMs: number;
  durationMs: number;
  /** 0-100. Zero when no duration is known. */
  percent: number;
  /** 0-2, shadowed by the drag draft. */
  volumeDraft: number;
  seekDraft: number | null;
  setSeekDraft: (ms: number | null) => void;

  addFiles: () => Promise<void>;
  playPause: () => Promise<void>;
  stop: () => Promise<void>;
  next: () => Promise<void>;
  prev: () => Promise<void>;
  seekTo: (fraction: number) => Promise<void>;
  setVolume: (fraction: number) => void;
  setTrackLoop: (on: boolean) => Promise<void>;
  setQueueLooping: (on: boolean) => Promise<void>;
  setMonitor: (on: boolean) => Promise<void>;
  jumpTo: (index: number) => Promise<void>;
  removeAt: (index: number) => Promise<void>;
  moveItem: (from: number, to: number) => Promise<void>;
  clearQueue: () => Promise<void>;
}

export function useMusic({ onStatus }: UseMusicOptions = {}): MusicController {
  const [status, setStatus] = useState<MusicStatus | null>(null);
  const [queue, setQueue] = useState<QueueItem[]>([]);
  const [seekDraft, setSeekDraft] = useState<number | null>(null);
  const [volumeDraft, setVolumeDraft] = useState(1.0);
  const volumeTimer = useRef<number | null>(null);

  // Held in a ref so an inline arrow from the caller does not invalidate every
  // callback below on each render.
  const sayRef = useRef(onStatus);
  sayRef.current = onStatus;
  const say = useCallback((text: string, isError = false) => {
    sayRef.current?.(text, isError);
  }, []);

  /** Re-read status. `syncVolume` is false for event-driven refreshes so that a
   *  queue change cannot yank the fader out from under a drag. */
  const fetchStatus = useCallback((syncVolume = false) => {
    invoke<MusicStatus>("music_status")
      .then((s) => {
        setStatus(s);
        if (syncVolume) setVolumeDraft(s.volume);
      })
      .catch(console.error);
  }, []);

  const fetchQueue = useCallback(() => {
    invoke<QueueItem[]>("music_queue_list").then(setQueue).catch(console.error);
  }, []);

  useEffect(() => {
    fetchStatus(true);
    fetchQueue();

    let disposed = false;
    const unlisten: Array<() => void> = [];
    const keep = (u: () => void) => {
      if (disposed) u();
      else unlisten.push(u);
    };

    void listen<MusicStatus>("music_status_changed", (ev) => {
      setStatus(ev.payload);
    }).then(keep);

    void listen<QueueEvent>("music_queue_changed", (ev) => {
      setQueue(ev.payload.items);
      // Keep the queue shape in status honest without touching the fader.
      setStatus((prev) =>
        prev
          ? {
              ...prev,
              queue_index: ev.payload.index,
              queue_len: ev.payload.items.length,
            }
          : prev,
      );
    }).then(keep);

    return () => {
      disposed = true;
      unlisten.forEach((u) => u());
    };
  }, [fetchStatus, fetchQueue]);

  // Flush any pending volume write on unmount, so a drag that ends as the panel
  // closes is not silently dropped.
  useEffect(
    () => () => {
      if (volumeTimer.current !== null) window.clearTimeout(volumeTimer.current);
    },
    [],
  );

  const positionMs = seekDraft !== null ? seekDraft : status?.position_ms ?? 0;
  const durationMs = status?.duration_ms ?? 0;
  const percent =
    durationMs > 0
      ? Math.max(0, Math.min(100, (positionMs / durationMs) * 100))
      : 0;

  const addFiles = useCallback(async () => {
    try {
      const paths = await invoke<string[] | null>("music_pick_files");
      if (!paths || paths.length === 0) return;
      const nothingLoaded = !status?.path;
      const res = await invoke<QueueAddResult>("music_queue_add", { paths });
      const total = res.added + res.skipped.length;
      if (res.skipped.length > 0) {
        say(
          `Added ${res.added} of ${total} — could not read ${res.skipped.length} file(s).`,
          true,
        );
      } else {
        say(`Added ${res.added} track${res.added === 1 ? "" : "s"}.`);
      }
      if (nothingLoaded && res.added > 0) {
        // Load the first entry paused so Play works, without broadcasting music
        // into the room before the user asks for it.
        await invoke("music_queue_jump", { index: 0, play: false }).catch(
          console.error,
        );
      }
    } catch (e) {
      say(`Could not add files: ${e}`, true);
    }
  }, [say, status?.path]);

  const playPause = useCallback(async () => {
    try {
      await invoke(status?.playing ? "music_pause" : "music_play");
      fetchStatus(true);
    } catch (e) {
      say(`${e}`, true);
    }
  }, [fetchStatus, say, status?.playing]);

  const stop = useCallback(async () => {
    await invoke("music_stop").catch(console.error);
    fetchStatus(true);
  }, [fetchStatus]);

  const next = useCallback(async () => {
    await invoke("music_next").catch(console.error);
    fetchStatus(true);
  }, [fetchStatus]);

  const prev = useCallback(async () => {
    await invoke("music_prev").catch(console.error);
    fetchStatus(true);
  }, [fetchStatus]);

  const seekTo = useCallback(
    async (fraction: number) => {
      if (durationMs <= 0) return;
      const ms = Math.round(fraction * durationMs);
      setSeekDraft(null);
      await invoke("music_seek", { ms }).catch(console.error);
      fetchStatus(true);
    },
    [durationMs, fetchStatus],
  );

  const setVolume = useCallback(
    (fraction: number) => {
      const v = Math.max(0, Math.min(2, fraction * 2));
      setVolumeDraft(v);
      if (volumeTimer.current !== null) window.clearTimeout(volumeTimer.current);
      volumeTimer.current = window.setTimeout(() => {
        void invoke("music_set_volume", { v })
          .then(() => fetchStatus(false))
          .catch(console.error);
      }, 100);
    },
    [fetchStatus],
  );

  const setTrackLoop = useCallback(
    async (on: boolean) => {
      await invoke("music_set_loop", { on }).catch(console.error);
      fetchStatus(true);
    },
    [fetchStatus],
  );

  const setQueueLooping = useCallback(
    async (on: boolean) => {
      await invoke("music_queue_set_loop", { on }).catch(console.error);
      fetchStatus(true);
    },
    [fetchStatus],
  );

  const setMonitor = useCallback(
    async (on: boolean) => {
      await invoke("music_set_monitor", { on }).catch(console.error);
      fetchStatus(true);
    },
    [fetchStatus],
  );

  const jumpTo = useCallback(
    async (index: number) => {
      await invoke("music_queue_jump", { index, play: true }).catch(console.error);
      fetchStatus(true);
    },
    [fetchStatus],
  );

  const removeAt = useCallback(
    async (index: number) => {
      await invoke("music_queue_remove", { index }).catch(console.error);
      fetchStatus(true);
    },
    [fetchStatus],
  );

  const moveItem = useCallback(
    async (from: number, to: number) => {
      await invoke("music_queue_move", { from, to }).catch(console.error);
    },
    [],
  );

  const clearQueue = useCallback(async () => {
    await invoke("music_queue_clear").catch(console.error);
    fetchStatus(true);
  }, [fetchStatus]);

  return {
    status,
    queue,
    hasTrack: Boolean(status?.path),
    positionMs,
    durationMs,
    percent,
    volumeDraft,
    seekDraft,
    setSeekDraft,
    addFiles,
    playPause,
    stop,
    next,
    prev,
    seekTo,
    setVolume,
    setTrackLoop,
    setQueueLooping,
    setMonitor,
    jumpTo,
    removeAt,
    moveItem,
    clearQueue,
  };
}

/** `mm:ss`, clamped at zero. Unchanged from the pre-redesign implementation. */
export function formatTime(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}

/** Fraction of an element's width at a client X, clamped to 0-1. */
export function fractionFromEvent(el: HTMLElement, clientX: number): number {
  const rect = el.getBoundingClientRect();
  return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
}

/** File name from a path, for display beside the track title. */
export function fileNameOf(path: string | null): string {
  if (!path) return "";
  return path.split(/[\\/]/).pop() ?? "";
}
