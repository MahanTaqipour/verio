import { forwardRef, type InputHTMLAttributes } from "react";
import { cn } from "@/lib/utils";

/**
 * Switch — a tactile hardware toggle.
 *
 * design.md forbids fully-rounded shapes outside avatars and status dots, so
 * this is a small rounded rectangle with a sliding block, not the usual pill.
 * That also matches the Stitch deck's "hardware push button" direction.
 *
 * Accessibility and focus: a real <input type="checkbox"> is used, stretched
 * over the track with `opacity-0`. Because the input's box is exactly the
 * track's box, the base layer's global `:focus-visible` outline lands around
 * the visible control — no mirrored focus styling, no `sr-only` clipping.
 */

export interface SwitchProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "type"> {
  /** Accessible name when the switch is not inside a <label>. */
  label?: string;
}

export const Switch = forwardRef<HTMLInputElement, SwitchProps>(function Switch(
  { className, label, checked, ...rest },
  ref,
) {
  return (
    <span
      className={cn("relative inline-flex h-[18px] w-[34px] shrink-0", className)}
    >
      <input
        ref={ref}
        type="checkbox"
        role="switch"
        checked={checked}
        aria-label={label}
        className="peer absolute inset-0 z-10 m-0 h-full w-full cursor-pointer appearance-none rounded-full opacity-0 disabled:cursor-not-allowed"
        {...rest}
      />
      {/* Track */}
      <span
        aria-hidden="true"
        className={cn(
          "absolute inset-0 rounded-full border transition-colors ease-verio",
          "border-border bg-[var(--bg-input)]",
          "peer-hover:border-border-2",
          "peer-checked:border-accent/55 peer-checked:bg-accent/15",
          "peer-disabled:opacity-50",
        )}
      />
      {/* Knob */}
      <span
        aria-hidden="true"
        className={cn(
          "pointer-events-none absolute top-[2px] left-[2px] h-[12px] w-[12px] rounded-full",
          "border border-border-2 bg-text-dim",
          "transition-[transform,background-color,border-color] ease-verio",
          "peer-checked:translate-x-[16px] peer-checked:border-accent peer-checked:bg-accent",
        )}
      />
    </span>
  );
});
