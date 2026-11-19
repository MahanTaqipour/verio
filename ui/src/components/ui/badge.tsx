import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * Badge — the small status chip used for transport mode, connection state,
 * "OPEN MIC", per-peer MUTED/DEAFENED and the music state.
 *
 * design.md rules encoded here:
 *   - uppercase 11px with 0.15em tracking for *labels*; the tracking is the
 *     `label-caps` utility so it matches every other label in the app.
 *   - tone carries meaning and nothing else: `neutral` for information,
 *     `accent` for the active/primary state, `idle` for a connected-but-idle
 *     state, `warn`/`danger` for problems. There is no decorative tone.
 *   - tinted backgrounds are the tone at low alpha over the surface, never a
 *     saturated fill — the only saturated fill in the system is a primary
 *     button.
 *   - no pill radius: design.md uses the 5px --r-sm rectangle.
 */

export type BadgeTone = "neutral" | "accent" | "idle" | "warn" | "danger";

const TONES: Record<BadgeTone, string> = {
  neutral: "bg-surface-3/90 text-text-dim",
  accent: "bg-accent/15 text-accent border-accent/50",
  idle: "bg-idle/15 text-idle border-idle/50",
  warn: "bg-warn/15 text-warn border-warn/50",
  danger: "bg-danger/15 text-[#fca5a5] border-danger/50",
};

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: BadgeTone;
  /** Adds a 1px border in the tone colour. Used when the badge sits on --bg. */
  outlined?: boolean;
  icon?: ReactNode;
  children?: ReactNode;
}

export function Badge({
  tone = "neutral",
  outlined = false,
  icon,
  className,
  children,
  ...rest
}: BadgeProps) {
  return (
    <span
      className={cn(
        "inline-flex select-none items-center gap-1.5 rounded-full px-2.5 py-1",
        "label-caps whitespace-nowrap",
        TONES[tone],
        outlined && "border",
        className,
      )}
      {...rest}
    >
      {icon}
      {children}
    </span>
  );
}
