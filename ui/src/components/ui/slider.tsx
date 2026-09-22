import { forwardRef, type InputHTMLAttributes } from "react";
import { cn } from "@/lib/utils";

/**
 * Slider — wraps the one piece of custom CSS in the system (`styles/range.css`).
 *
 * Why custom CSS rather than utilities: the track is 4px with a 2px radius and
 * the thumb is a 12x16px knurled grip drawn with a repeating gradient. Chromium
 * exposes those as `::-webkit-slider-runnable-track` / `::-webkit-slider-thumb`,
 * which are shadow-DOM pseudo-elements — Tailwind utilities cannot reach them,
 * and `accent-color` cannot express a knurled grip. design.md calls this out as
 * the sanctioned exception.
 *
 * The fill is driven by `--range-fill` (a percentage) rather than a background
 * gradient, so a caller can update it from an imperative hot path without
 * re-rendering React — the same reason the mic meter is imperative.
 *
 * The element is a real `<input type="range">`: keyboard support, ARIA
 * `role="slider"` semantics, and `aria-valuetext` all come from the platform.
 */

export interface SliderProps extends InputHTMLAttributes<HTMLInputElement> {
  /** 0–100. Drives the filled portion of the track. */
  fillPercent?: number;
  /** Accessible description of the current value, when the number alone is unclear. */
  valueText?: string;
}

export const Slider = forwardRef<HTMLInputElement, SliderProps>(function Slider(
  { className, fillPercent, valueText, style, ...rest },
  ref,
) {
  return (
    <input
      ref={ref}
      type="range"
      aria-valuetext={valueText}
      className={cn("verio-range", className)}
      style={
        fillPercent === undefined
          ? style
          : { ...style, ["--range-fill" as string]: `${Math.min(100, Math.max(0, fillPercent))}%` }
      }
      {...rest}
    />
  );
});
