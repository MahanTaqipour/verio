import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * KbdChip — a hotkey or key hint.
 *
 * design.md, twice over:
 *   - "every number, code, and address" uses JetBrains Mono, and hotkey chips
 *     are named explicitly in that list
 *   - "4px — internal padding on tiny elements (hotkey chips, badges)"
 *   - `--r-sm` (5px) is the radius for "buttons, inputs, chips, small cards"
 *
 * Rendered as a real `<kbd>` element, which is what the element is for. Note the
 * chip is not the accessible name of its control: a button reading "Mic M" to a
 * screen reader is worse than one reading "Mic", so callers keep the key hint
 * decorative (`aria-hidden`) where the surrounding control already has a name,
 * and the chip's text stays available to sighted users.
 */
export interface KbdChipProps {
  /** The key or chord, e.g. "M" or "Ctrl+Shift+M". */
  children: ReactNode;
  className?: string;
  /**
   * Hide from assistive tech. Set this when the chip is a hint inside a control
   * whose accessible name is already complete.
   */
  decorative?: boolean;
}

export function KbdChip({ children, className, decorative = false }: KbdChipProps) {
  return (
    <kbd
      aria-hidden={decorative || undefined}
      className={cn(
        // leading-none keeps the chip from inheriting a tall line box, which is
        // what made the old .kbd chip 17px tall for 10px text.
        "inline-flex shrink-0 items-center rounded-sm bg-surface-3 px-1 py-1",
        "font-mono text-10 leading-none tracking-normal text-text-dim",
        className,
      )}
    >
      {children}
    </kbd>
  );
}
