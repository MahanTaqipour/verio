/**
 * StatusBanner — the app's only transient message surface.
 *
 * It replaced the legacy `.toast-banner` / `.toast-close` pair, which were the
 * last unlayered App.css rules reachable from the shell. Presentation only: the
 * message and its tone arrive as props, and the caller owns dismissal.
 *
 * `role="status"` with `aria-live="polite"` for ordinary messages and
 * `role="alert"` for failures. That split is deliberate rather than decorative:
 * a failure interrupts, an acknowledgement does not. The legacy markup was a
 * bare `<div>`, so a screen reader announced neither.
 *
 * The close control is a real `<button>` with an accessible name. The legacy
 * one was a `<button>` containing an inline `<svg>` and no text — which is fine
 * — but it had no `aria-label` either, so it announced as "button".
 */

import { Button } from "@/components/ui/button";
import { IconAlert, IconCheck, IconX } from "@/components/verio/icons";
import { cn } from "@/lib/utils";

export interface StatusBannerProps {
  text: string;
  /** Failures get the danger treatment and `role="alert"`. */
  isError?: boolean;
  onDismiss: () => void;
  className?: string;
}

export function StatusBanner({ text, isError = false, onDismiss, className }: StatusBannerProps) {
  return (
    <div
      role={isError ? "alert" : "status"}
      aria-live={isError ? "assertive" : "polite"}
      className={cn(
        "flex items-center gap-3 rounded-sm border bg-surface-2 py-2.5 pr-2 pl-3.5",
        // Session 29d: the banner drops in rather than appearing at full height.
        // `animate-rise` is opacity + a 4px settle, so it never changes the layout
        // it sits in — the room below is measured by the probes and must not move.
        "animate-rise",
        isError ? "border-danger/35" : "border-border-2",
        className,
      )}
    >
      {/* The icon carries the tone for sighted users; the role carries it for
          assistive tech. Both, because neither alone is enough. */}
      {isError ? (
        <IconAlert size={14} className="shrink-0 text-danger" />
      ) : (
        <IconCheck size={14} className="shrink-0 text-idle" />
      )}
      <span className={cn("min-w-0 flex-1 text-13", isError ? "text-danger" : "text-text")}>
        {text}
      </span>
      <Button
        variant="ghost"
        size="sm"
        onClick={onDismiss}
        aria-label="Dismiss message"
        className="shrink-0"
      >
        <IconX size={14} />
      </Button>
    </div>
  );
}
