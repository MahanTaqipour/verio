import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";

const MESSAGES = [
  "INITIALIZING RELAY...",
  "AUTHENTICATING SESSION...",
  "SYNCING ROSTER...",
] as const;

export interface CoreSpinLoaderProps {
  /** Overrides default cycling telemetry phrase when caller wants fixed text. */
  statusText?: string;
  className?: string;
}

/**
 * CoreSpinLoader — counter-rotating multi-ring orbital loader shown while
 * connecting to a room. Uses `#B9A7E6` Lavender, cyan arc, and `#E8FF3A` Acid
 * Lime accents with cycling uppercase telemetry copy.
 */
export function CoreSpinLoader({ statusText, className }: CoreSpinLoaderProps) {
  const [msgIndex, setMsgIndex] = useState(0);

  useEffect(() => {
    if (statusText) return;
    const id = window.setInterval(() => {
      setMsgIndex((prev) => (prev + 1) % MESSAGES.length);
    }, 1100);
    return () => window.clearInterval(id);
  }, [statusText]);

  const text = statusText ?? MESSAGES[msgIndex];

  return (
    <div
      role="status"
      aria-live="polite"
      className={cn(
        "glass-card flex items-center gap-4 rounded-md px-5 py-3.5",
        className,
      )}
    >
      <div aria-hidden="true" className="relative grid size-10 shrink-0 place-items-center">
        {/* Outer dashed Lavender orbital */}
        <span className="animate-orbit-cw absolute inset-0 rounded-full border border-dashed border-[#b9a7e6]/65" />

        {/* Cyan arc ring */}
        <span className="animate-orbit-cw absolute inset-1 rounded-full border-2 border-transparent border-t-[#22d3ee] border-r-[#22d3ee]/40" />

        {/* Reverse Acid Lime orbital arc */}
        <span className="animate-orbit-ccw absolute inset-2 rounded-full border-2 border-transparent border-b-accent border-l-accent/50" />

        {/* Orbiting node */}
        <span className="animate-orbit-cw absolute inset-0">
          <span className="absolute -top-0.5 left-1/2 size-1.5 -translate-x-1/2 rounded-full bg-accent shadow-[0_0_8px_#E8FF3A]" />
        </span>

        {/* Glowing core */}
        <span className="size-2 rounded-full bg-accent shadow-[0_0_12px_rgba(232,255,58,0.8)]" />
      </div>

      <div className="flex min-w-0 flex-col gap-0.5">
        <span className="label-caps text-accent">Connecting Voice Channel</span>
        <span className="mono-num truncate text-13 font-medium tracking-chip text-text">
          {text}
        </span>
      </div>
    </div>
  );
}
