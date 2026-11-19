import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * StateChip — the one way Verio renders a small uppercase status chip.
 *
 * It exists because three places needed the same object: the header's connection
 * status, the dock's transmission-mode badge, and the music panel's playback
 * state. Written three times they drift; written once, "what does amber mean
 * here" has a single answer.
 *
 * Tone vocabulary is derived from design.md's semantic colours, and each tone is
 * used for exactly what design.md says it is for:
 *
 *   neutral  — no state to signal. Also the default when a caller has nothing
 *              meaningful to say, so an empty state never borrows a colour it
 *              has not earned.
 *   accent   — `--accent`: reserved for primary actions and the speaking
 *              indicator. A chip using it must be reporting one of those.
 *   idle     — `--idle`: quiet, connected, at rest.
 *   warn     — `--warn`: muted / deafened. Intentional states that should still
 *              be visible.
 *   danger   — `--danger`: genuinely destructive or failed states only.
 *
 * A chip is inert unless `onClick` is supplied. That is deliberate: the previous
 * header rendered a `<button>` with no handler, which looked interactive and did
 * nothing. Here, "looks clickable" and "is clickable" cannot come apart.
 */

export type ChipTone = "neutral" | "accent" | "idle" | "warn" | "danger";

const TONES: Record<ChipTone, string> = {
  neutral: "border-border bg-surface-2/85 text-text-dim",
  accent: "border-accent/40 bg-accent/12 text-accent",
  idle: "border-idle/35 bg-idle/12 text-idle",
  warn: "border-warn/35 bg-warn/12 text-warn",
  danger: "border-danger/45 bg-danger/15 text-[#fca5a5]",
};

/** The dot colour that matches each tone, for callers that show a leading dot. */
export const TONE_DOT: Record<ChipTone, string> = {
  neutral: "bg-text-faint",
  accent: "bg-accent",
  idle: "bg-idle",
  warn: "bg-warn",
  danger: "bg-danger",
};

export interface StateChipProps {
  tone?: ChipTone;
  /** Rendered before the label — normally a `size-1.5 rounded-full` dot. */
  leading?: ReactNode;
  /** Rendered after the label, muted. Used for the transport segment. */
  trailing?: ReactNode;
  children: ReactNode;
  className?: string;
  /**
   * Supply this to make the chip a real button. Omit it and the chip is an inert
   * span that cannot be focused or clicked.
   */
  onClick?: () => void;
  /** Tooltip / accessible description. */
  title?: string;
  /**
   * The chip's accessible name. Required when the chip contains only an icon:
   * the chip renders as a plain `<span>` (or `<button>`), and every icon in the
   * module is `aria-hidden`, so without this an icon-only chip would be silent
   * to a screen reader. Ignored when the chip already has text children.
   */
  ariaLabel?: string;
  /** Only meaningful when `onClick` is set. */
  expanded?: boolean;
}

export function StateChip({
  tone = "neutral",
  leading,
  trailing,
  children,
  className,
  onClick,
  title,
  ariaLabel,
  expanded,
}: StateChipProps) {
  const classes = cn(
    "inline-flex select-none items-center gap-2 whitespace-nowrap rounded-full border px-2.5 py-1",
    "font-mono text-10 uppercase tracking-chip",
    "transition-colors ease-verio",
    TONES[tone],
    onClick && "cursor-pointer hover:border-border-2 hover:bg-surface-3",
    className,
  );

  // `min-w-0` on the label so a caller can opt into truncation — `max-w-full` on
  // the chip plus `truncate` on the text. Without it the label's automatic minimum
  // size is its whole content width, and since a chip never wraps
  // (`whitespace-nowrap` is its own rule) a long value runs straight out of its
  // card. That is exactly what a long music filename did.
  const body = (
    <>
      {leading}
      <span className="min-w-0">{children}</span>
      {trailing}
    </>
  );

  if (!onClick) {
    return (
      <span title={title} aria-label={ariaLabel} className={classes}>
        {body}
      </span>
    );
  }

  return (
    <button
      type="button"
      onClick={onClick}
      title={title}
      aria-label={ariaLabel}
      aria-expanded={expanded}
      className={classes}
    >
      {body}
    </button>
  );
}
