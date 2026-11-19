import { Button } from "@/components/ui/button";
import { IconAlert, IconCheck, IconX } from "@/components/verio/icons";
import { cn } from "@/lib/utils";

export interface StatusBannerProps {
  text: string;
  /** Failures get the danger treatment and `role="alert"`. */
  isError?: boolean;
  onDismiss: () => void;
  onPauseAutoDismiss?: () => void;
  onResumeAutoDismiss?: () => void;
  className?: string;
}

/**
 * Compact floating pill toast anchored cleanly under the top bar so status
 * feedback never pushes down page layout or leaves a permanent banner bar.
 */
export function StatusBanner({
  text,
  isError = false,
  onDismiss,
  onPauseAutoDismiss,
  onResumeAutoDismiss,
  className,
}: StatusBannerProps) {
  return (
    <div
      role={isError ? "alert" : "status"}
      aria-live={isError ? "assertive" : "polite"}
      onMouseEnter={onPauseAutoDismiss}
      onMouseLeave={onResumeAutoDismiss}
      onFocus={onPauseAutoDismiss}
      onBlur={onResumeAutoDismiss}
      className={cn(
        "fixed top-16 right-6 z-50 flex max-w-md items-center gap-2.5",
        "rounded-full border py-1.5 pr-2 pl-4 shadow-[var(--shadow-overlay)] backdrop-blur-2xl",
        "animate-rise",
        isError
          ? "border-danger/55 bg-[rgba(69,18,28,0.86)]"
          : "border-border-2 bg-[rgba(28,20,46,0.82)]",
        className,
      )}
    >
      {isError ? (
        <IconAlert size={14} className="shrink-0 text-[#fca5a5]" />
      ) : (
        <IconCheck size={14} className="shrink-0 text-idle" />
      )}
      <span
        className={cn(
          "min-w-0 truncate text-13 font-medium",
          isError ? "text-[#fca5a5]" : "text-text",
        )}
      >
        {text}
      </span>
      <Button
        variant="ghost"
        size="sm"
        onClick={onDismiss}
        aria-label="Dismiss message"
        className="size-6 shrink-0 rounded-full p-0"
      >
        <IconX size={14} />
      </Button>
    </div>
  );
}
