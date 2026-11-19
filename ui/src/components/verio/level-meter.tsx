/**
 * Live microphone level meter — the imperative one.
 *
 * `input_level` arrives from Rust at roughly 20 Hz. Routing that through React
 * state would re-render the whole tree 20 times a second for a single bar
 * width, which is exactly the cost the existing implementation avoids by
 * writing `style.width` on a ref.
 *
 * That behaviour is preserved deliberately. The contract is:
 *
 *     input_level (dB, ~20 Hz)
 *          ↓
 *     ref.current.style.width = `${pct}%`
 *          ↓
 *     zero React renders
 *
 * Do not "clean this up" into `useState`. There is no correctness problem to
 * solve here, and the re-render cost is real.
 *
 * The dB→percent mapping below is carried over unchanged from the original
 * App.tsx so the meter reads identically.
 */

import {
  forwardRef,
  useEffect,
  useImperativeHandle,
  useRef,
  type CSSProperties,
} from "react";
import { listen } from "@tauri-apps/api/event";
import { cn } from "@/lib/utils";

/** -60 dB → 0%, 0 dB → 100%. Unchanged from the pre-redesign implementation. */
export function dbToPercent(db: number): number {
  return Math.min(100, Math.max(0, (db + 60) * 1.67));
}

export interface LevelMeterHandle {
  /** Set the fill directly. Safe to call at any rate; causes no re-render. */
  setLevel(percent: number): void;
  /** Convenience wrapper that applies the dB mapping. */
  setDb(db: number): void;
}

export interface LevelMeterProps {
  className?: string;
  /** Fill colour class. Defaults to the accent token. */
  fillClassName?: string;
  /** Accessible name for the meter. */
  label?: string;
}

/**
 * A meter bed whose fill is driven imperatively.
 *
 * Usage:
 *   const meter = useRef<LevelMeterHandle>(null);
 *   useMicLevelStream(meter);
 *   <MicLevelMeter ref={meter} />
 */
export const MicLevelMeter = forwardRef<LevelMeterHandle, LevelMeterProps>(
  function MicLevelMeter(
    { className, fillClassName = "bg-accent", label = "Microphone level" },
    ref,
  ) {
    const fillRef = useRef<HTMLDivElement>(null);

    useImperativeHandle(
      ref,
      () => ({
        setLevel(percent: number) {
          const fill = fillRef.current;
          if (!fill) return;
          const clamped = Math.min(100, Math.max(0, percent));
          fill.style.width = `${clamped}%`;
        },
        setDb(db: number) {
          this.setLevel(dbToPercent(db));
        },
      }),
      [],
    );

    return (
      <div
        className={cn(
          "h-1.5 w-full overflow-hidden rounded-full border border-border/60 bg-[var(--bg-input)]",
          className,
        )}
        role="meter"
        aria-label={label}
        aria-valuemin={0}
        aria-valuemax={100}
      >
        <div
          ref={fillRef}
          className={cn("h-full rounded-full transition-none", fillClassName)}
          style={{ width: "0%" } satisfies CSSProperties}
        />
      </div>
    );
  },
);

/**
 * Subscribe to the `input_level` event and push values into a meter handle.
 *
 * Deliberately does not touch React state. The listener is removed on unmount.
 */
export function useMicLevelStream(
  meter: React.RefObject<LevelMeterHandle | null>,
  enabled = true,
): void {
  useEffect(() => {
    if (!enabled) {
      meter.current?.setLevel(0);
      return;
    }
    let disposed = false;
    let unlisten: (() => void) | undefined;

    void listen<number>("input_level", (ev) => {
      meter.current?.setDb(ev.payload);
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [meter, enabled]);
}
