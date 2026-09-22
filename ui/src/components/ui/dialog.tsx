import { useCallback, useEffect, useRef, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { IconButton } from "@/components/ui/button";
import { IconX } from "@/components/verio/icons";

/**
 * Dialog — built on the native `<dialog>` element via `showModal()`.
 *
 * Chosen over a Radix-style portal because the platform already implements
 * everything a modal needs, correctly and for free:
 *
 *   - focus is moved into the dialog and trapped there,
 *   - Escape closes it (the `cancel` event),
 *   - the rest of the page is inert to pointer and assistive tech,
 *   - `::backdrop` is a real backdrop pseudo-element,
 *   - the top layer means no z-index arms race.
 *
 * Reimplementing that with a portal and manual listeners is a well-known source
 * of focus bugs, and it would be a new runtime dependency for no gain.
 *
 * `onClose` is called for every dismissal path (Escape, backdrop, the close
 * button), so callers keep one piece of state rather than three.
 */

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  /** Accessible name. Rendered as the visible title unless `hideTitle`. */
  title: string;
  /** Set when the caller renders its own heading and only needs the label. */
  hideTitle?: boolean;
  /** Hides the close button for dialogs that require an explicit choice. */
  hideClose?: boolean;
  children: ReactNode;
  /** Width cap. design.md: overlays may be wide, but must stay on the grid. */
  size?: "sm" | "md" | "lg";
  className?: string;
}

const SIZES = {
  sm: "max-w-[380px]",
  md: "max-w-[520px]",
  lg: "max-w-[720px]",
} as const;

export function Dialog({
  open,
  onClose,
  title,
  hideTitle = false,
  hideClose = false,
  children,
  size = "md",
  className,
}: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useRef(`dialog-title-${Math.random().toString(36).slice(2)}`).current;

  // Drive the native element from React state rather than the reverse: `open`
  // is the single source of truth, and `showModal()` is called as a side effect
  // of it. Without this the element can drift out of sync with React.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    if (open && !el.open) el.showModal();
    else if (!open && el.open) el.close();
  }, [open]);

  // `cancel` fires on Escape; `close` fires for every dismissal. Both route
  // back to the same callback so the caller never sees a stuck dialog.
  const handleClose = useCallback(() => onClose(), [onClose]);

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onClose={handleClose}
      // Clicking the backdrop targets the dialog element itself, because the
      // dialog's own box is only as large as its content.
      onClick={(e) => {
        if (e.target === ref.current) onClose();
      }}
      className={cn(
        "m-auto w-full rounded-lg border border-border-2 bg-surface p-0 text-text",
        "shadow-overlay backdrop:bg-bg/80",
        SIZES[size],
        className,
      )}
    >
      <div className="flex items-center justify-between gap-3 border-b border-border px-5 py-4">
        <h2 id={titleId} className={cn("text-15 font-medium text-text", hideTitle && "sr-only")}>
          {title}
        </h2>
        {!hideClose && (
          <IconButton label="Close" size="sm" onClick={onClose}>
            <IconX size={14} />
          </IconButton>
        )}
      </div>
      <div className="px-5 py-4">{children}</div>
    </dialog>
  );
}
