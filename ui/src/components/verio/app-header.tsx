import { invoke } from "@tauri-apps/api/core";
import { cn } from "@/lib/utils";
import {
  IconCheck,
  IconCopy,
  IconMessageSquare,
  IconMinus,
  IconWindowMaximize,
  IconX,
  IconZap,
} from "@/components/verio/icons";
import verioLogo from "@/assets/verio-logo-primary-96.png";
import type { NavView } from "@/components/verio/left-rail";

type RoomState = "idle" | "connecting" | "connected" | "closed";

export interface UpdateAvailableInfo {
  version: string;
  current_version: string;
  date: string | null;
  body: string | null;
}

export interface AppHeaderProps {
  activeView: NavView;
  rttMs: number | null;
  state: RoomState;
  roomCode?: string | null;
  isInCall: boolean;
  elapsedSeconds: number;
  copied?: boolean;
  onCopyCode: () => void;
  onJoinMainRoom: () => void;
  chatOpen: boolean;
  onToggleChat: () => void;
  unreadChatCount?: number;
  availableUpdate?: UpdateAvailableInfo | null;
  onOpenUpdateDialog?: () => void;
}

const VIEW_LABELS: Record<NavView, string> = {
  stage: "ROOM",
  music: "MUSIC",
  settings: "SETTINGS",
};

const STATE_STYLE: Record<RoomState, { label: string; dot: string; text: string }> = {
  idle: { label: "Idle", dot: "bg-[#94a3b8]", text: "text-text-dim" },
  connecting: {
    label: "Connecting",
    dot: "bg-accent shadow-[0_0_8px_rgba(232,255,58,0.85)]",
    text: "text-accent",
  },
  connected: {
    label: "Connected",
    dot: "bg-[#10b981] shadow-[0_0_8px_rgba(16,185,129,0.75)]",
    text: "text-[#10b981]",
  },
  closed: { label: "Closed", dot: "bg-warn", text: "text-warn" },
};

function formatTimer(totalSeconds: number): string {
  const s = Math.max(0, Math.floor(totalSeconds));
  const hrs = Math.floor(s / 3600);
  const mins = Math.floor((s % 3600) / 60);
  const secs = s % 60;
  return `${String(hrs).padStart(2, "0")}:${String(mins).padStart(2, "0")}:${String(secs).padStart(2, "0")}`;
}

export function AppHeader({
  activeView,
  rttMs,
  state,
  roomCode,
  isInCall,
  elapsedSeconds,
  copied = false,
  onCopyCode,
  onJoinMainRoom,
  chatOpen,
  onToggleChat,
  unreadChatCount = 0,
  availableUpdate = null,
  onOpenUpdateDialog,
}: AppHeaderProps) {
  const connection = STATE_STYLE[state] ?? STATE_STYLE.idle;
  const inMainRoom = isInCall && roomCode === "1111";

  const handleHeaderMouseDown = (e: React.MouseEvent<HTMLElement>) => {
    if (e.button !== 0) return;
    if ((e.target as HTMLElement).closest("button, input, select, textarea, a")) {
      return;
    }
    void invoke("window_start_drag").catch(() => {});
  };

  return (
    <header
      data-tauri-drag-region
      onMouseDown={handleHeaderMouseDown}
      className={cn(
        "relative z-30 flex h-[48px] shrink-0 items-center justify-between gap-4",
        "rounded-2xl border border-border bg-[var(--bg-titlebar-glass)] px-4 backdrop-blur-2xl",
      )}
    >
      {/* Left: Brand + active view */}
      <div className="pointer-events-none flex min-w-0 items-center gap-2.5">
        <img
          src={verioLogo}
          alt=""
          width={24}
          height={24}
          className="size-6 shrink-0 rounded-lg"
        />
        <span className="font-mono text-13 font-bold uppercase tracking-[0.22em] text-text">
          Verio
        </span>
        <span aria-hidden="true" className="text-text-faint">
          /
        </span>
        <span className="mono-num truncate text-11 font-medium uppercase tracking-chip text-lavender-bright">
          {VIEW_LABELS[activeView]}
        </span>
      </div>

      {/* Center: Single authoritative Room Code + Copy + Timer (or quick Main #1111 jump when idle) */}
      <div className="flex shrink-0 items-center gap-2">
        {isInCall ? (
          <>
            <div
              className={cn(
                "inline-flex h-8 items-center gap-2 rounded-full border px-3",
                inMainRoom
                  ? "border-[rgba(232,255,58,0.45)] bg-[rgba(232,255,58,0.14)] text-accent"
                  : "border-border-2 bg-[var(--bg-input)] text-text",
              )}
            >
              <span
                aria-hidden="true"
                className={cn(
                  "size-1.5 rounded-full",
                  inMainRoom ? "bg-accent" : "bg-idle",
                )}
              />
              <span className="mono-num font-mono text-11 font-bold tracking-chip">
                {inMainRoom ? "ROOM #1111 · MAIN" : `ROOM #${roomCode || "----"}`}
              </span>
            </div>

            {roomCode && (
              <button
                type="button"
                onClick={onCopyCode}
                aria-live="polite"
                title="Copy room code"
                className={cn(
                  "inline-flex h-8 items-center gap-1.5 rounded-full border px-3",
                  "border-border bg-[var(--bg-input)] text-11 font-medium text-text-dim",
                  "transition-colors ease-verio hover:border-border-2 hover:bg-surface-2 hover:text-text press",
                )}
              >
                {copied ? (
                  <IconCheck size={14} className="text-idle" />
                ) : (
                  <IconCopy size={14} />
                )}
                <span>{copied ? "Copied" : "Copy"}</span>
              </button>
            )}

            <span
              aria-label="Call duration"
              className="mono-num hidden h-8 items-center rounded-full border border-border bg-[var(--bg-input)] px-3 font-mono text-11 text-text-dim md:inline-flex"
            >
              {formatTimer(elapsedSeconds)}
            </span>
          </>
        ) : (
          <button
            type="button"
            onClick={onJoinMainRoom}
            disabled={state === "connecting"}
            title="Join permanent Main Squad Room #1111 in one click"
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-full border px-3.5 press",
              "border-[rgba(232,255,58,0.45)] bg-[rgba(232,255,58,0.14)] text-accent",
              "font-mono text-11 font-bold tracking-chip transition-colors ease-verio",
              "hover:bg-accent hover:text-on-accent",
              "disabled:pointer-events-none disabled:opacity-50",
            )}
          >
            <IconZap size={14} />
            <span>MAIN #1111</span>
          </button>
        )}
      </div>

      {/* Right: Update Pill + Connection Status + Single Chat Toggle + Custom Window Chrome */}
      <div className="flex shrink-0 items-center gap-2">
        {availableUpdate && (
          <button
            type="button"
            onClick={onOpenUpdateDialog}
            title={`Update available: v${availableUpdate.version}`}
            className="inline-flex h-7 shrink-0 items-center rounded-full bg-accent px-3 text-11 font-bold text-[#120D1D] transition-colors ease-verio hover:bg-accent-hi press"
          >
            Update v{availableUpdate.version}
          </button>
        )}

        <div
          aria-label={`Connection status: ${connection.label}`}
          className="inline-flex h-8 items-center gap-2 rounded-full border border-border bg-[var(--bg-input)] px-3 mono-num text-11"
        >
          <span aria-hidden="true" className={cn("size-1.5 rounded-full", connection.dot)} />
          <span className={cn("font-medium uppercase tracking-chip", connection.text)}>
            {connection.label}
          </span>
          {isInCall && (
            <>
              <span aria-hidden="true" className="text-text-faint">
                ·
              </span>
              <span className="text-text">
                {rttMs !== null ? `${Math.round(rttMs)} ms` : "-- ms"}
              </span>
            </>
          )}
        </div>

        {isInCall && (
          <button
            type="button"
            onClick={onToggleChat}
            aria-expanded={chatOpen}
            aria-label={chatOpen ? "Hide room chat" : "Show room chat"}
            title={chatOpen ? "Hide room chat" : "Show room chat"}
            className={cn(
              "inline-flex h-8 items-center gap-1.5 rounded-full border px-3",
              "text-11 font-medium transition-colors ease-verio press",
              chatOpen
                ? "border-accent/55 bg-accent/14 text-accent"
                : "border-border bg-[var(--bg-input)] text-text-dim hover:border-border-2 hover:text-text",
            )}
          >
            <IconMessageSquare size={14} />
            <span>Chat</span>
            {!chatOpen && unreadChatCount > 0 && (
              <span className="mono-num rounded-full bg-accent px-1.5 py-0.5 font-mono text-10 font-bold leading-none text-on-accent">
                {unreadChatCount > 9 ? "9+" : unreadChatCount}
              </span>
            )}
          </button>
        )}

        <span aria-hidden="true" className="mx-0.5 h-4 w-px bg-border" />

        {/* Frameless window controls */}
        <div className="flex items-center gap-1">
          <button
            type="button"
            aria-label="Minimize window"
            title="Minimize"
            onClick={() => void invoke("window_minimize").catch(() => {})}
            className="grid size-7 place-items-center rounded-xl text-text-dim transition-colors hover:bg-[rgba(185,167,230,0.14)] hover:text-text"
          >
            <IconMinus size={14} />
          </button>
          <button
            type="button"
            aria-label="Toggle maximize window"
            title="Maximize / Restore"
            onClick={() => void invoke("window_toggle_maximize").catch(() => {})}
            className="grid size-7 place-items-center rounded-xl text-text-dim transition-colors hover:bg-[rgba(185,167,230,0.14)] hover:text-text"
          >
            <IconWindowMaximize size={14} />
          </button>
          <button
            type="button"
            aria-label="Close window"
            title="Close"
            onClick={() => void invoke("window_close").catch(() => {})}
            className="grid size-7 place-items-center rounded-xl text-text-dim transition-colors hover:bg-danger/25 hover:text-[#fca5a5]"
          >
            <IconX size={14} />
          </button>
        </div>
      </div>
    </header>
  );
}
