/**
 * Session 29b: the music bot's state, and the play/stop policy that goes with it.
 *
 * The bot is a **second session of this client**, so "press Play in a room" now
 * means two things in sequence: start the local player, then open a second
 * connection and join the room as a peer called `Music`. That ordering is the whole
 * reason this is a hook rather than a flag: it is a small state machine with real
 * failure modes, and it belongs in one place rather than spread across the panel's
 * two buttons.
 *
 * WHAT THIS DELIBERATELY DOES NOT DO
 * ----------------------------------
 * It does not own the music bus. `useMusic` does, and this hook only *calls* it. The
 * one thing it adds is the rule that a bot session and a local player are started
 * and stopped together, because a stopped player with a live bot session would be a
 * peer sitting in the room sending nothing — and a running player with no bot
 * session is what the app did before this session, which is a different audio path
 * the UI cannot show.
 *
 * WHY THE HOST IS NOT THE ROOM CREATOR
 * ------------------------------------
 * The host is whoever pressed Play. Nothing here looks at who created the room,
 * because the two are unrelated: the room creator may be a listener, and the host
 * may be somebody who joined last.
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { MusicController } from "./use-music";

/** Mirrors `BotState` in ui/src-tauri/src/lib.rs. */
export interface MusicBotState {
  hosting: boolean;
  /** The bot session's id. `null` when idle. */
  session_id: string | null;
  /** The person this bot belongs to. */
  host_name: string | null;
}

const IDLE: MusicBotState = { hosting: false, session_id: null, host_name: null };

export interface UseMusicBotOptions {
  /** True while this client is in a room. A bot only exists inside one. */
  inCall: boolean;
  /** The local player, which the bot streams. */
  music: MusicController;
  /**
   * True when the roster holds a peer that declared itself a bot. Combined here
   * with whether *we* host, because "a bot is in the room" and "the bot is mine"
   * call for opposite answers to the same button press.
   */
  botPeerPresent: boolean;
  onStatus?: (text: string, isError?: boolean) => void;
}

export interface MusicBotController {
  state: MusicBotState;
  /** Play, or pause. Outside a room this is only the local player. */
  playPause: () => Promise<void>;
  /** Stop the player, and end the bot session when this client hosts it. */
  stop: () => Promise<void>;
}

export function useMusicBot({
  inCall,
  music,
  botPeerPresent,
  onStatus,
}: UseMusicBotOptions): MusicBotController {
  const [state, setState] = useState<MusicBotState>(IDLE);

  const sayRef = useRef(onStatus);
  sayRef.current = onStatus;
  const say = useCallback((text: string, isError = false) => {
    sayRef.current?.(text, isError);
  }, []);

  useEffect(() => {
    invoke<MusicBotState>("music_bot_state").then(setState).catch(console.error);

    let disposed = false;
    const unlisten: Array<() => void> = [];
    const keep = (u: () => void) => {
      if (disposed) u();
      else unlisten.push(u);
    };

    // Every transition, including the ones this client did not cause: the queue
    // running out, and another peer asking us to stop. Without this the UI would
    // keep offering Stop for a session that had already ended.
    void listen<MusicBotState>("music_bot_changed", (ev) => {
      setState(ev.payload);
    }).then(keep);

    // Only the bot's own endings reach here — a user's Stop does not, because the
    // bridge distinguishes them. So a reason always explains something the user
    // could not otherwise account for.
    void listen<{ reason: string }>("music_bot_ended", (ev) => {
      say(`Music stopped: ${ev.payload.reason}.`);
    }).then(keep);

    return () => {
      disposed = true;
      unlisten.forEach((u) => u());
    };
  }, [say]);

  const hosting = state.hosting;
  const hostingRef = useRef(hosting);
  hostingRef.current = hosting;

  const startInFlightRef = useRef<Promise<void> | null>(null);
  const stopRequestedRef = useRef(false);

  const playPause = useCallback(async () => {
    if (startInFlightRef.current) {
      // Avoid toggling or double-starting while an in-flight bot join is underway.
      return;
    }

    if (music.status?.playing) {
      // A pause keeps the bot alive on purpose: the session survives so the queue
      // can be resumed without rejoining the room as a new peer.
      await music.playPause();
      return;
    }

    // Starting. The room gets to answer before any audio does — otherwise a
    // refusal would leave this client playing through the pre-29b path, mixing
    // music into the voice stream where nothing in the UI can show or stop it.
    if (inCall && !hostingRef.current && botPeerPresent) {
      say("Music is already playing in this room.", true);
      return;
    }

    if (!music.hasTrack) {
      await music.playPause();
      return;
    }

    if (!inCall || hostingRef.current) {
      await music.playPause();
      return;
    }

    stopRequestedRef.current = false;
    const runStart = (async () => {
      await music.playPause();
      if (stopRequestedRef.current) {
        await music.stop();
        return;
      }

      try {
        await invoke("music_bot_start");
      } catch (e) {
        // The player is already running, so leaving it would broadcast through a
        // path the room cannot see. Put it back and report why.
        await music.stop();
        say(`${e}`, true);
        return;
      }

      if (stopRequestedRef.current) {
        await music.stop();
        await invoke("music_bot_stop").catch(console.error);
      }
    })();

    startInFlightRef.current = runStart;
    try {
      await runStart;
    } finally {
      if (startInFlightRef.current === runStart) {
        startInFlightRef.current = null;
      }
    }
  }, [botPeerPresent, inCall, music, say]);

  const stop = useCallback(async () => {
    stopRequestedRef.current = true;
    const inFlight = startInFlightRef.current;

    // Playback first, deliberately. The pipeline takes the music bus back the
    // moment the bot lets go, so ending playback *before* releasing the bus closes
    // the window in which the voice session could pull a chunk and mix it into the
    // outgoing voice frame.
    await music.stop();

    if (inFlight) {
      await inFlight.catch(() => {});
    }

    if (hostingRef.current || inFlight !== null) {
      await invoke("music_bot_stop").catch(console.error);
    }
  }, [music]);

  return { state, playPause, stop };
}
