import { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import { AppHeader } from "./components/verio/app-header";
import { Dock } from "./components/verio/dock";
import {
  useMicLevelStream,
  type LevelMeterHandle,
} from "./components/verio/level-meter";
import { TelemetryRow, TelemetryList } from "./components/verio/telemetry-row";
import { IconButton } from "./components/ui/button";
import { SectionLabel } from "./components/ui/field";
import { IconError, IconX } from "./components/verio/icons";
import { useMusic } from "./lib/use-music";
import { comboFromEvent } from "./lib/hotkey";
import { MusicPanel } from "./components/verio/music-panel";
import { Lobby } from "./components/verio/lobby";
import { Settings, type HotkeyAction } from "./components/verio/settings";
import { Room } from "./components/verio/room";
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
  remote_endpoint: string | null;
  local_addr: string | null;
  packets_sent: number;
  packets_recv: number;
  bytes_sent: number;
  bytes_recv: number;
  last_recv_ms_ago: number | null;
  rtt_ms: number | null;
  ice_state: string | null;
  status: string;
  last_error: string | null;
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
  name: string;
  speaking: boolean;
  mute: boolean;
  deafen: boolean;
  /** Session 14: peer is playing music into the room. */
  music: boolean;
  music_title: string;
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
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number } | null>(null);
  const [rttMs, setRttMs] = useState<number | null>(null);
  const [diagnostics, setDiagnostics] = useState<TransportDiagnostics | null>(null);
  const [isDiagnosticsOpen, setIsDiagnosticsOpen] = useState(false);

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

  // UI inputs
  const [joinCodeInput, setJoinCodeInput] = useState("");
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isEditingNickname, setIsEditingNickname] = useState(false);
  const [nicknameInput, setNicknameInput] = useState("");
  const [vpsAddressInput, setVpsAddressInput] = useState("");
  const [peerVolumes, setPeerVolumes] = useState<Record<string, number>>({});
  const [copied, setCopied] = useState(false);
  const [capturingAction, setCapturingAction] = useState<HotkeyAction | null>(null);
  const [statusMsg, setStatusMsg] = useState<{ text: string; isError?: boolean } | null>(null);

  // Imperative handle for the real-time mic meter. `input_level` arrives at
  // ~20 Hz; this writes `style.width` on a ref and causes zero React renders.
  // Do not convert it to state — see components/verio/level-meter.tsx.
  const micLevelRef = useRef<LevelMeterHandle>(null);

  // The ~20 Hz `input_level` subscription. Same event, same dB→percent
  // mapping, same direct-DOM write — only the listener registration moved out
  // of the initial-load effect so the meter owns its own lifecycle.
  useMicLevelStream(micLevelRef);

  const isInCall = roomState === "connected" || (roomState === "connecting" && !!roomCode);

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

        if (snap.diagnostics) setDiagnostics(snap.diagnostics);
      })
      .catch((e) => console.error("get_room_state failed", e));

    const unlistens: Array<() => void> = [];

    // Mic level is subscribed separately via `useMicLevelStream` above.

    listen<boolean>("speaking_changed", (ev) => {
      setAudioState((prev) => ({ ...prev, speaking: ev.payload }));
    }).then((u) => unlistens.push(u));

    listen<AudioState>("audio_state_changed", (ev) => {
      setAudioState(ev.payload);
    }).then((u) => unlistens.push(u));

    listen<RoomState>("room_state", (ev) => {
      setRoomState(ev.payload);
      if (ev.payload === "idle" || ev.payload === "closed") {
        setPeers([]);
        setRttMs(null);
        setDiagnostics(null);
        setIsDiagnosticsOpen(false);
      }
    }).then((u) => unlistens.push(u));

    listen<{ code: string }>("room_created", (ev) => {
      setRoomCode(ev.payload.code);
      setStatusMsg({ text: `Room #${ev.payload.code} created. Share code to connect.` });
    }).then((u) => unlistens.push(u));

    listen<{ code: string }>("room_joined", (ev) => {
      setRoomCode(ev.payload.code);
      setStatusMsg({ text: `Connected to room #${ev.payload.code}.` });
    }).then((u) => unlistens.push(u));

    listen<{ name: string; uuid?: string }>("peer_connected", (ev) => {
      const uuid = ev.payload.uuid || ev.payload.name;
      setPeers((prev) => {
        const rest = prev.filter((p) => p.uuid !== uuid);
        const existing = prev.find((p) => p.uuid === uuid);
        return [
          ...rest,
          {
            uuid,
            name: ev.payload.name,
            speaking: existing?.speaking ?? false,
            mute: existing?.mute ?? false,
            deafen: existing?.deafen ?? false,
            music: existing?.music ?? false,
            music_title: existing?.music_title ?? "",
          },
        ].sort((a, b) => a.uuid.localeCompare(b.uuid));
      });
      setStatusMsg({ text: `${ev.payload.name} joined the call.` });
    }).then((u) => unlistens.push(u));

    listen<{ reason: string; peer_id?: string | null }>("peer_disconnected", (ev) => {
      const gone = ev.payload.peer_id;
      if (gone) {
        setPeers((prev) => prev.filter((p) => p.uuid !== gone));
      }
      setStatusMsg({ text: `Peer disconnected: ${ev.payload.reason}` });
    }).then((u) => unlistens.push(u));

    listen<{
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
    }).then((u) => unlistens.push(u));

    listen<{ ms: number }>("rtt_updated", (ev) => {
      setRttMs(Math.round(ev.payload.ms));
    }).then((u) => unlistens.push(u));

    // music_status_changed is subscribed by useMusic.

    listen<string>("room_error", (ev) => {
      setStatusMsg({ text: ev.payload, isError: true });
    }).then((u) => unlistens.push(u));

    return () => {
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

  // Poll live transport diagnostics every 1000 ms while in-call
  useEffect(() => {
    if (!isInCall) {
      setDiagnostics(null);
      return;
    }
    const pollDiag = () => {
      invoke<TransportDiagnostics | null>("get_transport_diagnostics")
        .then((d) => {
          if (d) {
            setDiagnostics(d);
            if (d.rtt_ms !== null && d.rtt_ms !== undefined) {
              setRttMs(Math.round(d.rtt_ms));
            }
          }
        })
        .catch(() => {});
    };
    pollDiag();
    const interval = setInterval(pollDiag, 1000);
    return () => clearInterval(interval);
  }, [isInCall]);

  // -------------------------------------------------------------------------
  // Real Hotkey Recording (Captures any key combo: B, Shift+B, Ctrl+Shift+B)
  // -------------------------------------------------------------------------
  const handleKeyDown = useCallback(
    async (e: KeyboardEvent) => {
      if (!capturingAction) return;

      e.preventDefault();
      e.stopPropagation();

      if (e.key === "Escape") {
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
  const [musicPanelOpen, setMusicPanelOpen] = useState(false);

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

  const handleLeaveCall = async () => {
    try {
      await invoke("leave_room");
      setRoomCode(null);
      setPeers([]);
      setRoomState("idle");
      setStatusMsg({ text: "Left call." });
    } catch (e: any) {
      console.error(e);
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

  return (
    <div className="app-container">
      {/*
        Everything that sits *behind* the Settings surface.
        `display: contents` keeps this wrapper out of the layout — its children
        are still the flex items of .app-container, so nothing moves — while
        giving `inert` a single node to apply to. That is what makes the
        `aria-modal="true"` in components/verio/settings.tsx true rather than
        aspirational: while Settings is open, the header, lobby/room, dock and
        context menu are removed from the tab order and from assistive tech.
        The legacy modal made no such claim and left the whole app tabbable
        behind it.
      */}
      <div className="contents" inert={isSettingsOpen || undefined}>
        <AppHeader
          rttMs={rttMs}
          state={roomState}
          roomCode={roomCode}
          isInCall={isInCall}
          diagnosticsOpen={isDiagnosticsOpen}
          onCopyCode={handleCopyCode}
          onToggleDiagnostics={() => setIsDiagnosticsOpen((v) => !v)}
        />

        {/* Main Content */}
        <main className="main">
          {/* Phase 7: the last legacy shell markup. `StatusBanner` keeps the
              message and its tone, and adds the two things the legacy
              `.toast-banner` did not have — a live region (so the message is
              announced at all) and an accessible name on the close button.
              It sits in normal flow at the top of `main`, as the banner did. */}
          {statusMsg && (
            <StatusBanner
              text={statusMsg.text}
              isError={statusMsg.isError}
              onDismiss={() => setStatusMsg(null)}
            />
          )}

          {isInCall ? (
            /* IN-CALL VIEW */
            <>
              <Room
                roomCode={roomCode}
                copied={copied}
                onCopyCode={handleCopyCode}
                onLeave={() => void handleLeaveCall()}
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
              />

              {/* Live network diagnostics — opened from the header's status chip,
                  which is its only caller. It reports what the transport layer
                  actually knows; see the notes on each row for what was removed. */}
              {isDiagnosticsOpen && (
                <aside
                  aria-label="Network diagnostics"
                  // `top-header`, not `top-14`: this overlay must begin exactly
                  // where the header ends, and `h-header` on the header and
                  // `top-header` here now read from one `@theme` declaration
                  // instead of two literals that had to be kept in step by hand.
                  className="fixed top-header right-0 bottom-0 z-90 flex w-80 flex-col gap-3 border-l border-border-2 bg-surface p-5"
                >
                  <div className="flex items-center justify-between gap-3">
                    <SectionLabel level={2} className="text-text-dim">
                      Real-time diagnostics
                    </SectionLabel>
                    <IconButton
                      label="Close diagnostics"
                      size="sm"
                      onClick={() => setIsDiagnosticsOpen(false)}
                    >
                      <IconX size={14} />
                    </IconButton>
                  </div>

                  <TelemetryList className="min-h-0 flex-1 overflow-y-auto">
                    <TelemetryRow
                      label="Latency (RTT)"
                      value={
                        diagnostics?.rtt_ms !== null && diagnostics?.rtt_ms !== undefined
                          ? `${Math.round(diagnostics.rtt_ms)} ms`
                          : rttMs !== null
                            ? `${rttMs} ms`
                            : "Measuring…"
                      }
                    />

                    <TelemetryRow
                      label="Packets"
                      value={`out ${diagnostics?.packets_sent ?? 0} / in ${diagnostics?.packets_recv ?? 0}`}
                      hint="Voice datagrams"
                    />

                    <TelemetryRow
                      label="Last recv"
                      value={
                        diagnostics?.last_recv_ms_ago !== null &&
                        diagnostics?.last_recv_ms_ago !== undefined
                          ? `${diagnostics.last_recv_ms_ago} ms ago`
                          : "None yet"
                      }
                      hint={
                        (diagnostics?.last_recv_ms_ago ?? 0) > 2000
                          ? "High jitter / delayed"
                          : "Healthy inbound stream"
                      }
                      tone={(diagnostics?.last_recv_ms_ago ?? 0) > 2000 ? "warn" : "default"}
                    />

                    {/* The drawer previously had a second "Last Relay Recv" row
                        rendering `last_recv_ms_ago` — the SAME field as the row
                        above, under a relay-specific label. Measured: both rows
                        read "14 ms ago". `TransportDiagnostics` has no
                        relay-specific recency field, so the app does not know it
                        and the row is gone rather than left implying it does. */}
                    <TelemetryRow
                      label="Relay packets"
                      value={`${diagnostics?.relay_packets_sent ?? 0} sent / ${diagnostics?.relay_packets_recv ?? 0} recv`}
                    />

                    <TelemetryRow
                      label="Relay mode"
                      value={diagnostics?.relay_mode || "unknown"}
                      hint={`${diagnostics?.relay_bytes_sent ?? 0} B out / ${diagnostics?.relay_bytes_recv ?? 0} B in`}
                    />

                    {/* Both addresses previously fell back to hardcoded literals —
                        "141.11.1.110:3478" and "0.0.0.0" — which presented a
                        specific address the app had not observed. A null endpoint
                        now says so. */}
                    <TelemetryRow
                      label="Remote endpoint"
                      value={diagnostics?.remote_endpoint || "Not reported"}
                    />

                    <TelemetryRow
                      label="Local socket"
                      value={diagnostics?.local_addr || "Not reported"}
                    />
                  </TelemetryList>

                  {diagnostics?.last_error && (
                    <div className="flex items-start gap-2 rounded-sm border border-danger/35 bg-danger/8 p-3">
                      <IconError size={14} className="mt-0.5 shrink-0 text-danger" />
                      <span className="text-13 text-danger">{diagnostics.last_error}</span>
                    </div>
                  )}
                </aside>
              )}

            </>
          ) : (
            /* LOBBY VIEW */
            <Lobby
              code={joinCodeInput}
              onCodeChange={setJoinCodeInput}
              onCreateRoom={handleCreateRoom}
              onJoinRoom={handleJoinRoom}
              busy={roomState === "connecting"}
            />
          )}
        </main>

        {/* Bottom Dock */}
        {/* Session 13: music player panel (ported from design/music-player) */}
        {musicPanelOpen && (
          <MusicPanel
            music={music}
            deafened={audioState.deafened}
            onClose={() => setMusicPanelOpen(false)}
            className="mx-auto my-4 w-full max-w-140"
          />
        )}


              <Dock
                nickname={settings?.nickname || "User"}
                mode={isPttMode ? "ptt" : "open"}
                pttHotkey={settings?.hotkeys.ptt || ""}
                isEditingNickname={isEditingNickname}
                nicknameInput={nicknameInput}
                onNicknameInputChange={setNicknameInput}
                onStartEditNickname={() => setIsEditingNickname(true)}
                onSaveNickname={() => void handleSaveNickname()}
                onCancelEditNickname={() => setIsEditingNickname(false)}
                muted={audioState.muted}
                deafened={audioState.deafened}
                onToggleMute={() => void handleToggleMute()}
                onToggleDeafen={() => void handleToggleDeafen()}
                muteHotkey={settings?.hotkeys.mute || "Ctrl+Shift+M"}
                deafenHotkey={settings?.hotkeys.deafen || "Ctrl+Shift+D"}
                onOpenSettings={() => setIsSettingsOpen(true)}
                musicPanelOpen={musicPanelOpen}
                onToggleMusicPanel={() => setMusicPanelOpen((v) => !v)}
                onOpenMusicPanel={() => setMusicPanelOpen(true)}
                // `MusicStatus` is structurally assignable to `DockMusic`: the dock
                // needs `active`/`playing` and treats title/path as optional, and
                // the extra queue fields are simply ignored.
                music={music.status}
                musicPercent={music.percent}
                meterRef={micLevelRef}
              />


              {/* The context menu sits inside the inert region too: while
                  Settings is open it must not be reachable either, and it is
                  always dismissed by the same click that opens Settings. */}
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
                      setIsSettingsOpen(true);
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
      </div>
      {/* /contents — end of the inert region */}

      {/* Settings — Phase 6 rebuild. A full-screen surface, not a centred
          modal: see the header comment in components/verio/settings.tsx for why,
          and REDESIGN-PLAN.md R6 for the decision it implements. It renders
          outside the inert wrapper above so it is the only interactive region
          while it is open. */}
      {isSettingsOpen && (
        <Settings
          settings={settings}
          devices={devices}
          capturingAction={capturingAction}
          vpsAddress={vpsAddressInput}
          onVpsAddressChange={setVpsAddressInput}
          onSaveVpsAddress={() => void handleSaveVpsAddress()}
          onClose={() => setIsSettingsOpen(false)}
          onSelectVoiceActivity={() => void handleSelectVoiceActivity()}
          onSelectPushToTalk={() => void handleSelectPushToTalk()}
          onBeginCapture={setCapturingAction}
          onClearHotkey={(a) => void handleClearHotkey(a)}
          onSetInputDevice={(n) => void handleSetInputDevice(n)}
          onSetOutputDevice={(n) => void handleSetOutputDevice(n)}
          onSetInputGain={(db) => void handleSetInputGain(db)}
          onSetNoiseSuppression={(on) => void handleSetNoiseSuppression(on)}
          onSetLoopback={(on) => void handleSetLoopback(on)}
          onSetDebugWavs={(on) => void handleSetDebugWavs(on)}
          onSetVoiceQuality={(k) => void handleSetVoiceQuality(k)}
          onSetMusicQuality={(k) => void handleSetMusicQuality(k)}
        />
      )}
    </div>
  );
}
