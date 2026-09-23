import { cn } from "@/lib/utils";
import { IconCopy } from "@/components/verio/icons";
// 96px derivative of design/logo/verio-logo-primary.png (1254px, 863 kB).
// Rendered at 24 CSS px, so the master was a ~52x downscale decoded on every
// startup and 863 kB of the bundle. The 96px copy is 5,315 bytes (99.38%
// smaller) and measured pixel-identical at 1x (0/576 perceptible pixels) and
// 0.3% at 2x. The master stays in design/logo/ as the source the Tauri icons
// are generated from — do not delete it.
import verioLogo from "@/assets/verio-logo-primary-96.png";

/**
 * AppHeader — the shell's top bar.
 *
 * Direction: the Stitch decks' `win-titlebar` (brand + screen breadcrumb on the
 * left, a live status cluster on the right). The deck also draws minimise /
 * maximise / close buttons; those are deliberately NOT implemented, because
 * Verio uses Tauri's native window decorations — drawing our own would be three
 * controls that look functional and do nothing.
 *
 * Behaviour preserved from the previous header: the brand mark, the ping read
 * out, and the room chip that copies the room code.
 *
 * Behaviour fixed here (see REDESIGN-PLAN.md §6):
 *   - the status dot's `connected` / `connecting` / `idle` classes had no CSS
 *     at all, so the dot never changed colour. It is now driven by `state`.
 *   - the status control was a `<button>` with no `onClick` — it looked
 *     interactive and did nothing. It is now a real button that opens the
 *     diagnostics drawer while in a call, and a plain read-only chip otherwise.
 *     That also makes the diagnostics drawer reachable; it previously had no
 *     caller at all.
 *
 * REMOVED (Session 22): the live transport suffix in the status chip.
 *
 * It read `diagnostics.mode` via `describeLiveTransport` and rendered as
 * `Connected · Relay`. The user's decision was that it should not be shown, and
 * the backend agrees it is not a distinction worth a permanent header slot:
 * `room.rs` sets `effective_mode` to `CloudRelay` unconditionally, and the only
 * remaining input to the label is `Settings.transport_mode`, which the UI has
 * not been able to change since Session 11 removed the transport plumbing.
 * With its `auto` default that resolves to `cloud_relay` — so in the real app
 * the chip could only ever read "Relay". The `direct_p2p` branch is reachable
 * only through `activate_session`, whose callers are the direct-LAN and invite
 * paths that no longer have any UI.
 *
 * A value that cannot vary is not information, so it is gone from the header
 * rather than kept as decoration. It then lived on only in the diagnostics
 * drawer, until Session 24 removed it there too — see the note in `App.tsx`.
 * With that row gone, `lib/transport.ts` had no consumer left and was deleted.
 * The diagnostics drawer now shows only values the app measured.
 */

/** Mirrors the `RoomState` union the app already keeps; structural, so the
 *  caller's own type is assignable without a shared import. */
type RoomState = "idle" | "connecting" | "connected" | "closed";

export interface AppHeaderProps {
  /** Round-trip time in ms, or null while unmeasured. */
  rttMs: number | null;
  /** Connection state reported by the backend. */
  state: RoomState;
  /** Room code, present only once a room exists. */
  roomCode?: string | null;
  /** True once a call exists — gates the room chip and the diagnostics toggle. */
  isInCall: boolean;
  /** Whether the diagnostics drawer is currently open. */
  diagnosticsOpen?: boolean;
  onCopyCode: () => void;
  onToggleDiagnostics: () => void;
}

/**
 * Connection state → label + dot colour.
 *
 * `connected` uses --idle (the teal), not --accent: the accent is reserved for
 * things that want attention, and a healthy call should read as calm.
 */
const STATE_STYLE: Record<RoomState, { label: string; dot: string; text: string }> = {
  idle: { label: "Idle", dot: "bg-text-faint", text: "text-text-dim" },
  connecting: { label: "Connecting", dot: "bg-accent", text: "text-accent" },
  connected: { label: "Connected", dot: "bg-idle", text: "text-idle" },
  closed: { label: "Closed", dot: "bg-warn", text: "text-warn" },
};

export function AppHeader({
  rttMs,
  state,
  roomCode,
  isInCall,
  diagnosticsOpen = false,
  onCopyCode,
  onToggleDiagnostics,
}: AppHeaderProps) {
  const connection = STATE_STYLE[state] ?? STATE_STYLE.idle;

  const statusContent = (
    <>
      <span aria-hidden="true" className={cn("size-1.5 shrink-0 rounded-full", connection.dot)} />
      <span className={connection.text}>{connection.label}</span>
    </>
  );

  const statusClasses =
    "inline-flex items-center gap-2 rounded-sm border border-border bg-surface-2 px-2.5 py-1 " +
    "font-mono text-10 uppercase tracking-[0.08em] transition-colors ease-verio";

  return (
    <header className="flex h-header shrink-0 items-center justify-between gap-4 border-b border-border px-5">
      {/* Brand + screen breadcrumb */}
      <div className="flex min-w-0 items-center gap-3">
        <div className="flex shrink-0 items-center gap-2.5">
          <img src={verioLogo} alt="" width={24} height={24} className="size-6" />
          <span className="font-mono text-11 font-semibold uppercase tracking-[0.22em] text-text">
            Verio
          </span>
          <span aria-hidden="true" className="size-1 rounded-full bg-accent" />
        </div>

        {/* The screen name. Always shown: measured with header-metrics.mjs, the
            widest case (in a call, so the room chip and status chip are both
            present) occupies 498.8px of the 700px floor — 201.2px of slack, on
            one 56px row. No breakpoint is needed. */}
        <span aria-hidden="true" className="text-border-2">
          /
        </span>
        <span className="whitespace-nowrap text-11 text-text-dim">
          {isInCall ? "In call" : "Lobby"}
        </span>
      </div>

      {/* Status cluster */}
      <div className="flex shrink-0 items-center gap-4">
        <div className="flex items-center gap-2 font-mono text-11 text-text-dim">
          <span>Ping</span>
          <b className="font-medium text-text">
            {rttMs !== null ? `${rttMs} ms` : "--"}
          </b>
        </div>

        {isInCall && roomCode && (
          <button
            type="button"
            onClick={onCopyCode}
            title="Copy room code"
            className={cn(
              "inline-flex items-center gap-2.5 rounded-sm border border-border bg-surface px-3 py-1.5",
              "font-mono text-13 tracking-[0.08em] text-text",
              "transition-colors ease-verio",
              "hover:border-border-2 hover:bg-surface-2",
            )}
          >
            <span className="label-caps text-text-dim">Room</span>
            <span>{roomCode}</span>
            <IconCopy
              size={14}
              className="opacity-50 transition-opacity ease-verio"
            />
          </button>
        )}

        {isInCall ? (
          <button
            type="button"
            onClick={onToggleDiagnostics}
            aria-expanded={diagnosticsOpen}
            title={`${connection.label}. Show network diagnostics.`}
            className={cn(
              statusClasses,
              "cursor-pointer hover:border-border-2 hover:bg-surface-3",
            )}
          >
            {statusContent}
          </button>
        ) : (
          <span
            title="Not in a call — no voice path is established."
            className={statusClasses}
          >
            {statusContent}
          </span>
        )}
      </div>
    </header>
  );
}
