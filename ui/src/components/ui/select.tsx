import { forwardRef, type SelectHTMLAttributes } from "react";
import { cn } from "@/lib/utils";
import { IconChevronDown } from "@/components/verio/icons";

/**
 * Select — design.md §"Controls".
 *
 * A styled *native* `<select>`, not a custom listbox. Deliberate:
 *
 *   - The app runs in WebView2, where the native popup is the only one that
 *     renders above the window correctly at every DPI.
 *   - Keyboard, type-ahead, screen-reader and OS-level accessibility come for
 *     free and cannot be got wrong.
 *   - A custom listbox needs a positioning layer, focus management, roving
 *     tabindex and collision handling — real complexity, and a real place for
 *     the "looks functional but isn't" bug the project forbids.
 *
 * The one thing the native control cannot do is style the popup list, so the
 * closed control is styled and the open list uses the platform default. That is
 * a deliberate trade: correctness over surface polish.
 *
 * `appearance-none` is required or the platform arrow survives alongside the
 * custom chevron.
 */

export interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  /** Rendered at the control's natural width instead of filling its parent. */
  auto?: boolean;
}

export const Select = forwardRef<HTMLSelectElement, SelectProps>(function Select(
  { className, children, auto = false, ...rest },
  ref,
) {
  return (
    <div className={cn("relative", auto ? "inline-flex" : "w-full")}>
      <select
        ref={ref}
        className={cn(
          "h-8 w-full appearance-none rounded-sm border border-border bg-bg",
          "pl-3 pr-8 text-13 text-text",
          "transition-[border-color] ease-verio",
          "hover:border-border-2",
          "disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        {...rest}
      >
        {children}
      </select>
      {/* Decorative: the native control already announces its role and value,
          so the chevron is hidden from assistive tech and does not intercept
          clicks. */}
      <IconChevronDown
        size={14}
        className="pointer-events-none absolute right-2 top-1/2 -translate-y-1/2 text-text-dim"
      />
    </div>
  );
});
