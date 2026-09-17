import { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "./App.css";
import verioLogo from "./assets/verio-logo-primary.png";

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

type RoomState = "idle" | "connecting" | "connected" | "closed";

/** Session 12: one remote participant (relay fans out, so there can be several). */
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



/** Session 13: music player state (mirrors verio-app::pipeline::MusicStatus). */
interface MusicStatus {
  active: boolean;
  playing: boolean;
  path: string | null;
  title: string | null;
  duration_ms: number;
  position_ms: number;
  volume: number;
  loop: boolean;
  monitor: boolean;
}

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

type HotkeyAction = "mute" | "deafen" | "ptt";

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
  const [directAddrInput, setDirectAddrInput] = useState("");
  const [directCodes, setDirectCodes] = useState<string[]>([]);
  const [isAdvancedOpen, setIsAdvancedOpen] = useState(false);
  const [isSettingsOpen, setIsSettingsOpen] = useState(false);
  const [isEditingNickname, setIsEditingNickname] = useState(false);
  const [nicknameInput, setNicknameInput] = useState("");
  const [vpsAddressInput, setVpsAddressInput] = useState("");
  const [peerVolumes, setPeerVolumes] = useState<Record<string, number>>({});
  const [copied, setCopied] = useState(false);
  const [capturingAction, setCapturingAction] = useState<HotkeyAction | null>(null);
  const [statusMsg, setStatusMsg] = useState<{ text: string; isError?: boolean } | null>(null);

  // Direct DOM ref for the real-time mic meter (0 React re-renders on 50 ms tick!)
  const micLevelRef = useRef<HTMLDivElement>(null);

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

    invoke<MusicStatus>("music_status")
      .then((s) => {
        setMusic(s);
        setVolumeDraft(s.volume);
      })
      .catch(() => {});

    invoke<AudioState>("get_audio_state")
      .then(setAudioState)
      .catch((e) => console.error("get_audio_state failed", e));

    invoke<RoomSnapshot>("get_room_state")
      .then((snap) => {
        setRoomState(snap.state);
        setPeers(snap.peers ?? []);
        setRoomCode(snap.room_code);
        setDirectCodes(snap.direct_codes);

        if (snap.diagnostics) setDiagnostics(snap.diagnostics);
      })
      .catch((e) => console.error("get_room_state failed", e));

    const unlistens: Array<() => void> = [];

    // Real-time Mic Level: DIRECT DOM MUTATION (Zero React re-render!)
    listen<number>("input_level", (ev) => {
      const db = ev.payload;
      const pct = Math.min(100, Math.max(0, (db + 60) * 1.67));
      if (micLevelRef.current) {
        micLevelRef.current.style.width = `${pct}%`;
      }
    }).then((u) => unlistens.push(u));

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

    listen<MusicStatus>("music_status_changed", (ev) => {
      setMusic(ev.payload);
      setVolumeDraft(ev.payload.volume);
    }).then((u) => unlistens.push(u));

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

      // Ignore pure modifier keys alone until a main key is pressed
      if (["Control", "Shift", "Alt", "Meta"].includes(e.key)) {
        return;
      }

      // Build key combo
      const parts: string[] = [];
      if (e.ctrlKey) parts.push("Ctrl");
      if (e.shiftKey) parts.push("Shift");
      if (e.altKey) parts.push("Alt");

      let key = e.key;
      if (e.code.startsWith("Key")) {
        key = e.code.replace(/^Key/, "").toUpperCase();
      } else if (e.code.startsWith("Digit")) {
        key = e.code.replace(/^Digit/, "");
      } else if (e.code === "Space") {
        key = "Space";
      } else if (key.length === 1) {
        key = key.toUpperCase();
      }

      parts.push(key);
      const combo = parts.join("+");

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
  const [music, setMusic] = useState<MusicStatus | null>(null);
  const [musicPanelOpen, setMusicPanelOpen] = useState(false);
  const [seekDraft, setSeekDraft] = useState<number | null>(null);
  const [volumeDraft, setVolumeDraft] = useState(1.0);
  const musicVolumeTimer = useRef<number | null>(null);

  const musicPosition = seekDraft !== null ? seekDraft : music?.position_ms ?? 0;
  const musicDuration = music?.duration_ms ?? 0;
  const musicPercent =
    musicDuration > 0
      ? Math.max(0, Math.min(100, (musicPosition / musicDuration) * 100))
      : 0;

  const refreshMusicStatus = () => {
    invoke<MusicStatus>("music_status")
      .then((s) => {
        setMusic(s);
        setVolumeDraft(s.volume);
      })
      .catch(console.error);
  };

  const pickAndOpenMusic = async () => {
    try {
      const path = await invoke<string | null>("music_pick_file");
      if (!path) return;
      const info = await invoke<{ title: string }>("music_open", { path });
      setStatusMsg({ text: `Loaded ${info.title}` });
      refreshMusicStatus();
    } catch (e: any) {
      setStatusMsg({ text: `Could not open file: ${e}`, isError: true });
    }
  };

  const toggleMusicPlay = async () => {
    try {
      if (music?.playing) {
        await invoke("music_pause");
      } else {
        await invoke("music_play");
      }
      refreshMusicStatus();
    } catch (e: any) {
      setStatusMsg({ text: `${e}`, isError: true });
    }
  };

  const stopMusic = async () => {
    await invoke("music_stop").catch(console.error);
    refreshMusicStatus();
  };

  const seekMusicTo = async (fraction: number) => {
    if (musicDuration <= 0) return;
    const ms = Math.round(fraction * musicDuration);
    setSeekDraft(null);
    await invoke("music_seek", { ms }).catch(console.error);
    refreshMusicStatus();
  };

  const setMusicVolume = (fraction: number) => {
    const v = Math.max(0, Math.min(2, fraction * 2));
    setVolumeDraft(v);
    if (musicVolumeTimer.current !== null) window.clearTimeout(musicVolumeTimer.current);
    musicVolumeTimer.current = window.setTimeout(() => {
      void invoke("music_set_volume", { v }).then(refreshMusicStatus).catch(console.error);
    }, 100);
  };

  const setMusicLoop = async (on: boolean) => {
    await invoke("music_set_loop", { on }).catch(console.error);
    refreshMusicStatus();
  };

  const setMusicMonitor = async (on: boolean) => {
    await invoke("music_set_monitor", { on }).catch(console.error);
    refreshMusicStatus();
  };

  const fractionFromEvent = (el: HTMLElement, clientX: number) => {
    const rect = el.getBoundingClientRect();
    return Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
  };

  const formatTime = (ms: number) => {
    const total = Math.max(0, Math.round(ms / 1000));
    return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
  };

  const handleSetVoiceQuality = async (kbps: number) => {
    await invoke("set_voice_quality", { kbps }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, voice_bitrate_kbps: kbps } : null));
    setStatusMsg({ text: `Voice quality set to ${kbps} kbps.` });
  };

  const handleSetMusicQuality = async (kbps: number) => {
    await invoke("set_music_quality", { kbps }).catch(console.error);
    setSettings((prev) => (prev ? { ...prev, music_bitrate_kbps: kbps } : null));
    setStatusMsg({ text: `Music quality set to ${kbps} kbps.` });
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

  const handleConnectDirect = async () => {
    if (!directAddrInput.trim()) return;
    try {
      setStatusMsg({ text: `Connecting to ${directAddrInput}...` });
      await invoke("connect_direct", { addr: directAddrInput.trim() });
    } catch (e: any) {
      setStatusMsg({ text: `Direct connect error: ${e}`, isError: true });
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



  const getInitials = (name?: string | null) => {
    if (!name) return "U";
    return name
      .trim()
      .split(/\s+/)
      .map((part) => part[0])
      .join("")
      .toUpperCase()
      .slice(0, 2);
  };

  const isPttMode = Boolean(settings?.hotkeys.ptt && settings.hotkeys.ptt.trim().length > 0);

  // Internal only: the transport label is no longer surfaced in the normal UI.
  const getTransportBadgeInfo = () => ({
    label: "Relay",
    styleClass: "cloud",
    hint: "Encrypted relay through the Verio server",
  });

  return (
    <div className="app-container">
            {/* Header - reference shell: brand / Ping / room chip */}
      <header className="header">
        <div className="brand">
          <div className="brand-mark">
            <img src={verioLogo} alt="" width={20} height={20} />
          </div>
          <span className="brand-word">Verio</span>
          <span className="brand-dot" />
        </div>

        <div className="header-right">
          <div className="rtt">
            <span>Ping</span>
            <b>{rttMs !== null ? `${rttMs} ms` : "--"}</b>
          </div>

          {isInCall && roomCode ? (
            <button className="room-chip" title="Copy room code" onClick={handleCopyCode}>
              <span className="label">Room</span>
              <span>{roomCode}</span>
              <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
                <rect x="2" y="2" width="6" height="6" rx="1" stroke="currentColor" strokeWidth="1.2" />
                <path d="M4 8v2h6V4H8" stroke="currentColor" strokeWidth="1.2" strokeLinecap="round" />
              </svg>
            </button>
          ) : null}

          {/* Connection state; still the diagnostics toggle while in a call */}
          <button
            className="status-pill"
          >
            <span className={`dot ${roomState}`} />
            <span>
              {roomState === "connected"
                ? "Connected"
                : roomState === "connecting"
                ? "Connecting"
                : "Idle"}
            </span>
          </button>
        </div>
      </header>

      {/* Main Content */}
      <main className="main">
        {statusMsg && (
          <div className={`toast-banner ${statusMsg.isError ? "error" : ""}`}>
            <span>{statusMsg.text}</span>
            <button className="toast-close" onClick={() => setStatusMsg(null)}>
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
            </button>
          </div>
        )}

        {isInCall ? (
          /* IN-CALL VIEW */
          <div className="incall-container">
            <div className="incall-header">
              <div className="incall-header-left">
                <div className="room-tag">
                  <span style={{ fontSize: "0.82rem", color: "var(--text-muted)" }}>Room</span>
                  <span className="room-number">{roomCode || "----"}</span>
                  {roomCode && (
                    <button className="secondary-btn" style={{ height: 28, fontSize: "0.75rem" }} onClick={handleCopyCode}>
                      {copied ? "Copied" : "Copy Code"}
                    </button>
                  )}
                </div>
              </div>

              <button className="danger-btn" onClick={handleLeaveCall}>
                Leave Call
              </button>
            </div>

            {/* Live Network Diagnostics Drawer / Card */}
            {isDiagnosticsOpen && (
              <div className="diag-drawer">
                <div className="diagnostics-panel-header">
                  <div className="diagnostics-panel-title">
                    <span className="diagnostics-pulse" />
                    <span>Real-Time Network Diagnostics</span>
                  </div>
                  <button
                    className="diagnostics-close-btn"
                    onClick={() => setIsDiagnosticsOpen(false)}
                  >
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                  </button>
                </div>

                <div className="diagnostics-metrics-grid">
                  <div className="diag-row">
                    <span className="diag-label">Transport Path</span>
                    <span className="diag-value highlight">{getTransportBadgeInfo().label}</span>
                    <span className="diag-sub">{getTransportBadgeInfo().hint}</span>
                  </div>

                  <div className="diag-row">
                    <span className="diag-label">Latency (RTT)</span>
                    <span className="diag-value">
                      {diagnostics?.rtt_ms !== null && diagnostics?.rtt_ms !== undefined
                        ? `${Math.round(diagnostics.rtt_ms)} ms`
                        : rttMs !== null
                        ? `${rttMs} ms`
                        : "Measuring..."}
                    </span>
                    <span className="diag-sub">Round-trip time</span>
                  </div>

                  <div className="diag-row">
                    <span className="diag-label">Packets In / Out</span>
                    <span className="diag-value">
                      out {diagnostics?.packets_sent ?? 0} / in {diagnostics?.packets_recv ?? 0}
                    </span>
                    <span className="diag-sub">Voice datagrams</span>
                  </div>

                  <div className="diag-row">
                    <span className="diag-label">Last Packet Recv</span>
                    <span
                      className={`diag-value ${
                        (diagnostics?.last_recv_ms_ago ?? 0) > 2000 ? "warn" : ""
                      }`}
                    >
                      {diagnostics?.last_recv_ms_ago !== null && diagnostics?.last_recv_ms_ago !== undefined
                        ? `${diagnostics.last_recv_ms_ago} ms ago`
                        : "Streaming"}
                    </span>
                    <span className="diag-sub">
                      {(diagnostics?.last_recv_ms_ago ?? 0) > 2000
                        ? " High jitter / Delayed"
                        : "Healthy inbound stream"}
                    </span>
                  </div>

                  <div className="diag-row">
                    <span className="diag-label">Relay Packets S / R</span>
                    <span className="diag-value">
                      {diagnostics?.relay_packets_sent ?? 0} / {diagnostics?.relay_packets_recv ?? 0}
                    </span>
                    <span className="diag-sub">Relay datagrams sent / received</span>
                  </div>

                  <div className="diag-row">
                    <span className="diag-label">Last Relay Recv</span>
                    <span
                      className={`diag-value ${
                        (diagnostics?.last_recv_ms_ago ?? 0) > 2000 ? "warn" : ""
                      }`}
                    >
                      {diagnostics?.last_recv_ms_ago !== null && diagnostics?.last_recv_ms_ago !== undefined
                        ? `${diagnostics.last_recv_ms_ago} ms ago`
                        : "Streaming"}
                    </span>
                    <span className="diag-sub">
                      {(diagnostics?.last_recv_ms_ago ?? 0) > 2000
                        ? "No relay packets - firewall / NAT / wrong address"
                        : "Healthy relay inbound stream"}
                    </span>
                  </div>

                  <div className="diag-row">
                    <span className="diag-label">Relay Mode</span>
                    <span className="diag-value highlight">
                      {diagnostics?.relay_mode || "direct"}
                    </span>
                    <span className="diag-sub">
                      {diagnostics?.relay_bytes_sent ?? 0} B out / {diagnostics?.relay_bytes_recv ?? 0} B in
                    </span>
                  </div>

                  <div className="diag-row span-2">
                    <span className="diag-label">Remote Endpoint</span>
                    <span className="diag-value mono">
                      {diagnostics?.remote_endpoint || "141.11.1.110:3478"}
                    </span>
                    <span className="diag-sub">Active destination address</span>
                  </div>

                  <div className="diag-row span-2">
                    <span className="diag-label">Local Socket</span>
                    <span className="diag-value mono">
                      {diagnostics?.local_addr || "0.0.0.0"}
                    </span>
                    <span className="diag-sub">Bound UDP interface</span>
                  </div>
                </div>

                {diagnostics?.last_error && (
                  <div className="diag-error-row">
                    <span className="diag-error-icon"></span>
                    <span className="diag-error-msg">{diagnostics.last_error}</span>
                  </div>
                )}
              </div>
            )}

            <div className="section-label">In call - {peers.length + 1} of 10</div>

            <div className="peers">
              {/* Local User */}
              <div
                className={`peer ${audioState.speaking ? "is-speaking" : ""} ${
                  audioState.muted ? "is-muted" : ""
                }`}
              >
                <div className="peer-avatar">
                  {getInitials(settings?.nickname || "You")}
                </div>
                <div className="peer-body">
                  <span className="peer-name">{settings?.nickname || "You"}</span>
                  <span className="pill-tag">You</span>
                </div>
                <div className="card-badges">
                  {audioState.muted && <span className="badge muted">Muted</span>}
                  {audioState.deafened && <span className="badge deafened">Deafened</span>}
                </div>
              </div>

              {/* Remote peers (one card each) */}
              {peers.map((p) => (
                <div
                  key={p.uuid}
                  className={`peer ${p.speaking ? "is-speaking" : ""}`}
                >
                  <div className="peer-avatar">{getInitials(p.name)}</div>
                  <div className="peer-body">
                    <span className="peer-name">{p.name}</span>
                  </div>
                  <div className="card-badges">
                    {p.mute && <span className="badge muted">Muted</span>}
                    {p.deafen && <span className="badge deafened">Deafened</span>}
                    {p.music && (
                      <span className="badge music" title={p.music_title || "Playing music"}>
                        {"\u266a "}
                        {p.music_title || "Music"}
                      </span>
                    )}
                  </div>
                  <div className="volume-row">
                    <div className="volume-meta">
                      <span>Volume</span>
                      <span>{Math.round((peerVolumes[p.uuid] ?? 1) * 100)}%</span>
                    </div>
                    <input
                      type="range"
                      min="0"
                      max="2"
                      step="0.05"
                      value={peerVolumes[p.uuid] ?? 1}
                      onChange={(e) =>
                        handlePeerVolumeChange(p.uuid, parseFloat(e.target.value))
                      }
                      className="slider-control"
                    />
                  </div>
                </div>
              ))}

              {peers.length === 0 && (
                <div className="peer" style={{ borderStyle: "dashed", opacity: 0.6 }}>
                  <div className="peer-avatar" style={{ background: "transparent" }}>
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
                  </div>
                  <div className="peer-body">
                    <span style={{ fontSize: "0.85rem", color: "var(--text-muted)" }}>Waiting for others</span>
                  </div>
                  <span style={{ fontSize: "0.75rem", color: "var(--text-muted)", marginTop: 4 }}>
                    Share room code <strong style={{ color: "var(--text-primary)" }}>{roomCode}</strong>
                  </span>
                </div>
              )}
            </div>
          </div>
        ) : (
          /* LOBBY VIEW */
          <div className="lobby-container">
            <div className="lobby-hero">
              <h1>Instant Voice</h1>
              <p>Create a room or enter a 4-digit code to start talking.</p>
            </div>

            <div className="lobby-grid">
              <div className="minimal-card">
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 6 }}>
                  <div className="card-title" style={{ marginBottom: 0 }}>Host Call</div>
                  <button
                    className="mode-indicator-btn"
                    onClick={() => setIsSettingsOpen(true)}
                    title="Change voice transport mode in Settings"
                  >
                    {getTransportBadgeInfo().label} 
                  </button>
                </div>
                <div className="card-desc">
                  Generate a 4-digit room code ({getTransportBadgeInfo().hint}).
                </div>
                <button
                  className="primary-btn"
                  onClick={handleCreateRoom}
                  disabled={roomState === "connecting"}
                >
                  Create Room
                </button>
              </div>

              <div className="minimal-card">
                <div className="card-title">Join Call</div>
                <div className="card-desc">Enter the 4-digit room code shared by your host.</div>
                <div className="code-input-row">
                  <div className="otp-group">
                    {[0, 1, 2, 3].map((i) => (
                      <input
                        key={i}
                        type="text"
                        inputMode="numeric"
                        maxLength={1}
                        className="otp-box"
                        value={joinCodeInput[i] ?? ""}
                        aria-label={`Room code digit ${i + 1}`}
                        onChange={(e) => {
                          const d = e.target.value.replace(/[^0-9]/g, "").slice(-1);
                          const arr = joinCodeInput.split("");
                          arr[i] = d;
                          setJoinCodeInput(arr.join("").slice(0, 4));
                          if (d) {
                            (e.target.nextElementSibling as HTMLInputElement | null)?.focus();
                          }
                        }}
                        onKeyDown={(e) => {
                          if (e.key === "Backspace" && !joinCodeInput[i]) {
                            (e.currentTarget.previousElementSibling as HTMLInputElement | null)?.focus();
                          }
                        }}
                      />
                    ))}
                  </div>
                  <button
                    className="primary-btn"
                    style={{ flex: 1 }}
                    onClick={handleJoinRoom}
                    disabled={joinCodeInput.trim().length !== 4 || roomState === "connecting"}
                  >
                    Join
                  </button>
                </div>
              </div>
            </div>

            {/* Direct LAN Connect: hidden from the product UI */}
            {false && (
            <div className="accordion">
              <div className="accordion-header" onClick={() => setIsAdvancedOpen(!isAdvancedOpen)}>
                <span>Direct LAN Connect</span>
                <span>{isAdvancedOpen ? "-" : "+"}</span>
              </div>
              {isAdvancedOpen && (
                <div className="accordion-body">
                  <div style={{ display: "flex", gap: 8, marginBottom: 8 }}>
                    <input
                      type="text"
                      placeholder="192.168.1.50:49860"
                      value={directAddrInput}
                      onChange={(e) => setDirectAddrInput(e.target.value)}
                      className="text-input"
                      style={{ flex: 1 }}
                    />
                    <button className="secondary-btn" onClick={handleConnectDirect}>
                      Connect
                    </button>
                  </div>
                  {directCodes.length > 0 && (
                    <div style={{ fontSize: "0.72rem", color: "var(--text-muted)" }}>
                      <span>Your LAN addresses: </span>
                      {directCodes.map((code) => (
                        <span key={code} style={{ marginRight: 6, color: "var(--text-secondary)" }}>
                          {code}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              )}
            </div>
            )}
          </div>
        )}
      </main>

      {/* Bottom Dock */}
      {/* Session 13: music player panel (ported from design/music-player) */}
            {musicPanelOpen && (
              <div className="section-label">Now playing</div>
            )}

            {musicPanelOpen && (
              <div className="music-panel-wrapper">
                <section className="music-panel" aria-label="Music Player Controls">
                  <div className="music-header-row">
                    <div className="music-header-left">
                      <svg className="music-header-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                        <path d="M9 18V5l12-2v13" />
                        <circle cx="6" cy="18" r="3" />
                        <circle cx="18" cy="16" r="3" />
                      </svg>
                      <span className="music-header-title">{music?.active ? "Now Playing" : "Music"}</span>
                    </div>
                    <div className="music-header-right">
                      <div
                        className={`status-pill ${music?.active ? "status-sending" : music?.playing ? "status-paused" : "status-idle"}`}
                        aria-live="polite"
                      >
                        <span className="status-dot" />
                        <span>{music?.active ? "Sending to room" : music?.playing ? "Paused" : "Idle"}</span>
                      </div>
                      <button
                        className="btn-panel-close"
                        onClick={() => setMusicPanelOpen(false)}
                        aria-label="Collapse music player panel"
                        title="Collapse panel"
                      >
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
                          <line x1="18" y1="6" x2="6" y2="18" />
                          <line x1="6" y1="6" x2="18" y2="18" />
                        </svg>
                      </button>
                    </div>
                  </div>

                  <div className="music-track-row">
                    {music?.title ? (
                      <div className="track-meta">
                        <span className="track-title">{music.title}</span>
                        <span className="track-artist">
                          {music.path ? `- ${music.path.split(/[\\/]/).pop()}` : ""}
                        </span>
                      </div>
                    ) : (
                      <div className="track-idle-placeholder">No track loaded</div>
                    )}
                  </div>

                  <div className="music-progress-row">
                    <div
                      className="seek-container"
                      role="slider"
                      aria-label="Track playback position"
                      aria-valuemin={0}
                      aria-valuemax={Math.max(1, Math.round(musicDuration / 1000))}
                      aria-valuenow={Math.round(musicPosition / 1000)}
                      tabIndex={0}
                      onPointerDown={(e) =>
                        setSeekDraft(fractionFromEvent(e.currentTarget, e.clientX) * musicDuration)
                      }
                      onPointerUp={(e) =>
                        void seekMusicTo(fractionFromEvent(e.currentTarget, e.clientX))
                      }
                    >
                      <div className="seek-track">
                        <div className="seek-fill" style={{ width: `${musicPercent}%` }}>
                          <div className="seek-handle" />
                        </div>
                      </div>
                    </div>
                    <div className="time-display" aria-label="Current and total track duration">
                      {formatTime(musicPosition)} / {formatTime(musicDuration)}
                    </div>
                  </div>

                  <div className="music-controls-row">
                    <div className="controls-left-group">
                      <button
                        className="btn-play-pause"
                        onClick={() => void toggleMusicPlay()}
                        aria-label={music?.playing ? "Pause track" : "Play track"}
                        title="Play / Pause"
                      >
                        {music?.playing ? (
                          <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
                            <rect x="6" y="4" width="4" height="16" />
                            <rect x="14" y="4" width="4" height="16" />
                          </svg>
                        ) : (
                          <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
                            <polygon points="6,4 20,12 6,20" />
                          </svg>
                        )}
                      </button>

                      <button
                        className="btn-transport"
                        onClick={() => void stopMusic()}
                        aria-label="Stop playback"
                        title="Stop"
                      >
                        <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
                          <rect x="6" y="6" width="12" height="12" rx="1.5" />
                        </svg>
                      </button>

                      <button
                        className={`btn-transport btn-loop ${music?.loop ? "is-active" : ""}`}
                        onClick={() => void setMusicLoop(!music?.loop)}
                        aria-label="Loop playback"
                        title="Toggle repeat"
                        aria-pressed={Boolean(music?.loop)}
                      >
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeLinecap="round" strokeLinejoin="round">
                          <polyline points="17 1 21 5 17 9" />
                          <path d="M3 11V9a4 4 0 0 1 4-4h14" />
                          <polyline points="7 23 3 19 7 15" />
                          <path d="M21 13v2a4 4 0 0 1-4 4H3" />
                        </svg>
                      </button>

                      <div className="volume-group">
                        <button className="volume-icon-btn" aria-label="Volume setting" title="Volume">
                          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
                            <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
                            <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
                          </svg>
                        </button>
                        <div
                          className="volume-slider-box"
                          role="slider"
                          aria-label="Music volume"
                          aria-valuemin={0}
                          aria-valuemax={200}
                          aria-valuenow={Math.round(volumeDraft * 100)}
                          tabIndex={0}
                          onPointerDown={(e) =>
                            setMusicVolume(fractionFromEvent(e.currentTarget, e.clientX))
                          }
                        >
                          <div className="volume-slider-track">
                            <div
                              className="volume-slider-fill"
                              style={{ width: `${(volumeDraft / 2) * 100}%` }}
                            >
                              <div className="volume-slider-thumb" />
                            </div>
                          </div>
                        </div>
                        <span className="volume-label">{Math.round(volumeDraft * 100)}%</span>
                      </div>
                    </div>

                    <div className="controls-right-group">
                      {audioState.deafened && music?.active ? (
                        <div
                          className="local-muted-badge"
                          title="Your client is deafened; music monitor is silenced locally"
                        >
                          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                            <line x1="1" y1="1" x2="23" y2="23" />
                            <path d="M9 9v3a3 3 0 0 0 5.12 2.12M15 9.34V4a3 3 0 0 0-5.94-.6" />
                            <path d="M17 16.95A7 7 0 0 1 5 12v-2m14 0v2a7 7 0 0 1-.11 1.23" />
                          </svg>
                          <span>Muted locally</span>
                        </div>
                      ) : null}

                      <div className="monitor-control-wrapper">
                        <button
                          className={`monitor-toggle ${music?.monitor ? "is-active" : ""}`}
                          onClick={() => void setMusicMonitor(!music?.monitor)}
                          role="switch"
                          aria-checked={Boolean(music?.monitor)}
                          aria-label="Toggle local monitor playback"
                        >
                          <span>Local Monitor</span>
                          <div className="monitor-switch-pip">
                            <div className="monitor-switch-thumb" />
                          </div>
                        </button>
                        <div className="monitor-tooltip" role="tooltip">
                          Feedback warning: Keep headphones on when monitor is active
                        </div>
                      </div>

                      <button
                        className="btn-open-file"
                        onClick={() => void pickAndOpenMusic()}
                        aria-label="Open audio file from disk"
                      >
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                        </svg>
                        <span>Open File.</span>
                      </button>
                    </div>
                  </div>
                </section>
              </div>
            )}


            <footer className="dock">

        <div className="dock-left">
          <div className="me">
            {(settings?.nickname || "User").slice(0, 1).toUpperCase()}
          </div>
          <div className="me-meta">
            {isEditingNickname ? (
              <div style={{ display: "flex", gap: 4 }}>
                <input
                  type="text"
                  value={nicknameInput}
                  onChange={(e) => setNicknameInput(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleSaveNickname();
                    if (e.key === "Escape") setIsEditingNickname(false);
                  }}
                  className="text-input"
                  style={{ height: 26, padding: "0 6px", fontSize: "0.8rem", width: 100 }}
                  autoFocus
                />
                <button className="secondary-btn" style={{ height: 26, padding: "0 8px" }} onClick={handleSaveNickname}>
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><polyline points="20 6 9 17 4 12" /></svg>
                </button>
              </div>
            ) : (
              <>
                <span className="me-name">{settings?.nickname || "User"}</span>
                <span className={`dock-mode-badge ${isPttMode ? "ptt" : "open"}`}>
                  {isPttMode ? `Push to talk (${settings?.hotkeys.ptt})` : "Open mic"}
                </span>
                <button
                  onClick={() => setIsEditingNickname(true)}
                  className="dock-btn"
                  style={{ padding: "2px 8px", height: 22 }}
                  title="Change your display name"
                >
                  <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true"><path d="M12 20h9" /><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" /></svg>
                </button>
              </>
            )}
          </div>
          {/* Real-time mic meter: direct DOM reference */}
          <div className="mic-meter-track" title="Microphone Level">
            <div ref={micLevelRef} className="mic-meter-fill" />
          </div>
        </div>

        {/* Session 13: dock music button + glanceable strip */}
              <button
                className={`btn-dock-music ${musicPanelOpen ? "panel-is-open" : ""}`}
                onClick={() => setMusicPanelOpen((v) => !v)}
                title="Music player"
                aria-label="Toggle music player panel"
              >
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <path d="M9 18V5l12-2v13" />
                  <circle cx="6" cy="18" r="3" />
                  <circle cx="18" cy="16" r="3" />
                </svg>
              </button>

              {(music?.active || music?.playing) && (
                <div
                  className="dock-music-strip"
                  onClick={() => setMusicPanelOpen(true)}
                  title={music?.title ? `${music.title} - ${music.path ?? ""}` : "Music"}
                >
                  <div className="strip-play-glyph">
                    {music?.playing ? (
                      <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
                        <rect x="6" y="4" width="4" height="16" />
                        <rect x="14" y="4" width="4" height="16" />
                      </svg>
                    ) : (
                      <svg viewBox="0 0 24 24" fill="currentColor" stroke="none">
                        <polygon points="6,4 20,12 6,20" />
                      </svg>
                    )}
                  </div>
                  <div className="strip-info">
                    <div className="strip-title">{music?.title || "Music"}</div>
                    <div className="strip-progress-bar">
                      <div className="strip-progress-fill" style={{ width: `${musicPercent}%` }} />
                    </div>
                  </div>
                </div>
              )}

              
              <div className="dock-right">

          {/* Mute Button */}
          <button
            className={`dock-btn ${audioState.muted ? "muted" : ""}`}
            onClick={handleToggleMute}
            title={`Toggle Mute (${settings?.hotkeys.mute || "Ctrl+Shift+M"})`}
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              {audioState.muted ? (
                <>
                  <line x1="2" y1="2" x2="22" y2="22" />
                  <path d="M18.89 13.23A7.12 7.12 0 0 0 19 12v-2" />
                  <path d="M5 10v2a7 7 0 0 0 12 5" />
                  <line x1="12" y1="19" x2="12" y2="22" />
                  <line x1="8" y1="22" x2="16" y2="22" />
                </>
              ) : (
                <>
                  <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
                  <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
                  <line x1="12" y1="19" x2="12" y2="22" />
                  <line x1="8" y1="22" x2="16" y2="22" />
                </>
              )}
            </svg>
            <span>Mic</span>
            <span className="kbd">
              {(settings?.hotkeys.mute || "M").replace(/^Ctrl\+Shift\+/, "")}
            </span>
          </button>

          {/* Deafen Button */}
          <button
            className={`dock-btn ${audioState.deafened ? "deafened" : ""}`}
            onClick={handleToggleDeafen}
            title={`Toggle Deafen (${settings?.hotkeys.deafen || "Ctrl+Shift+D"})`}
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M3 18v-6a9 9 0 0 1 18 0v6" />
              <path d="M21 19a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3zM3 19a2 2 0 0 0 2 2h1a2 2 0 0 0 2-2v-3a2 2 0 0 0-2-2H3z" />
              {audioState.deafened && <line x1="2" y1="2" x2="22" y2="22" />}
            </svg>
            <span>Deafen</span>
          </button>

          {/* Settings Gear */}
          <button
            className="dock-btn"
            onClick={() => setIsSettingsOpen(true)}
            title="Settings"
            aria-label="Settings"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" />
              <circle cx="12" cy="12" r="3" />
            </svg>
          </button>
        </div>
      </footer>

      {/* Settings Modal */}
      {isSettingsOpen && (
        <div className="settings-overlay" onClick={() => setIsSettingsOpen(false)}>
          <div className="settings-panel" onClick={(e) => e.stopPropagation()}>
            <div className="modal-header">
              <h2>Settings</h2>
              <button
                style={{ background: "none", border: "none", color: "var(--text-muted)", cursor: "pointer", fontSize: "1.2rem" }}
                onClick={() => setIsSettingsOpen(false)}
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
              </button>
            </div>

            <div className="modal-body">
              {/* Transmission Mode Section */}
              <div className="settings-section">
                <div className="settings-section-title">Transmission Mode</div>
                
                {/* Premium Segmented Control */}
                <div className="segmented-control">
                  <button
                    type="button"
                    className={`segmented-item ${!isPttMode ? "active" : ""}`}
                    onClick={handleSelectVoiceActivity}
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" />
                      <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
                    </svg>
                    <span>Voice Activity</span>
                  </button>

                  <button
                    type="button"
                    className={`segmented-item ${isPttMode ? "active" : ""}`}
                    onClick={handleSelectPushToTalk}
                  >
                    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                      <rect x="2" y="4" width="20" height="16" rx="2" />
                      <path d="M6 8h.001M10 8h.001M14 8h.001M18 8h.001M8 12h.001M12 12h.001M16 12h.001M7 16h10" />
                    </svg>
                    <span>Push to Talk</span>
                  </button>
                </div>

                {isPttMode && (
                  <div className="settings-row" style={{ marginTop: 8 }}>
                    <div className="settings-meta">
                      <span className="settings-label">PTT Key</span>
                      <span className="settings-hint">Hold this key to transmit your voice</span>
                    </div>
                    <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                      <button
                        className={`keybind-chip ${capturingAction === "ptt" ? "recording" : ""}`}
                        onClick={() => setCapturingAction("ptt")}
                        title="Click to change keybind"
                      >
                        {capturingAction === "ptt" ? "Press key..." : settings?.hotkeys.ptt || "Press key..."}
                      </button>
                      <button
                        className="keybind-clear-btn"
                        onClick={handleSelectVoiceActivity}
                        title="Clear and switch to Voice Activity"
                      >
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                      </button>
                    </div>
                  </div>
                )}
              </div>

              {/* Voice Transport Section */}
              <div className="settings-section">
                <div className="settings-section-title">Voice Transport</div>
                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Encrypted relay</span>
                    <span className="settings-hint">
                      Every call runs through the Verio relay server. Nothing to configure.
                    </span>
                  </div>
                  <span className="badge cloud-badge">Active</span>
                </div>



              </div>

              {/* Hotkeys Section */}
              <div className="settings-section">
                <div className="settings-section-title">Global Shortcuts</div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Toggle Mute</span>
                    <span className="settings-hint">Mutes/unmutes your microphone</span>
                  </div>
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <button
                      className={`keybind-chip ${capturingAction === "mute" ? "recording" : ""}`}
                      onClick={() => setCapturingAction("mute")}
                    >
                      {capturingAction === "mute" ? "Press key..." : settings?.hotkeys.mute || "None"}
                    </button>
                    {settings?.hotkeys.mute && (
                      <button
                        className="keybind-clear-btn"
                        onClick={async () => {
                          await invoke("set_hotkey", { action: "mute", combo: "" });
                          setSettings((prev) => prev ? { ...prev, hotkeys: { ...prev.hotkeys, mute: "" } } : null);
                        }}
                      >
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                      </button>
                    )}
                  </div>
                </div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Toggle Deafen</span>
                    <span className="settings-hint">Mutes incoming sound and mic</span>
                  </div>
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <button
                      className={`keybind-chip ${capturingAction === "deafen" ? "recording" : ""}`}
                      onClick={() => setCapturingAction("deafen")}
                    >
                      {capturingAction === "deafen" ? "Press key..." : settings?.hotkeys.deafen || "None"}
                    </button>
                    {settings?.hotkeys.deafen && (
                      <button
                        className="keybind-clear-btn"
                        onClick={async () => {
                          await invoke("set_hotkey", { action: "deafen", combo: "" });
                          setSettings((prev) => prev ? { ...prev, hotkeys: { ...prev.hotkeys, deafen: "" } } : null);
                        }}
                      >
                        <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true"><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
                      </button>
                    )}
                  </div>
                </div>
              </div>

              {/* Audio Devices Section */}
              <div className="settings-section">
                <div className="settings-section-title">Audio Devices</div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Input Device</span>
                  </div>
                  <select
                    className="custom-select"
                    value={settings?.input_device || ""}
                    onChange={async (e) => {
                      const val = e.target.value || null;
                      await invoke("set_input_device", { name: val });
                      setSettings((prev) => prev ? { ...prev, input_device: val } : null);
                    }}
                  >
                    <option value="">Default Input Device</option>
                    {devices.inputs.map((d) => (
                      <option key={d.name} value={d.name}>
                        {d.name} {d.is_default ? "(Default)" : ""}
                      </option>
                    ))}
                  </select>
                </div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Output Device</span>
                  </div>
                  <select
                    className="custom-select"
                    value={settings?.output_device || ""}
                    onChange={async (e) => {
                      const val = e.target.value || null;
                      await invoke("set_output_device", { name: val });
                      setSettings((prev) => prev ? { ...prev, output_device: val } : null);
                    }}
                  >
                    <option value="">Default Output Device</option>
                    {devices.outputs.map((d) => (
                      <option key={d.name} value={d.name}>
                        {d.name} {d.is_default ? "(Default)" : ""}
                      </option>
                    ))}
                  </select>
                </div>
              </div>

              {/* Audio Quality Section (Session 14) */}
              <div className="settings-section">
                <div className="settings-section-title">Audio Quality</div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Voice quality</span>
                    <span className="settings-hint">Encoder bitrate used for speech</span>
                  </div>
                  <select
                    className="text-input"
                    style={{ width: 132, height: 32 }}
                    value={settings?.voice_bitrate_kbps ?? 32}
                    onChange={(e) => void handleSetVoiceQuality(parseInt(e.target.value))}
                    aria-label="Voice encoder bitrate"
                  >
                    <option value={24}>24 kbps (low)</option>
                    <option value={32}>32 kbps (default)</option>
                    <option value={48}>48 kbps</option>
                    <option value={64}>64 kbps</option>
                    <option value={96}>96 kbps</option>
                  </select>
                </div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Music quality</span>
                    <span className="settings-hint">
                      Bitrate while music is playing - music uses this instead of the voice setting
                    </span>
                  </div>
                  <select
                    className="text-input"
                    style={{ width: 132, height: 32 }}
                    value={settings?.music_bitrate_kbps ?? 96}
                    onChange={(e) => void handleSetMusicQuality(parseInt(e.target.value))}
                    aria-label="Music encoder bitrate"
                  >
                    <option value={64}>64 kbps</option>
                    <option value={96}>96 kbps (default)</option>
                    <option value={128}>128 kbps</option>
                    <option value={160}>160 kbps</option>
                    <option value={192}>192 kbps</option>
                  </select>
                </div>
              </div>

              {/* Audio Processing Section */}
              <div className="settings-section">
                <div className="settings-section-title">Voice Processing</div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Input Gain</span>
                    <span className="settings-hint">+{Math.round(settings?.input_gain_db || 0)} dB</span>
                  </div>
                  <input
                    type="range"
                    min="0"
                    max="30"
                    step="1"
                    value={settings?.input_gain_db || 0}
                    onChange={async (e) => {
                      const val = parseFloat(e.target.value);
                      await invoke("set_input_gain", { gainDb: val });
                      setSettings((prev) => prev ? { ...prev, input_gain_db: val } : null);
                    }}
                    className="slider-control"
                    style={{ width: 120 }}
                  />
                </div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">AI Noise Suppression</span>
                    <span className="settings-hint">RNNoise deep learning background noise filter</span>
                  </div>
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={settings?.noise_suppression || false}
                      onChange={async (e) => {
                        const val = e.target.checked;
                        await invoke("set_noise_suppression", { on: val });
                        setSettings((prev) => prev ? { ...prev, noise_suppression: val } : null);
                      }}
                    />
                    <span className="switch-track" />
                  </label>
                </div>

                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">Loopback Test</span>
                    <span className="settings-hint">Hear your microphone output locally</span>
                  </div>
                  <label className="switch">
                    <input
                      type="checkbox"
                      checked={settings?.loopback || false}
                      onChange={async (e) => {
                        const val = e.target.checked;
                        await invoke("set_loopback", { on: val });
                        setSettings((prev) => prev ? { ...prev, loopback: val } : null);
                      }}
                    />
                    <span className="switch-track" />
                  </label>
                </div>
              </div>

              {/* Server Section */}
              <div className="settings-section">
                <div className="settings-section-title">Signaling Server</div>
                <div className="settings-row">
                  <div className="settings-meta">
                    <span className="settings-label">VPS Address</span>
                    <span className="settings-hint">WebSocket signaling host URL</span>
                  </div>
                  <div style={{ display: "flex", gap: 6 }}>
                    <input
                      type="text"
                      value={vpsAddressInput}
                      onChange={(e) => setVpsAddressInput(e.target.value)}
                      className="text-input"
                      style={{ height: 32, width: 160 }}
                    />
                    <button className="secondary-btn" style={{ height: 32 }} onClick={handleSaveVpsAddress}>
                      Save
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>
      )}

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
  );
}
