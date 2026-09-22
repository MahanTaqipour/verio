import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * TelemetryRow — one label/value reading, as used by the diagnostics drawer.
 *
 * Layout is two lines: the label and its value share the first line (label left,
 * value right), and the optional hint spans the second. That shape is not
 * cosmetic — the drawer has 280px of content width, and the previous markup put
 * label, value and sub-text on ONE row as three flex children of a
 * `justify-content: space-between` container. Each of the three then wrapped into
 * its own narrow column: measured at 860px, "Transport Path", "Encrypted relay"
 * and "Every call runs through the Verio relay server." rendered as three
 * interleaved multi-line stacks that overlapped the row below. Two lines is the
 * widest shape that fits, so it is the shape.
 *
 * `value` is monospace with tabular figures (`mono-num`) because it changes
 * while you look at it — a proportional face makes the digits jitter, and in a
 * column of readings that reads as instability that isn't there.
 *
 * The label is `telemetry-label` (Inter, 10px, 0.08em) rather than `label-caps`
 * (Inter, 11px, 0.15em). design.md reserves 0.15em for section labels; at 10px it
 * pushes a two-word label past the value it shares a line with.
 */

export type TelemetryTone = "default" | "accent" | "warn";

/**
 * Tone is about the reading, not decoration:
 *   default — a normal reading.
 *   accent  — the row that answers "what is carrying my voice".
 *   warn    — a reading that is currently degraded. Never used for "unknown";
 *             unknown is stated in words, not coloured.
 */
const TONE: Record<TelemetryTone, string> = {
  default: "text-text",
  accent: "text-accent",
  warn: "text-warn",
};

export interface TelemetryRowProps {
  label: ReactNode;
  value: ReactNode;
  /** Supporting text on its own line. Omit when it would restate the label. */
  hint?: ReactNode;
  tone?: TelemetryTone;
  className?: string;
}

/**
 * The container for a group of rows. Separate from `TelemetryRow` because the
 * divider belongs to the group, not the row: `divide-y` on the parent draws
 * rules *between* rows and therefore never leaves a trailing rule under the last
 * one, which `last:border-b-0` on each row only achieves by exception.
 */
export function TelemetryList({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <dl className={cn("flex flex-col divide-y divide-border", className)}>{children}</dl>
  );
}

export function TelemetryRow({
  label,
  value,
  hint,
  tone = "default",
  className,
}: TelemetryRowProps) {
  return (
    /* A description list, not a stack of divs: these are label/value pairs, which
       is exactly what <dl>/<dt>/<dd> means. It also gives the verification probe
       a stable structural hook, so a probe can find rows without keying on a
       styling class — keying on `.diag-row` is what made the last probe report
       "drawer not found" the moment the drawer was rebuilt. */
    <div className={cn("flex flex-col gap-1 py-2.5", className)}>
      <div className="flex items-baseline justify-between gap-3">
        <dt className="telemetry-label shrink-0 text-text-dim">{label}</dt>
        <dd className={cn("mono-num min-w-0 text-right text-13", TONE[tone])}>
          {value}
        </dd>
      </div>
      {hint && <dd className="text-11 text-text-faint">{hint}</dd>}
    </div>
  );
}
