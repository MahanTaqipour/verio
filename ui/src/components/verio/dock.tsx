import type { RefObject } from "react";
import { Button, IconButton } from "@/components/ui/button";
import { StateChip } from "@/components/verio/state-chip";
import { KbdChip } from "@/components/verio/kbd-chip";
import { MicLevelMeter, type LevelMeterHandle } from "@/components/verio/level-meter";
import {
  IconCheck,
  IconHeadphoneOff,
  IconHeadphones,
  IconMic,
  IconMicOff,
  IconMusic,
  IconPause,
  IconPencil,
  IconPlay,
  IconSettings,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";

/**
 * Dock — the persistent bottom bar.
 *
 * Direction: the Stitch in-call deck's bottom rail (utility cluster left, call
 * controls right, monospace telemetry chips). Two things from that deck are
 * deliberately NOT reproduced:
 *
 *   1. Its left cluster is `SHARE SM7B (BUS 01)` / `MONITOR OUT 82%` — device
 *      routing controls. Verio has no device-selection or monitor-output
 *      capability, so those would be dead controls describing features that do
 *      not exist. The left cluster here shows what Verio actually knows: who you
 *      are, how you are transmitting, and your live input level.
 *   2. Its button row is a 1080px-wide text-label rail. At Verio's 860px default
 *      the same rail does not fit, so controls are icon+label at 13px.
 *
 * Defects fixed here, all measured before the change:
 *
 *   - **The dock changed height.** `.dock` had `min-height: 72px` with
 *     content-sized children: 72px normally, 76px once the music strip appeared.
 *     Now a fixed `h-18` (72px).
 *   - **The identity block wrapped to two lines** when the music strip squeezed
 *     the left region, pushing the avatar off-centre. `.me-meta` had
 *     `flex-wrap: wrap`. Now a single non-wrapping row whose name truncates.
 *   - **The mic meter shrank from 90px to 78px** when the music strip appeared,
 *     because it had no `flex-shrink: 0`. The meter is a fixed-scale readout, so
 *     its width must not depend on what else is in the bar. Now `shrink-0`, at
 *     40px — see the note on `MicLevelMeter` below for why not 90px.
 *   - **Deafen showed no hotkey while Mic did**, although both hotkeys exist in
 *     settings. Both now show one.
 *   - **The music strip was a clickable `<div>`** — unreachable by keyboard.
 *     Now a real `<button>`.
 *
 * The mic meter remains imperative: `MicLevelMeter` writes `style.width` on a
 * ref and causes zero React renders at ~20 Hz. See level-meter.tsx.
 */

export interface DockMusic {
  /** True once a track is loaded (playing or paused). */
  active: boolean;
  playing: boolean;
  title?: string | null;
  /** File path — used only for the tooltip, never rendered as text. */
  path?: string | null;
}

export interface DockProps {
  nickname: string;
  /** Transmission mode. Both states are real; neither is a placeholder. */
  mode: "open" | "ptt";
  /** The configured push-to-talk hotkey, shown while in PTT mode. */
  pttHotkey: string;

  isEditingNickname: boolean;
  nicknameInput: string;
  onNicknameInputChange: (value: string) => void;
  onStartEditNickname: () => void;
  onSaveNickname: () => void;
  onCancelEditNickname: () => void;

  muted: boolean;
  deafened: boolean;
  onToggleMute: () => void;
  onToggleDeafen: () => void;
  muteHotkey: string;
  deafenHotkey: string;
  onOpenSettings: () => void;

  musicPanelOpen: boolean;
  onToggleMusicPanel: () => void;
  onOpenMusicPanel: () => void;
  /** Null when nothing has been loaded — the strip is then not rendered. */
  music: DockMusic | null;
  /** 0–100 playback position. */
  musicPercent: number;

  meterRef: RefObject<LevelMeterHandle | null>;
}

/**
 * A dock control. `Button` with its height and padding overridden.
 *
 * design.md puts buttons at 13px, which is `Button`'s `md` size — but `md` is
 * 32px tall, and the dock's controls were 38px. `lg` is the right height (40px)
 * but the wrong type size (14px). So the height and padding are overridden on top
 * of `md`, keeping one variant/hover/press/focus implementation rather than a
 * second button written by hand.
 */
const DOCK_BUTTON = "h-10 px-4";

/** Strip the `Ctrl+Shift+` prefix so the chip shows only the distinguishing key. */
function shortKey(hotkey: string): string {
  return hotkey.replace(/^Ctrl\+Shift\+/, "");
}

export function Dock({
  nickname,
  mode,
  pttHotkey,
  isEditingNickname,
  nicknameInput,
  onNicknameInputChange,
  onStartEditNickname,
  onSaveNickname,
  onCancelEditNickname,
  muted,
  deafened,
  onToggleMute,
  onToggleDeafen,
  muteHotkey,
  deafenHotkey,
  onOpenSettings,
  musicPanelOpen,
  onToggleMusicPanel,
  onOpenMusicPanel,
  music,
  musicPercent,
  meterRef,
}: DockProps) {
  const initial = (nickname || "User").slice(0, 1).toUpperCase();
  const showStrip = !!music && (music.active || music.playing);
  const playing = !!music?.playing;

  return (
    /* `@container` makes the dock the reference for its own capacity. The dock
       holds seven things (identity, transmission mode, mic level, music toggle,
       now-playing strip, and two labelled call controls plus settings); measured,
       that needs ~800px. Below that something has to give, and it must not be the
       user's own name or the call controls — so the now-playing strip is dropped
       and the music toggle (which is still there, and still opens the panel that
       shows the same title and a real seek bar) carries that state alone. */
    <footer className="@container flex h-18 shrink-0 items-center gap-5 border-t border-border bg-linear-to-b from-bg to-bg-deep px-5">
      {/* ── Identity + live input level ───────────────────────────────────── */}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <div
          aria-hidden="true"
          className="grid size-9 shrink-0 place-items-center rounded-full bg-accent font-mono text-13 font-semibold text-bg"
        >
          {initial}
        </div>

        {isEditingNickname ? (
          <div className="flex shrink-0 items-center gap-1">
            <input
              type="text"
              value={nicknameInput}
              onChange={(e) => onNicknameInputChange(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") onSaveNickname();
                if (e.key === "Escape") onCancelEditNickname();
              }}
              aria-label="Display name"
              autoFocus
              /* 96px is a component dimension, not a spacing value. design.md
                 defines a spacing scale (4·8·12·16·20·24·32·40·60) and no
                 dimension scale, so there is no on-scale step for an input that
                 must show a name. 96px is the nearest step below the 100px this
                 input used before. */
              className="h-8 w-24 rounded-sm border border-border bg-surface-2 px-2 text-13 text-text outline-none focus-visible:border-border-2"
            />
            <IconButton label="Save display name" onClick={onSaveNickname}>
              <IconCheck size={16} />
            </IconButton>
          </div>
        ) : (
          <div className="flex min-w-0 items-center gap-2">
            {/* The name is the only element allowed to shrink: it is the one
                piece of user-supplied text that can be any length. `min-w-8`
                keeps it from collapsing to nothing — a name that vanishes
                entirely is worse than a truncated one. */}
            <span className="min-w-8 truncate text-14 font-medium text-text">
              {nickname || "User"}
            </span>
            <StateChip tone="neutral">
              {mode === "ptt" ? `Push to talk (${pttHotkey})` : "Open mic"}
            </StateChip>
            <IconButton label="Change your display name" onClick={onStartEditNickname}>
              <IconPencil size={16} />
            </IconButton>
          </div>
        )}

        <MicLevelMeter
          ref={meterRef}
          label="Microphone level"
          /* 40px. The old meter was 90px, which is off design.md's spacing scale
             (4·8·12·16·20·24·32·40·60), so it had to move. 60px was tried first
             and measured: at the 700px floor it clips the display name to its
             32px minimum in every scenario. The name is worth more than 20px of
             meter, so 40px it is — the nearest on-scale step that still reads as
             a level. Do not raise this without re-running dock-metrics.mjs. */
          className="h-1 w-10 shrink-0"
        />
      </div>

      {/* ── Music ─────────────────────────────────────────────────────────── */}
      {/* Below the dock's container breakpoint the now-playing strip is hidden,
          so this button is the only music affordance left. It therefore carries
          the playing state itself: the icon takes the accent while a track is
          playing, and the button takes the pressed background while the panel is
          open. Both are facts the backend reports; neither is decoration.

          The accessible name stays "Toggle music player panel" — an action, as a
          toggle's name should be — and the state goes in `aria-pressed` and the
          title. Folding "(playing)" into the name would conflate state with
          name, and it silently broke the screenshot harness, which drives this
          button by that label. */}
      <IconButton
        label="Toggle music player panel"
        title={playing ? "Music player (playing)" : "Music player"}
        onClick={onToggleMusicPanel}
        aria-pressed={musicPanelOpen}
        variant={musicPanelOpen ? "secondary" : "ghost"}
        size="lg"
        className={cn(
          (musicPanelOpen || playing) && "text-accent",
          musicPanelOpen && "border-accent/30",
        )}
      >
        <IconMusic size={16} />
      </IconButton>

      {showStrip && (
        <button
          type="button"
          onClick={onOpenMusicPanel}
          title={music.title ? `${music.title} — ${music.path ?? ""}` : "Music"}
          className={cn(
            "flex min-w-0 shrink items-center gap-2.5 rounded-sm border border-border bg-surface px-3 py-2",
            "text-left transition-colors ease-verio",
            "hover:border-border-2 hover:bg-surface-2",
            // Container query, not a viewport breakpoint: what matters is how
            // much room the dock has, which is a property of the dock.
            "@max-[800px]:hidden",
          )}
        >
          <span aria-hidden="true" className="grid size-5.5 shrink-0 place-items-center text-accent">
            {music.playing ? <IconPause size={16} filled /> : <IconPlay size={16} filled />}
          </span>
          <span className="flex min-w-0 w-20 flex-col gap-1">
            <span className="truncate text-13 text-text">{music.title || "Music"}</span>
            <span className="h-0.5 overflow-hidden rounded-sm bg-surface-3">
              <span
                className="block h-full bg-accent transition-[width] duration-200 ease-verio"
                style={{ width: `${musicPercent}%` }}
              />
            </span>
          </span>
        </button>
      )}

      {/* ── Call controls ─────────────────────────────────────────────────── */}
      <div className="flex shrink-0 items-center gap-2">
        <Button
          variant="secondary"
          className={cn(DOCK_BUTTON, muted && "border-warn/30 bg-warn/6 text-warn")}
          onClick={onToggleMute}
          aria-pressed={muted}
          title={`Toggle mute (${muteHotkey})`}
          icon={muted ? <IconMicOff size={16} /> : <IconMic size={16} />}
          trailingIcon={<KbdChip decorative>{shortKey(muteHotkey)}</KbdChip>}
        >
          Mic
        </Button>

        <Button
          variant="secondary"
          className={cn(DOCK_BUTTON, deafened && "border-danger/35 bg-danger/8 text-danger")}
          onClick={onToggleDeafen}
          aria-pressed={deafened}
          title={`Toggle deafen (${deafenHotkey})`}
          icon={deafened ? <IconHeadphoneOff size={16} /> : <IconHeadphones size={16} />}
          trailingIcon={<KbdChip decorative>{shortKey(deafenHotkey)}</KbdChip>}
        >
          Deafen
        </Button>

        <IconButton label="Settings" variant="secondary" size="lg" onClick={onOpenSettings}>
          <IconSettings size={16} />
        </IconButton>
      </div>
    </footer>
  );
}
