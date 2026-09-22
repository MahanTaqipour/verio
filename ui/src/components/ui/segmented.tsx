import { useId, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * SegmentedControl — a small set of mutually exclusive options rendered as one
 * control, e.g. "Voice Activity | Push to Talk" in Settings.
 *
 * Built from real `<input type="radio">` elements inside a `fieldset`, so:
 *   - arrow keys move between options and only one is tabbable (platform
 *     behaviour for a radio group, not something reimplemented here),
 *   - the group is announced with its legend,
 *   - the checked state is real state, not a class that only looks checked.
 *
 * The radios are visually hidden but not `display: none`, so they stay focusable
 * and the global `:focus-visible` outline lands on the visible label via
 * `peer-focus-visible`.
 */

export interface SegmentedOption<T extends string> {
  value: T;
  label: ReactNode;
  icon?: ReactNode;
  disabled?: boolean;
}

export interface SegmentedControlProps<T extends string> {
  /** Group legend. Visually hidden unless `showLegend` is set. */
  legend: string;
  options: ReadonlyArray<SegmentedOption<T>>;
  value: T;
  onChange: (value: T) => void;
  showLegend?: boolean;
  className?: string;
}

export function SegmentedControl<T extends string>({
  legend,
  options,
  value,
  onChange,
  showLegend = false,
  className,
}: SegmentedControlProps<T>) {
  const name = useId();

  return (
    <fieldset className={cn("min-w-0", className)}>
      <legend
        className={cn("label-caps text-text-dim", showLegend ? "mb-2" : "sr-only")}
      >
        {legend}
      </legend>
      <div className="inline-flex rounded-sm border border-border bg-surface-2 p-1">
        {options.map((opt) => {
          const checked = opt.value === value;
          return (
            <label
              key={opt.value}
              className={cn(
                "relative inline-flex select-none items-center gap-2 rounded-sm px-3 py-1.5 text-13",
                "transition-[background-color,color] ease-verio",
                opt.disabled
                  ? "cursor-not-allowed text-text-faint opacity-50"
                  : "cursor-pointer",
                checked
                  ? "bg-surface-3 text-text"
                  : !opt.disabled && "text-text-dim hover:text-text",
              )}
            >
              <input
                type="radio"
                name={name}
                value={opt.value}
                checked={checked}
                disabled={opt.disabled}
                onChange={() => onChange(opt.value)}
                className="peer sr-only"
              />
              {opt.icon}
              {opt.label}
              {/* The focus ring is drawn on the label because the input itself is
                  visually hidden; without this the group would be keyboard
                  operable but not keyboard *visible*. */}
              <span
                aria-hidden="true"
                className={cn(
                  "pointer-events-none absolute inset-0 rounded-sm",
                  "peer-focus-visible:outline peer-focus-visible:outline-2",
                  "peer-focus-visible:outline-accent peer-focus-visible:outline-offset-2",
                )}
              />
            </label>
          );
        })}
      </div>
    </fieldset>
  );
}
