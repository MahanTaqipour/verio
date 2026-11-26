import { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import { AmbientGlassCanvas } from "./components/verio/ambient-shell";
import { AppHeader } from "./components/verio/app-header";
import { Dock } from "./components/verio/dock";
import { LeftIconRail, type NavView } from "./components/verio/left-rail";
import {
  useMicLevelStream,
  type LevelMeterHandle,
} from "./components/verio/level-meter";
import { useMusic } from "./lib/use-music";
import { useMusicBot } from "./lib/use-music-bot";
import { comboFromEvent } from "./lib/hotkey";
import { MusicPanel } from "./components/verio/music-panel";
import { Lobby } from "./components/verio/lobby";
import { Settings, type HotkeyAction } from "./components/verio/settings";
import { Room } from "./components/verio/room";
import { type ChatMessage } from "./components/verio/chat-panel";
import { StatusBanner } from "./components/verio/toast";

// ---------------------------------------------------------------------------
// Rust Type Mirroring
// ---------------------------------------------------------------------------

interface Hotkeys {
  mute: string;
  deafen: string;
  ptt: string;
}

type TransportModeSetting = "auto" | "cloud_relay" | "peer_host" | "direct_p2p";

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
  // Session 7: relay instrumentation
  relay_packets_sent: number;
  relay_packets_recv: number;
  relay_bytes_sent: number;
  relay_bytes_recv: number;
  relay_mode: string;
}

interface Settings {
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
   *
   * A setting rather than live state. It is applied when this user *becomes* host,
   * so toggling it while hosting affects the next bot session, never the running
   * one — the value is announced to the room at join, and a bot that changed its
   * mind mid-session would refuse a stop it had said it would accept.
   */
  music_force_stop: boolean;
}

interface DeviceEntry {
  name: string;
  is_default: boolean;
}

interface DeviceList {
  inputs: DeviceEntry[];
  outputs: DeviceEntry[];
}

interface AudioState {
  muted: boolean;
  deafened: boolean;
  ptt_mode: boolean;
  ptt_held: boolean;
  speaking: boolean;
  loopback: boolean;
}

type RoomState = "idle" | "connecting" | "connected" | "closed";/** Session 12: one remote participant (relay fans out, so there can be several). */
interface PeerInfo {
  uuid: string;
  /**
   * Session 29: the peer's PERSISTENT id, which is what a room-wide mute is
   * keyed on. Never match a mute against `uuid` — that is the session id and
   * changes every time the peer relaunches. Empty until their `Hello` arrives.
   */
  user_id: string;
  name: string;
  speaking: boolean;
  mute: boolean;
  deafen: boolean;
  /** Session 14: peer is playing music into the room. */
  music: boolean;
  music_title: string;
  /**
   * Session 29b: this peer is a music bot session, not a person. Declared by the
   * peer itself and never inferred — the roster only ever sets this to `true`, so a
   * signalling-sourced `false` cannot demote a bot whose relay `Hello` arrived first.
   */
  is_bot: boolean;
  /**
   * Session 29b Force Stop: this bot accepts a remote stop — the host's setting, as
   * it stood when the bot joined.
   *
   * Meaningful only when `is_bot`, and only ever `true` for a peer that declared it
   * on the wire. The room offers a Stop control only on a peer that is a bot *and*
   * accepts one, so a host who disabled force-stop is not shown a button that would
   * do nothing.
   */
  force_stop: boolean;
}

/**
 * Session 29: one room-wide mute. Mirrors `verio_app::room::MuteInfo`.
 *
 * Kept separate from `PeerInfo` on purpose. A tile already carries `mute`, which
 * is the peer's *own* mic state, and a volume of 0, which is this user's private
 * choice not to listen. This is a third, different thing: room state, owned by
 * everyone, that silences that peer for all of us.
 */
interface MuteInfo {
  target_user_id: string;
  /** Who set it. Empty when unknown, e.g. restored from disk. */
  by: string;
}

// MusicStatus moved to src/lib/use-music.ts during the Phase 4 rebuild. It now
// carries the queue shape (queue_index / queue_len / queue_looping) and the hook
// that owns it exports the type.

interface RoomSnapshot {
  state: RoomState;
  peer_name: string | null;
  room_code: string | null;
  direct_codes: string[];
  signaling_port: number;
  vps_address: string;
  transport_mode?: string | null;
  diagnostics?: TransportDiagnostics | null;
  peers?: PeerInfo[];
  /** Session 29: the room-wide mutes, so a reload does not lose them. */
  mutes?: MuteInfo[];
  /** Session 29: this user's own persistent id. */
  self_user_id?: string;
}

// `HotkeyAction` now comes from components/verio/settings.tsx — the capture
// state and the capture handler have to agree on the same union, and the
// Settings surface is where the three actions are named. It was a local
// declaration here until Phase 6.

export default function App() {
  // State
  const [roomState, setRoomState] = useState<RoomState>("idle");
  const [roomCode, setRoomCode] = useState<string | null>(null);
  const [peers, setPeers] = useState<PeerInfo[]>([]);
  /**
   * Session 29: room-wide mutes, keyed by the silenced peer's persistent id.
   * The value is whoever set it, for the "Muted by <name>" banner.
   */
  const [roomMutes, setRoomMutes] = useState<Record<string, string>>({});
  /** Session 29: this user's own persistent id, to tell whether a mute names us. */
  const [selfUserId, setSelfUserId] = useState("");
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  const [rttMs, setRttMs] = useState<number | null>(null);
  const [callElapsedSec, setCallElapsedSec] = useState(0);

  const [settings, setSettings] = useState<Settings | null>(null);
  const [devices, setDevices] = useState<DeviceList>({ inputs: [], outputs: [] });
  const [audioState, setAudioState] = useState<AudioState>({
    muted: false,
    deafened: false,
    ptt_mode: false,
    ptt_held: false,
    speaking: false,
    loopback: false,
  });

  // UI inputs & 3-tab floating island navigation
  const [joinCodeInput, setJoinCodeInput] = useState("");
  const [activeView, setActiveView] = useState<NavView>("stage");
  const [isChatOpen, setIsChatOpen] = useState(false);
  const [isEditingNickname, setIsEditingNickname] = useState(false);
  const [nicknameInput, setNicknameInput] = useState("");
  const [vpsAddressInput, setVpsAddressInput] = useState("");
  const [peerVolumes, setPeerVolumes] = useState<Record<string, number>>({});
  const [copied, setCopied] = useState(false);
  const [capturingAction, setCapturingAction] = useState<HotkeyAction | null>(null);
  const [statusMsg, setStatusMsg] = useState<{ text: string; isError?: boolean } | null>(null);
  const [toastPaused, setToastPaused] = useState(false);

  // Auto-updater state (Session 36)
  const [appVersion, setAppVersion] = useState("0.1.0");
  const [availableUpdate, setAvailableUpdate] = useState<{
    version: string;
    current_version: string;
    date: string | null;
    body: string | null;
  } | null>(null);
  const [isUpdateDialogOpen, setIsUpdateDialogOpen] = useState(false);
  const [isInstallingUpdate, setIsInstallingUpdate] = useState(false);
  const [updateProgress, setUpdateProgress] = useState<{
    downloaded: number;
    total: number | null;
  } | null>(null);
  const [updateInstallError, setUpdateInstallError] = useState<string | null>(null);
  const [updateCheckStatus, setUpdateCheckStatus] = useState<{
    state: "idle" | "checking" | "up-to-date" | "available" | "error";
    message?: string;
  }>({ state: "idle" });

  useEffect(() => {
    let disposed = false;
    let unlistenProgress: (() => void) | null = null;

    invoke<string>("get_app_version")
      .then((v) => {
        if (!disposed && v) setAppVersion(v);
      })
      .catch(() => {});

    void listen<{ downloaded: number; total: number | null }>("update_progress", (ev) => {
      if (!disposed) setUpdateProgress(ev.payload);
    }).then((u) => {
      if (disposed) u();
      else unlistenProgress = u;
    });

    const timer = window.setTimeout(() => {
      if (disposed) return;
      invoke<{
        version: string;
        current_version: string;
        date: string | null;
        body: string | null;
      } | null>("check_for_update")
        .then((u) => {
          if (disposed || !u) return;
          setAvailableUpdate(u);
          setUpdateCheckStatus({
            state: "available",
            message: `Update v${u.version} available`,
          });
        })
        .catch(() => {
          // Ignore background network/release errors on startup.
        });
    }, 3000);

    return () => {
      disposed = true;
      window.clearTimeout(timer);
      unlistenProgress?.();
    };
  }, []);

  const handleCheckForUpdates = async () => {
    if (updateCheckStatus.state === "checking") return;
    setUpdateCheckStatus({ state: "checking" });
    try {
      const u = await invoke<{
        version: string;
        current_version: string;
        date: string | null;
        body: string | null;
      } | null>("check_for_update");
      if (u) {
        setAvailableUpdate(u);
        setUpdateCheckStatus({
          state: "available",
          message: `Update v${u.version} available`,
        });
      } else {
        setAvailableUpdate(null);
        setUpdateCheckStatus({
          state: "up-to-date",
          message: `Up to date (v${appVersion})`,
        });
      }
    } catch (e: any) {
      setUpdateCheckStatus({
        state: "error",
        message: String(e),
      });
    }
  };

  const handleInstallUpdate = async () => {
    if (isInstallingUpdate) return;
    setIsInstallingUpdate(true);
    setUpdateInstallError(null);
    setUpdateProgress({ downloaded: 0, total: null });
    try {
      await invoke("install_update");
    } catch (e: any) {
      setIsInstallingUpdate(false);
      setUpdateInstallError(String(e));
    }
  };

  // Auto-dismiss non-error toast notifications after 2500 ms unless hovered/focused
  useEffect(() => {
    if (!statusMsg || statusMsg.isError || toastPaused) return;
    const timer = window.setTimeout(() => {
      setStatusMsg(null);
    }, 2500);
    return () => window.clearTimeout(timer);
  }, [statusMsg, toastPaused]);

  const isInCall = roomState === "connected" || (roomState === "connecting" && !!roomCode);

  // Session 31: ephemeral in-call chat
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([]);
  const [unreadChatCount, setUnreadChatCount] = useState(0);
  const isChatVisible = activeView === "stage" && isInCall && isChatOpen;
  const isChatVisibleRef = useRef(isChatVisible);
  useEffect(() => {
    isChatVisibleRef.current = isChatVisible;
    if (isChatVisible) {
      setUnreadChatCount(0);
    }
  }, [isChatVisible]);

  const selfUserIdRef = useRef(selfUserId);
  useEffect(() => {
    selfUserIdRef.current = selfUserId;
  }, [selfUserId]);

  const nicknameRef = useRef(settings?.nickname);
  useEffect(() => {
    nicknameRef.current = settings?.nickname;
  }, [settings?.nickname]);

  const seenChatIds = useRef(new Set<string>());

  // Imperative handle for the real-time mic meter. `input_level` arrives at
  // ~20 Hz; this writes `style.width` on a ref and causes zero React renders.
  // Do not convert it to state — see components/verio/level-meter.tsx.
  const micLevelRef = useRef<LevelMeterHandle>(null);

  // The ~20 Hz `input_level` subscription. Same event, same dB→percent
  // mapping, same direct-DOM write — only the listener registration moved out
  // of the initial-load effect so the meter owns its own lifecycle.
  useMicLevelStream(micLevelRef);

  // Initial Load
  useEffect(() => {
    invoke<Settings>("get_settings")
      .then((s) => {
        setSettings(s);
        setNicknameInput(s.nickname);
        setVpsAddressInput(s.vps_address || "ws://127.0.0.1:8443");
      })
      .catch((e) => console.error("get_settings failed", e));

    invoke<DeviceList>("list_devices")
      .then(setDevices)
      .catch((e) => console.error("list_devices failed", e));

    // Music status is fetched and subscribed by useMusic, which owns it.

    invoke<AudioState>("get_audio_state")
      .then(setAudioState)
      .catch((e) => console.error("get_audio_state failed", e));

    invoke<RoomSnapshot>("get_room_state")
      .then((snap) => {
        setRoomState(snap.state);
        setPeers(snap.peers ?? []);
        setRoomCode(snap.room_code);
        setSelfUserId(snap.self_user_id ?? "");
        // The snapshot's list is the room's authority, so it is taken as-is
        // rather than merged — a merge would let a stale local entry survive a
        // mute that was lifted while this webview was reloading.
        const mutes: Record<string, string> = {};
        for (const m of snap.mutes ?? []) mutes[m.target_user_id] = m.by;
        setRoomMutes(mutes);

        if (snap.diagnostics?.rtt_ms != null) setRttMs(Math.round(snap.diagnostics.rtt_ms));
      })
      .catch((e) => console.error("get_room_state failed", e));

    let disposed = false;
    const unlistens: Array<() => void> = [];
    const subscribe = <T,>(
      event: string,
      handler: Parameters<typeof listen<T>>[1]
    ) => {
      void listen<T>(event, handler).then((u) => {
        if (disposed) {
          u();
        } else {
          unlistens.push(u);
        }
      });
    };

    // Mic level is subscribed separately via `useMicLevelStream` above.

    subscribe<boolean>("speaking_changed", (ev) => {
      setAudioState((prev) => ({ ...prev, speaking: ev.payload }));
    });

    subscribe<AudioState>("audio_state_changed", (ev) => {
      setAudioState(ev.payload);
    });

    subscribe<RoomState>("room_state", (ev) => {
      setRoomState(ev.payload);
      if (ev.payload === "idle" || ev.payload === "closed") {
        setRoomCode("");
        setPeers([]);
        setRoomMutes({});
        setRttMs(null);
        setCallElapsedSec(0);
        setChatMessages([]);
        setUnreadChatCount(0);
        setIsChatOpen(false);
        seenChatIds.current.clear();
      }
    });

    subscribe<{ code: string }>("room_created", (ev) => {
      setRoomCode(ev.payload.code);
      setStatusMsg({ text: `Room #${ev.payload.code} created. Share code to connect.` });
    });

    subscribe<{ code: string }>("room_joined", (ev) => {
      setRoomCode(ev.payload.code);
      setStatusMsg({ text: `Connected to room #${ev.payload.code}.` });
    });

    subscribe<{
      name: string;
      uuid?: string;
      user_id?: string;
      is_bot?: boolean;
      force_stop?: boolean;
      arrived?: boolean;
    }>("peer_connected", (ev) => {
      const uuid = ev.payload.uuid || ev.payload.name;
      setPeers((prev) => {
        const rest = prev.filter((p) => p.uuid !== uuid);
        const existing = prev.find((p) => p.uuid === uuid);
        return [
          ...rest,
          {
            uuid,
            // Session 29: the persistent id, which the tile needs to offer a
            // room-wide mute. A peer whose `Hello` has not arrived yet sends
            // none — `|| existing?.user_id` keeps an id we already learned
            // rather than letting an empty one erase it, the same rule the
            // Rust roster follows.
            user_id: ev.payload.user_id || existing?.user_id || "",
            name: ev.payload.name,
            speaking: existing?.speaking ?? false,
            mute: existing?.mute ?? false,
            deafen: existing?.deafen ?? false,
            music: existing?.music ?? false,
            music_title: existing?.music_title ?? "",
            // Session 29b: same one-way rule as the Rust roster — a bot
            // declaration is never withdrawn, because the signalling server
            // cannot carry it at all and would send `false` for every peer.
            is_bot: Boolean(ev.payload.is_bot || existing?.is_bot),
            // Session 29b Force Stop: the same one-way rule, and for the same
            // reason — only a relay `Hello` can carry this, so a signalling
            // event's `false` must not revoke a decision we already learned.
            force_stop: Boolean(ev.payload.force_stop || existing?.force_stop),
          },
        ].sort((a, b) => a.uuid.localeCompare(b.uuid));
      });
      // `arrived === false` means the roster is telling us who is *already*
      // here — the peers listed when we join a populated room, or a peer
      // renaming themselves. The peer belongs in the list either way, but
      // saying they "joined the call" would be a claim the app cannot support.
      if (ev.payload.arrived !== false) {
        setStatusMsg({ text: `${ev.payload.name} joined the call.` });
      }
    });

    subscribe<{ reason: string; peer_id?: string | null }>("peer_disconnected", (ev) => {
      const gone = ev.payload.peer_id;
      if (gone) {
        setPeers((prev) => prev.filter((p) => p.uuid !== gone));
      }
      setStatusMsg({ text: `Peer disconnected: ${ev.payload.reason}` });
    });

    subscribe<{
      peer_id?: string;
      speaking: boolean;
      mute: boolean;
      deafen: boolean;
      music?: boolean;
      music_title?: string;
    }>("peer_state", (ev) => {
      const id = ev.payload.peer_id;
      if (!id) return;
      setPeers((prev) =>
        prev.map((p) =>
          p.uuid === id
            ? {
                ...p,
                speaking: ev.payload.speaking,
                mute: ev.payload.mute,
                deafen: Boolean(ev.payload.deafen),
                music: Boolean(ev.payload.music),
                music_title: ev.payload.music_title ?? "",
              }
            : p
        )
      );
    });

    subscribe<{ target_user_id: string; muted: boolean; by: string }>(
      "peer_muted",
      (ev) => {
        const { target_user_id, muted, by } = ev.payload;
        setRoomMutes((prev) => {
          const next = { ...prev };
          if (muted) next[target_user_id] = by;
          else delete next[target_user_id];
          return next;
        });
        // Deliberately no status message. The *target* gets a banner explaining
        // why they are silent; announcing to the room who muted whom would turn
        // a friend-group control into a public event.
      }
    );

    subscribe<{ ms: number }>("rtt_updated", (ev) => {
      setRttMs(Math.round(ev.payload.ms));
    });

    // music_status_changed is subscribed by useMusic.

    subscribe<string>("room_error", (ev) => {
      setStatusMsg({ text: ev.payload, isError: true });
    });

    subscribe<string>("audio_error", (ev) => {
      setStatusMsg({ text: ev.payload, isError: true });
    });

    subscribe<{ by: string; acted: boolean }>("music_stop_requested", (ev) => {
      if (ev.payload.acted) {
        setStatusMsg({ text: `${ev.payload.by} stopped the music.` });
      }
    });

    subscribe<{
      id: string;
      sender_user_id: string;
      sender_name: string;
      text: string;
      timestamp_ms: number;
    }>("chat_message", (ev) => {
      const msg = ev.payload;
      if (seenChatIds.current.has(msg.id)) return;
      seenChatIds.current.add(msg.id);

      const isSelf =
        selfUserIdRef.current !== "" &&
        msg.sender_user_id !== "" &&
        msg.sender_user_id === selfUserIdRef.current;
      const newMsg: ChatMessage = {
        id: msg.id,
        senderUserId: msg.sender_user_id,
        senderName: isSelf ? "You" : msg.sender_name,
        text: msg.text,
        timestampMs: msg.timestamp_ms,
        isSelf,
      };
      setChatMessages((prev) => [...prev, newMsg]);
      if (!isChatVisibleRef.current && !isSelf) {
        setUnreadChatCount((prev) => prev + 1);
        const preview = msg.text.length > 40 ? msg.text.slice(0, 40) + "..." : msg.text;
        setStatusMsg({ text: `${msg.sender_name}: ${preview}` });
      }
    });

    return () => {
      disposed = true;
      for (const u of unlistens) u();
    };
  }, []);

  // Custom right-click menu: suppress the WebView's default (Print / Save as /
  // Reload / Inspect) and show an app-appropriate menu instead.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      e.preventDefault();
      setCtxMenu({ x: e.clientX, y: e.clientY });
    };
    const dismiss = () => setCtxMenu(null);
    window.addEventListener("contextmenu", onContextMenu);
    window.addEventListener("click", dismiss);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    return () => {
      window.removeEventListener("contextmenu", onContextMenu);
      window.removeEventListener("click", dismiss);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
    };
  }, []);

  // Poll RTT every 1000 ms while in-call
  useEffect(() => {
    if (!isInCall) {
      return;
    }
    const pollDiag = () => {
      invoke<TransportDiagnostics | null>("get_transport_diagnostics")
        .then((d) => {
          if (d && d.rtt_ms !== null && d.rtt_ms !== undefined) {
            setRttMs(Math.round(d.rtt_ms));
          }
        })
        .catch(() => {});
    };
    pollDiag();
    const interval = setInterval(pollDiag, 1000);
    return () => clearInterval(interval);
  }, [isInCall]);

  // Elapsed call duration timer (1 s ticks while connected)
  useEffect(() => {
    if (roomState !== "connected") {
      setCallElapsedSec(0);
      return;
    }
    const startedAt = Date.now();
    setCallElapsedSec(0);
    const t = setInterval(() => {
      setCallElapsedSec(Math.floor((Date.now() - startedAt) / 1000));
    }, 1000);
    return () => clearInterval(t);
  }, [roomState]);

  // -------------------------------------------------------------------------
  // Real Hotkey Recording (Captures any key combo: B, Shift+B, Ctrl+Shift+B)
  // -------------------------------------------------------------------------
  const handleKeyDown = useCallback(
    async (e: KeyboardEvent) => {
      if (!capturingAction) return;

      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
        try {
          await invoke("cancel_hotkey_capture");
        } catch {}
        setCapturingAction(null);
        return;
      }

      if (e.key === "Backspace" || e.key === "Delete") {
        try {
          await invoke("set_hotkey", { action: capturingAction, combo: "" });
          setSettings((prev) =>
            prev
              ? {
                  ...prev,
                  hotkeys: { ...prev.hotkeys, [capturingAction]: "" },
                }
              : null
          );
          setStatusMsg({ text: `Cleared ${capturingAction} hotkey.` });
        } catch (err: any) {
          setStatusMsg({ text: `Failed to clear hotkey: ${err}`, isError: true });
        }
        setCapturingAction(null);
        return;
      }

      // Build the binding from `event.code`, not `event.key`. The numpad
      // needs it: NumpadAdd reports "+" (which would build the unparseable
      // "Ctrl++"), and with NumLock off Numpad0-9 report Insert/End/ArrowDown.
      // See lib/hotkey.ts.
      const combo = comboFromEvent(e);
      if (combo === null) {
        return;
      }

      try {
        await invoke("set_hotkey", { action: capturingAction, combo });
        setSettings((prev) =>
          prev
            ? {
                ...prev,
                hotkeys: { ...prev.hotkeys, [capturingAction]: combo },
              }
            : null
        );
        setStatusMsg({ text: `Set ${capturingAction} to ${combo}` });
      } catch (err: any) {
        setStatusMsg({ text: `Could not bind ${combo}: ${err}`, isError: true });
      }
      setCapturingAction(null);
    },
    [capturingAction]
  );

  useEffect(() => {
    if (capturingAction) {
      window.addEventListener("keydown", handleKeyDown, true);
      return () => {
        window.removeEventListener("keydown", handleKeyDown, true);
      };
    }
  }, [capturingAction, handleKeyDown]);

  // Actions
  const handleBeginCapture = async (action: HotkeyAction) => {
    try {
      await invoke("prepare_hotkey_capture", { action });
    } catch (e) {
      console.warn("prepare_hotkey_capture failed:", e);
    }
    setCapturingAction(action);
  };

  const handleToggleMute = async () => {
    const updated = await invoke<AudioState>("toggle_mute");
    setAudioState(updated);
  };

  const handleToggleDeafen = async () => {
    const updated = await invoke<AudioState>("toggle_deafen");
    setAudioState(updated);
  };

  const handleCreateRoom = async () => {
    try {
      setStatusMsg({ text: "Creating room on signaling server..." });
      const code = await invoke<string>("create_room");
      setRoomCode(code);
    } catch (e: any) {
      setStatusMsg({ text: `Create room error: ${e}`, isError: true });
    }
  };

  const handleJoinRoom = async () => {
    if (joinCodeInput.trim().length < 4) return;
    try {
      setStatusMsg({ text: `Joining room ${joinCodeInput}...` });
      await invoke("join_room", { code: joinCodeInput.trim() });
      setRoomCode(joinCodeInput.trim());
    } catch (e: any) {
      setStatusMsg({ text: `Join error: ${e}`, isError: true });
    }
  };



  // ---------------------------------------------------------------- music ---
  // State, command calls and the queue live in useMusic (src/lib/use-music.ts).
  // Only the panel-open flag stays here: it is a layout concern of this screen,
  // not part of the music controller. `fractionFromEvent` and `formatTime` moved
  // with it — the music block was their only consumer.
  const music = useMusic({
    onStatus: (text, isError) => setStatusMsg({ text, isError }),
  });

  /**
   * Session 29b: the bot peer in this room, if there is one.
   *
   * Read from the roster rather than remembered, because it may not be ours. The
   * host is whoever pressed Play — not the room's creator — so every client has to
   * be able to tell "a bot is playing here" from "the bot is mine", and the roster
   * is the only place that knows.
   */
  const botPeer = peers.find((p) => p.is_bot);
  const bot = useMusicBot({
    inCall: isInCall,
    music,
    botPeerPresent: botPeer !== undefined,
    onStatus: (text, isError) => setStatusMsg({ text, isError }),
  });

  /**
   * The host is the person whose *voice* peer shares the bot's persistent id.
   *
   * That id is the only link between the two sessions, and it is what makes the
   * badge placeable at all: a peer called "Music" on its own does not say whose
   * music it is. Empty while the bot's `Hello` is still in flight, which reads as
   * "nobody is hosting" and is corrected within one gossip interval.
   */
  const botHostUserId = botPeer?.user_id ?? "";

  /**
   * Session 29c: the room's music, for the panel of a listener who is not hosting.
   *
   * The title needs no new wire work: the bot already announces it as
   * `music_title` in its control-channel `State`, immediately on every track
   * change and every 3 s, and the roster has carried it since Session 14. What the
   * wire does *not* carry is whose music it is — a bot is always named "Music" —
   * so the host is resolved exactly the way the Host badge is: the voice peer
   * sharing the bot's persistent id.
   *
   * Both are empty in the ordinary case where this client is the host, because
   * `peers` holds remote peers only and no remote peer shares our own id.
   */
  const roomMusicTitle = botPeer?.music_title ?? "";
  const roomMusicHost =
    botHostUserId === ""
      ? ""
      : (peers.find((p) => !p.is_bot && p.user_id === botHostUserId)?.name ?? "");

  /**
   * Session 29b Force Stop: this user's preference, and the running bot's decision.
   *
   * Two different things, deliberately kept apart.
   *
   * `forceStop` is the *setting* — stored, and applied the next time this user
   * becomes host. It drives the toggle, and it says nothing about the music that is
   * playing right now.
   *
   * `botAcceptsRemoteStop` is what the bot in the room actually announced, which is
   * the only thing that may decide whether a Stop control is offered. It is `false`
   * while no bot is present, because there is nothing to stop — and `false` for a
   * bot whose host disabled force-stop, which is what keeps a listener from being
   * shown a button that would do nothing.
   */
  const forceStop = settings?.music_force_stop ?? true;
  const botAcceptsRemoteStop = botPeer?.force_stop ?? false;

  const handleSetMusicForceStop = (on: boolean) => {
    setSettings((prev) => (prev ? { ...prev, music_force_stop: on } : prev));
    void invoke("set_music_force_stop", { enabled: on }).catch((e) => {
      // Put the switch back rather than leave it showing a value that was never
      // stored — a toggle that lies about what will be applied next time you host
      // is worse than one that visibly failed.
      setSettings((prev) => (prev ? { ...prev, music_force_stop: !on } : prev));
      setStatusMsg({
        text: `Could not save the Force Stop setting: ${e}`,
        isError: true,
      });
    });
  };

  const handleRequestMusicStop = () => {
    // Reports only that the request left. Whether the music actually stops is the
    // host's decision, and the roster — not this promise — is what will show it.
    void invoke("request_music_stop").catch((e) =>
      setStatusMsg({ text: `${e}`, isError: true }),
    );
  };

  const handleSetVoiceQuality = async (kbps: number) => {
    // Reports the outcome instead of assuming it. The legacy version did
    // `invoke(...).catch(console.error)` and then set the local value and the
    // "Voice quality set to N kbps." toast unconditionally — so a rejected
    // command still told you it had worked.
    try {
      await invoke("set_voice_quality", { kbps });
      setSettings((prev) => (prev ? { ...prev, voice_bitrate_kbps: kbps } : null));
      setStatusMsg({ text: `Voice quality set to ${kbps} kbps.` });
    } catch (e: any) {
      setStatusMsg({ text: `Could not set voice quality: ${e}`, isError: true });
    }
  };

  const handleSetMusicQuality = async (kbps: number) => {
    try {
      await invoke("set_music_quality", { kbps });
      setSettings((prev) => (prev ? { ...prev, music_bitrate_kbps: kbps } : null));
      setStatusMsg({ text: `Music quality set to ${kbps} kbps.` });
    } catch (e: any) {
      setStatusMsg({ text: `Could not set music quality: ${e}`, isError: true });
    }
  };

  const selectView = (next: NavView) => {
    if (activeView === "settings" && next !== "settings" && capturingAction) {
      void invoke("cancel_hotkey_capture").catch(console.error);
      setCapturingAction(null);
    }
    setActiveView(next);
  };

  const handleJoinMainRoom = async () => {
    if (roomState === "connecting") return;
    if (isInCall && roomCode === "1111") {
      selectView("stage");
      return;
    }
    try {
      if (isInCall) {
        await invoke("leave_room");
        setRoomCode(null);
        setPeers([]);
        setRoomMutes({});
        setChatMessages([]);
        setUnreadChatCount(0);
        setCallElapsedSec(0);
        setIsChatOpen(false);
        seenChatIds.current.clear();
      }
      setStatusMsg({ text: "Entering Main Squad Room #1111..." });
      await invoke("join_room", { code: "1111" });
      setRoomCode("1111");
      selectView("stage");
    } catch (e: any) {
      setStatusMsg({ text: `Could not enter #1111: ${e}`, isError: true });
    }
  };

  const handleLeaveCall = async () => {
    try {
      await invoke("leave_room");
      setRoomCode(null);
      setPeers([]);
      setRoomMutes({});
      setCallElapsedSec(0);
      setChatMessages([]);
      setUnreadChatCount(0);
      setIsChatOpen(false);
      seenChatIds.current.clear();
      setStatusMsg({ text: "Left call." });
    } catch (e: any) {
      console.error(e);
    }
  };

  const handleToggleChat = () => {
    if (activeView !== "stage") {
      selectView("stage");
      setIsChatOpen(true);
      setUnreadChatCount(0);
      return;
    }
    setIsChatOpen((v) => {
      const next = !v;
      if (next) setUnreadChatCount(0);
      return next;
    });
  };

  const handleSendChatMessage = async (text: string) => {
    try {
      await invoke("send_chat_message", { text });
    } catch (e: any) {
      setStatusMsg({ text: `Could not send message: ${e}`, isError: true });
    }
  };


  const handleCopyCode = () => {
    if (!roomCode) return;
    navigator.clipboard.writeText(roomCode);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handlePeerVolumeChange = (peerId: string, vol: number) => {
    setPeerVolumes((prev) => ({ ...prev, [peerId]: vol }));
    invoke("set_peer_volume", { peerId, volume: vol }).catch(console.error);
  };

  /**
   * B1 — per-peer local mute.
   *
   * There is no `set_peer_mute` command, and there does not need to be: a local
   * mute *is* `set_peer_volume(uuid, 0)`, which is the same call the slider
   * already makes. So this is a second control for an existing capability, not
   * a new one — the one thing it adds is memory, so that unmuting restores the
   * level the user had rather than a hardcoded 100 %.
   *
   * The memory is a ref, not state: it never needs to render. It is written
   * only at mute time from the live value, so it cannot drift out of date —
   * dragging the slider while unmuted updates `peerVolumes`, which is what the
   * next mute reads.
   *
   * `peerVolumes` is never cleared when a peer leaves (pre-existing — see
   * REDESIGN-PLAN.md R5, preserved deliberately). A stale entry here is
   * therefore equally possible and equally harmless: it is keyed by uuid, and a
   * rejoining peer gets a fresh one.
   */
  const peerVolumeMemory = useRef<Record<string, number>>({});

  const handleTogglePeerMute = (uuid: string) => {
    const current = peerVolumes[uuid] ?? 1;
    if (current === 0) {
      handlePeerVolumeChange(uuid, peerVolumeMemory.current[uuid] ?? 1);
    } else {
      peerVolumeMemory.current[uuid] = current;
      handlePeerVolumeChange(uuid, 0);
    }
  };

  /**
   * Session 29: silence a peer for the whole room, or lift it.
   *
   * A different control from `handleTogglePeerMute` above, which only changes
   * what *this* user hears. This one is broadcast and changes what everyone
   * hears, so it gets its own button and its own label rather than a second
   * meaning folded into the volume slider.
   *
   * The persistent id is what is sent. A peer whose `Hello` has not arrived yet
   * has none, and the button is not offered for them — muting a session id would
   * work until they relaunched and then quietly stop.
   */
  const handleToggleRoomMute = (userId: string, muted: boolean) => {
    invoke("set_peer_muted", { targetUserId: userId, muted }).catch((e) =>
      setStatusMsg({ text: `Could not change the room mute: ${e}`, isError: true })
    );
  };

  const handleSaveNickname = async () => {
    if (!nicknameInput.trim()) return;
    try {
      const trimmed = nicknameInput.trim();
      await invoke("set_nickname", { name: trimmed, nickname: trimmed });
      setSettings((prev) => prev ? { ...prev, nickname: trimmed } : null);
      setIsEditingNickname(false);
      setStatusMsg({ text: `Nickname updated to "${trimmed}".` });
    } catch (e: any) {
      setStatusMsg({ text: `Failed to update nickname: ${e}`, isError: true });
    }
  };

  const handleSaveVpsAddress = async () => {
    if (!vpsAddressInput.trim()) return;
    await invoke("set_vps_address", { address: vpsAddressInput.trim() });
    setSettings((prev) => prev ? { ...prev, vps_address: vpsAddressInput.trim() } : null);
    setStatusMsg({ text: "Signaling server address saved." });
  };

  // ---------------------------------------------------------------- settings ---
  // Phase 6. `components/verio/settings.tsx` is presentation only, so every
  // command call stays here. Each handler keeps the legacy shape — call the
  // command, then mirror the value into local state — so that rebuilding the
  // view cannot quietly change what the backend is asked to do.
  //
  // The two exceptions are called out where they happen: `handleClearHotkey`
  // and the two quality handlers now report failure instead of swallowing it.

  const handleSetInputDevice = async (name: string | null) => {
    // `name` is `Option<String>` on the Rust side, so null means "system
    // default" and is a real value, not a missing one.
    await invoke("set_input_device", { name }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, input_device: name } : null));
  };

  const handleSetOutputDevice = async (name: string | null) => {
    await invoke("set_output_device", { name }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, output_device: name } : null));
  };

  const handleSetInputGain = async (gainDb: number) => {
    // The Rust argument is `gain_db` because Tauri maps the camelCase key the
    // UI sends onto a snake_case parameter — see the Session 14 note in
    // src-tauri/src/lib.rs, where this was `db` and every invoke failed.
    await invoke("set_input_gain", { gainDb }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, input_gain_db: gainDb } : null));
  };

  const handleSetNoiseSuppression = async (on: boolean) => {
    await invoke("set_noise_suppression", { on }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, noise_suppression: on } : null));
  };

  const handleSetLoopback = async (on: boolean) => {
    await invoke("set_loopback", { on }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, loopback: on } : null));
  };

  /**
   * `debug_wavs` had a live command and a persisted field but no control
   * anywhere in the UI (REDESIGN-PLAN.md B4). This is its first caller.
   */
  const handleSetDebugWavs = async (on: boolean) => {
    await invoke("set_debug_wavs", { on }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, debug_wavs: on } : null));
    setStatusMsg({ text: `Debug WAV capture ${on ? "enabled" : "disabled"}.` });
  };

  /**
   * Clear one hotkey binding.
   *
   * The legacy modal cleared mute and deafen with a bare inline
   * `await invoke(...)` and no catch. `set_hotkey` rejects on an unparseable
   * combo, so a rejection there was an unhandled promise rejection and the chip
   * went on showing the old binding. This reports instead.
   */
  const handleClearHotkey = async (action: HotkeyAction) => {
    try {
      await invoke("set_hotkey", { action, combo: "" });
      setSettings((prev) =>
        prev ? { ...prev, hotkeys: { ...prev.hotkeys, [action]: "" } } : null
      );
      setStatusMsg({ text: `Cleared ${action} hotkey.` });
    } catch (e: any) {
      setStatusMsg({ text: `Failed to clear hotkey: ${e}`, isError: true });
    }
  };

  // Transmission Mode Switcher
  const handleSelectVoiceActivity = async () => {
    try {
      await invoke("set_hotkey", { action: "ptt", combo: "" });
      setSettings((prev) =>
        prev ? { ...prev, hotkeys: { ...prev.hotkeys, ptt: "" } } : null
      );
      setStatusMsg({ text: "Voice Activity mode enabled (Open Mic)." });
    } catch (err: any) {
      setStatusMsg({ text: `Error: ${err}`, isError: true });
    }
  };

  const handleSelectPushToTalk = async () => {
    const existing = settings?.hotkeys.ptt;
    const targetKey = existing && existing.trim().length > 0 ? existing : "B";
    try {
      await invoke("set_hotkey", { action: "ptt", combo: targetKey });
      setSettings((prev) =>
        prev ? { ...prev, hotkeys: { ...prev.hotkeys, ptt: targetKey } } : null
      );
      setStatusMsg({ text: `Push-to-Talk active (Key: ${targetKey}).` });
    } catch (err: any) {
      setStatusMsg({ text: `Error: ${err}`, isError: true });
    }
  };



  // `getInitials` moved to components/verio/room.tsx in Phase 7 — its only
  // caller was the peer tile, and it is presentation. Note the two versions did
  // NOT behave identically: this one could return an empty string for a name
  // made only of whitespace, where the Room version falls back to "U". The
  // fallback is the one that was always intended ("U" for unknown), so the
  // move is a fix, not a faithful port.

  const isPttMode = Boolean(settings?.hotkeys.ptt && settings.hotkeys.ptt.trim().length > 0);

  /**
   * Session 24: the live transport descriptor is gone, and with it
   * `src/lib/transport.ts`.
   *
   * It rendered exactly one row in the diagnostics drawer — "Transport path /
   * Encrypted relay" — and that row was the only thing in the panel asserted
   * from architecture rather than measured. `TransportDiagnostics` exposes no
   * crypto field at all, so the app cannot know whether the media is encrypted,
   * only that it ought to be. The claim was true (the transport establishes ICE
   * + DTLS) but every neighbouring row is a value the app actually read, and one
   * unmeasured row in a panel of measurements is the wrong kind of mixed.
   *
   * History worth keeping, because it explains why transport has no UI at all:
   * voice transport used to be user-selectable, and the Host Call card
   * advertised the configured mode behind a "change it in Settings" button. That
   * surface was a stack of three falsehoods — no control in Settings could
   * change the value, the frontend never called the existing `set_transport_mode`
   * command, and `room.rs` pinned the effective mode to cloud relay regardless.
   * Verio is cloud-relay only, so the surface was removed rather than repaired.
   */

  const isMusicTabActive = activeView === "music";
  const isLoopback = Boolean(settings?.loopback ?? audioState.loopback);
  const busyStatusText =
    roomState === "connecting"
      ? roomCode === "1111"
        ? "CONNECTING TO MAIN ROOM #1111..."
        : roomCode
          ? `JOINING ROOM #${roomCode}...`
          : "CONNECTING TO ROOM..."
      : undefined;

  const musicPanelSharedProps = {
    music,
    bot,
    deafened: audioState.deafened,
    forceStop,
    botPresent: botPeer !== undefined,
    botAcceptsRemoteStop,
    inCall: isInCall,
    roomMusicTitle,
    roomMusicHost,
    onSetForceStop: handleSetMusicForceStop,
    onRequestMusicStop: handleRequestMusicStop,
  };

  return (
    <AmbientGlassCanvas>
      {/* Zone 1: Floating 48px Top Command Bar */}
      <AppHeader
        activeView={activeView}
        rttMs={rttMs}
        state={roomState}
        roomCode={roomCode}
        isInCall={isInCall}
        elapsedSeconds={callElapsedSec}
        copied={copied}
        onCopyCode={handleCopyCode}
        onJoinMainRoom={() => void handleJoinMainRoom()}
        chatOpen={isChatOpen}
        onToggleChat={handleToggleChat}
        unreadChatCount={unreadChatCount}
        availableUpdate={availableUpdate}
        onOpenUpdateDialog={() => {
          setUpdateInstallError(null);
          setIsUpdateDialogOpen(true);
        }}
      />

      {/* Zone 2 + Zone 3: Floating Left Rail & Dynamic Stage */}
      <div className="flex min-h-0 flex-1 flex-row gap-2.5 overflow-hidden">
        {/* Zone 2: Floating 72px Left Icon Rail (Room, Music, Settings) */}
        <LeftIconRail
          activeView={activeView}
          onSelectView={selectView}
          musicPlaying={Boolean(music.status?.active || botPeer !== undefined)}
          isInCall={isInCall}
        />

        {/* Zone 3: Dynamic Center Stage */}
        <main className="relative flex min-w-0 flex-1 flex-col overflow-hidden">
          {activeView === "stage" &&
            (isInCall ? (
              <Room
                roomCode={roomCode}
                nickname={settings?.nickname || "You"}
                self={{
                  speaking: audioState.speaking,
                  muted: audioState.muted,
                  deafened: audioState.deafened,
                }}
                micMeterRef={micLevelRef}
                peers={peers}
                peerVolumes={peerVolumes}
                onPeerVolume={handlePeerVolumeChange}
                onTogglePeerMute={handleTogglePeerMute}
                roomMutes={roomMutes}
                onToggleRoomMute={handleToggleRoomMute}
                selfUserId={selfUserId}
                botHostUserId={botHostUserId}
                chatOpen={isChatOpen}
                onCloseChat={() => setIsChatOpen(false)}
                chatMessages={chatMessages}
                onSendMessage={(text) => void handleSendChatMessage(text)}
                music={music}
                bot={bot}
                botPresent={botPeer !== undefined}
                botAcceptsRemoteStop={botAcceptsRemoteStop}
                roomMusicTitle={roomMusicTitle}
                roomMusicHost={roomMusicHost}
                onRequestMusicStop={handleRequestMusicStop}
                onOpenMusicTab={() => selectView("music")}
              />
            ) : (
              <div className="glass-card min-h-0 flex-1 overflow-y-auto rounded-[22px]">
                <Lobby
                  code={joinCodeInput}
                  onCodeChange={setJoinCodeInput}
                  onCreateRoom={handleCreateRoom}
                  onJoinRoom={handleJoinRoom}
                  onJoinMainRoom={() => void handleJoinMainRoom()}
                  busy={roomState === "connecting"}
                  busyLabel={busyStatusText}
                />
              </div>
            ))}

          {activeView === "music" && (
            <div className="flex min-h-0 flex-1 overflow-y-auto p-1">
              <MusicPanel {...musicPanelSharedProps} />
            </div>
          )}

          {activeView === "settings" && (
            <Settings
              settings={settings}
              devices={devices}
              capturingAction={capturingAction}
              onCancelCapture={() => {
                if (capturingAction) {
                  void invoke("cancel_hotkey_capture").catch(console.error);
                  setCapturingAction(null);
                }
              }}
              vpsAddress={vpsAddressInput}
              onVpsAddressChange={setVpsAddressInput}
              onSaveVpsAddress={() => void handleSaveVpsAddress()}
              onClose={() => selectView("stage")}
              onSelectVoiceActivity={() => void handleSelectVoiceActivity()}
              onSelectPushToTalk={() => void handleSelectPushToTalk()}
              onBeginCapture={(a) => void handleBeginCapture(a)}
              onClearHotkey={(a) => void handleClearHotkey(a)}
              onSetInputDevice={(n) => void handleSetInputDevice(n)}
              onSetOutputDevice={(n) => void handleSetOutputDevice(n)}
              onSetInputGain={(db) => void handleSetInputGain(db)}
              onSetNoiseSuppression={(on) => void handleSetNoiseSuppression(on)}
              onSetLoopback={(on) => void handleSetLoopback(on)}
              onSetDebugWavs={(on) => void handleSetDebugWavs(on)}
              onSetVoiceQuality={(k) => void handleSetVoiceQuality(k)}
              onSetMusicQuality={(k) => void handleSetMusicQuality(k)}
              appVersion={appVersion}
              updateCheckStatus={updateCheckStatus}
              onCheckForUpdates={() => void handleCheckForUpdates()}
              onOpenUpdateDialog={() => {
                setUpdateInstallError(null);
                setIsUpdateDialogOpen(true);
              }}
            />
          )}
        </main>
      </div>

      {/* Zone 4: Floating 60px Bottom Hardware Deck */}
      <Dock
        nickname={settings?.nickname || "User"}
        mode={isPttMode ? "ptt" : "open"}
        pttHotkey={settings?.hotkeys.ptt || ""}
        speaking={audioState.speaking}
        isInCall={isInCall}
        isEditingNickname={isEditingNickname}
        nicknameInput={nicknameInput}
        onNicknameInputChange={setNicknameInput}
        onStartEditNickname={() => {
          setNicknameInput(settings?.nickname || "");
          setIsEditingNickname(true);
        }}
        onSaveNickname={() => void handleSaveNickname()}
        onCancelEditNickname={() => setIsEditingNickname(false)}
        muted={audioState.muted}
        deafened={audioState.deafened}
        loopback={isLoopback}
        onToggleMute={() => void handleToggleMute()}
        onToggleDeafen={() => void handleToggleDeafen()}
        onToggleLoopback={() => void handleSetLoopback(!isLoopback)}
        muteHotkey={settings?.hotkeys.mute || "Ctrl+Shift+M"}
        deafenHotkey={settings?.hotkeys.deafen || "Ctrl+Shift+D"}
        musicPanelOpen={isMusicTabActive}
        onToggleMusicPanel={() =>
          selectView(activeView === "music" ? "stage" : "music")
        }
        onOpenMusicPanel={() => selectView("music")}
        music={music.status}
        musicPercent={music.percent}
        hasTrack={music.hasTrack}
        onMusicPlayPause={() => void bot.playPause()}
        onMusicNext={() => void music.next()}
        onLeaveCall={() => void handleLeaveCall()}
        meterRef={micLevelRef}
      />

      {/* Compact Auto-Dismiss Floating Pill Toast */}
      {statusMsg && (
        <StatusBanner
          text={statusMsg.text}
          isError={statusMsg.isError}
          onDismiss={() => {
            setToastPaused(false);
            setStatusMsg(null);
          }}
          onPauseAutoDismiss={() => setToastPaused(true)}
          onResumeAutoDismiss={() => setToastPaused(false)}
        />
      )}

      {/* Right-click context menu */}
      {ctxMenu && (
        <div
          className="ctx-menu"
          style={{ left: ctxMenu.x, top: ctxMenu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          {roomCode ? (
            <button
              className="ctx-item"
              onClick={() => {
                handleCopyCode();
                setCtxMenu(null);
              }}
            >
              Copy room code
            </button>
          ) : null}
          <button
            className="ctx-item"
            onClick={() => {
              void handleToggleMute();
              setCtxMenu(null);
            }}
          >
            {audioState.muted ? "Unmute" : "Mute"}
          </button>
          <button
            className="ctx-item"
            onClick={() => {
              void handleToggleDeafen();
              setCtxMenu(null);
            }}
          >
            {audioState.deafened ? "Undeafen" : "Deafen"}
          </button>
          <div className="ctx-sep" />
          <button
            className="ctx-item"
            onClick={() => {
              selectView("settings");
              setCtxMenu(null);
            }}
          >
            Settings
          </button>
          {isInCall ? (
            <button
              className="ctx-item danger"
              onClick={() => {
                void handleLeaveCall();
                setCtxMenu(null);
              }}
            >
              Leave call
            </button>
          ) : null}
        </div>
      )}

      {/* Software Update Dialog */}
      {isUpdateDialogOpen && availableUpdate && (
        <div
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/55 p-4 backdrop-blur-sm animate-fade"
          onClick={() => {
            if (!isInstallingUpdate) setIsUpdateDialogOpen(false);
          }}
        >
          <div
            role="dialog"
            aria-modal="true"
            aria-label="Software update"
            onClick={(e) => e.stopPropagation()}
            className="glass-card w-full max-w-md rounded-[22px] p-6 shadow-[var(--shadow-overlay)] animate-lift"
          >
            <div className="flex items-start justify-between gap-4">
              <div className="flex flex-col gap-1">
                <span className="label-caps text-lavender">Update Available</span>
                <h2 className="text-15 font-semibold text-text">
                  Verio Update
                </h2>
                <p className="mono-num text-13 text-text-dim">
                  v{availableUpdate.current_version} →{" "}
                  <span className="font-semibold text-accent">
                    v{availableUpdate.version}
                  </span>
                </p>
              </div>
              {!isInstallingUpdate && (
                <button
                  type="button"
                  onClick={() => setIsUpdateDialogOpen(false)}
                  aria-label="Close update dialog"
                  className="inline-flex h-7 w-7 items-center justify-center rounded-xl text-text-dim transition-colors hover:bg-[rgba(185,167,230,0.14)] hover:text-text"
                >
                  ×
                </button>
              )}
            </div>

            {availableUpdate.body && (
              <div className="mt-4 max-h-40 overflow-y-auto rounded-2xl border border-border bg-[var(--bg-input)] p-4 text-13 leading-relaxed text-text-dim whitespace-pre-wrap">
                {availableUpdate.body}
              </div>
            )}

            {isInstallingUpdate && (
              <div className="mt-4 flex flex-col gap-2">
                {(() => {
                  const downloaded = updateProgress?.downloaded ?? 0;
                  const total = updateProgress?.total ?? null;
                  const pct =
                    total && total > 0
                      ? Math.min(100, Math.round((downloaded / total) * 100))
                      : 0;
                  return (
                    <>
                      <div className="flex items-center justify-between text-11 text-text-dim">
                        <span>Downloading &amp; installing update…</span>
                        <span className="mono-num font-semibold text-text">
                          {total && total > 0
                            ? `${pct}%`
                            : `${(downloaded / 1048576).toFixed(1)} MB`}
                        </span>
                      </div>
                      <div className="h-2 w-full overflow-hidden rounded-full border border-border bg-[var(--bg-input)]">
                        <div
                          className="h-full rounded-full bg-gradient-to-r from-[#B9A7E6] to-[#E8FF3A] transition-[width] duration-150"
                          style={{
                            width: `${total && total > 0 ? pct : 35}%`,
                          }}
                        />
                      </div>
                    </>
                  );
                })()}
              </div>
            )}

            {updateInstallError && (
              <p className="mt-3 rounded-xl border border-danger/45 bg-danger/15 px-3 py-2 text-13 text-[#fca5a5]">
                {updateInstallError}
              </p>
            )}

            <div className="mt-5 flex items-center justify-end gap-2">
              <button
                type="button"
                disabled={isInstallingUpdate}
                onClick={() => setIsUpdateDialogOpen(false)}
                className="h-9 rounded-xl border border-border bg-surface-2/85 px-4 text-13 font-medium text-text transition-colors hover:border-border-2 hover:bg-surface-3 disabled:pointer-events-none disabled:opacity-50 press"
              >
                Later
              </button>
              <button
                type="button"
                disabled={isInstallingUpdate}
                onClick={() => void handleInstallUpdate()}
                className="h-9 rounded-xl bg-accent px-4 text-13 font-bold text-[#120D1D] transition-colors hover:bg-accent-hi disabled:pointer-events-none disabled:opacity-60 press"
              >
                {isInstallingUpdate ? "Installing…" : "Update & Restart"}
              </button>
            </div>
          </div>
        </div>
      )}
    </AmbientGlassCanvas>
  );
}
