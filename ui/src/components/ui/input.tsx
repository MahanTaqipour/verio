import { forwardRef, type InputHTMLAttributes, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * Input — a recessed well.
 *
 * design.md does not name an input background explicitly. It does say inputs
 * are wells and that `--surface-2`/`--surface-3` are the well tiers, but both
 * are *lighter* than the app background, which would make an input read as
 * raised rather than recessed. The Stitch deck is unambiguous on intent: its
 * recessed wells are darker than its canvas.
 *
 * Resolution: input wells use `--bg`, the darkest token in design.md. That
 * keeps the palette strictly token-driven, gives a genuinely recessed well
 * inside any lighter panel, and matches the deck's intent. Flagged for review
 * in PROGRESS.md as an interpretation, not a transcription.
 *
 * Focus is NOT styled here: design.md defines focus as a 2px accent outline
 * with a 2px offset, applied globally via `:focus-visible` in the base layer.
 * Adding a second focus treatment would double-indicate.
 */

export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  /** Rendered inside the well, before the text. */
  leading?: ReactNode;
  /** Rendered inside the well, pinned right — e.g. a Ctrl+K hint. */
  trailing?: ReactNode;
  /** Applies the error treatment. Pair with an inline message. */
  invalid?: boolean;
  /** Wrapper class; `className` styles the input element itself. */
  wrapperClassName?: string;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { className, wrapperClassName, leading, trailing, invalid = false, ...rest },
  ref,
) {
  return (
    <div
      className={cn(
        "flex h-8 w-full items-center gap-2 rounded-sm border bg-bg px-3",
        "transition-[border-color] ease-verio",
        invalid
          ? "border-danger"
          : "border-border hover:border-border-2 focus-within:border-border-2",
        wrapperClassName,
      )}
    >
      {leading ? (
        <span className="flex shrink-0 items-center text-text-dim">{leading}</span>
      ) : null}
      <input
        ref={ref}
        aria-invalid={invalid || undefined}
        className={cn(
          "h-full min-w-0 flex-1 bg-transparent text-13 text-text",
          "placeholder:text-text-faint",
          "disabled:cursor-not-allowed disabled:text-text-faint disabled:opacity-50",
          className,
        )}
        {...rest}
      />
      {trailing ? (
        <span className="flex shrink-0 items-center text-text-faint">{trailing}</span>
      ) : null}
    </div>
  );
});

/**
 * A monospace input for machine values — room codes, addresses, ports.
 * design.md: every number, code and address is JetBrains Mono.
 */
export const MonoInput = forwardRef<HTMLInputElement, InputProps>(
  function MonoInput({ className, ...rest }, ref) {
    return <Input ref={ref} className={cn("mono-num tracking-chip", className)} {...rest} />;
  },
);
