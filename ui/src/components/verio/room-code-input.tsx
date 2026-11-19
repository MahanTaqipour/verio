/**
 * RoomCodeInput — the 4-digit room code entry.
 *
 * Behaviour carried over from the legacy `.otp-group` markup in App.tsx, and
 * deliberately unchanged because it is what the field already does:
 *
 *   - one digit per box, `inputMode="numeric"` so a touch keyboard offers digits
 *   - non-digits are stripped; pasting or over-typing keeps the LAST character,
 *     so typing over a filled box replaces it rather than appending
 *   - entering a digit advances to the next box; the last box does not wrap
 *   - Backspace on an empty box moves back to the previous one
 *   - each box is labelled "Room code digit N"
 *
 * Two things the rebuild changed on purpose:
 *
 *   1. **Focus moves via refs, not `nextElementSibling`.** The original walked
 *      the DOM, which silently breaks the moment anything is wrapped in another
 *      element. Refs address the inputs directly.
 *   2. **Focus styling comes from the base layer.** The legacy `.otp-box:focus`
 *      swapped the border to --accent and the fill to --surface-2. design.md
 *      specifies a 2px accent outline offset 2px on `:focus-visible`, applied
 *      globally, so the box no longer restyles itself — and the ring now shows
 *      for keyboard users only, not on every mouse click.
 *
 * The value is controlled. The owning screen keeps the string so its lifecycle
 * is unchanged from the original implementation.
 */

import { useRef, type KeyboardEvent } from "react";
import { cn } from "@/lib/utils";

export interface RoomCodeInputProps {
  /** Current code, at most `length` digits. Missing positions read as empty. */
  value: string;
  /** Receives the next code. Never longer than `length`, never non-numeric. */
  onChange: (next: string) => void;
  /** Number of boxes. Verio's room codes are 4 digits. */
  length?: number;
  /** Accessible-name prefix; each box is `${labelPrefix} ${n}`. */
  labelPrefix?: string;
  className?: string;
}

export function RoomCodeInput({
  value,
  onChange,
  length = 4,
  labelPrefix = "Room code digit",
  className,
}: RoomCodeInputProps) {
  const boxes = useRef<Array<HTMLInputElement | null>>([]);

  const focusAt = (index: number) => {
    const clamped = Math.max(0, Math.min(index, length - 1));
    const el = boxes.current[clamped];
    el?.focus();
    el?.select();
  };

  const handleChange = (index: number, raw: string) => {
    const digit = raw.replace(/[^0-9]/g, "").slice(-1);
    const chars = value.split("");
    chars[index] = digit;
    onChange(chars.join("").slice(0, length));
    // Only advance when a digit actually landed, so clearing a box keeps focus.
    if (digit) focusAt(index + 1);
  };

  const handleKeyDown = (index: number, event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Backspace" && !value[index]) focusAt(index - 1);
  };

  return (
    <div className={cn("flex items-center gap-2", className)}>
      {Array.from({ length }, (_, i) => (
        <input
          key={i}
          ref={(el) => {
            boxes.current[i] = el;
          }}
          type="text"
          inputMode="numeric"
          maxLength={1}
          value={value[i] ?? ""}
          aria-label={`${labelPrefix} ${i + 1}`}
          onChange={(e) => handleChange(i, e.target.value)}
          onKeyDown={(e) => handleKeyDown(i, e)}
          className={cn(
            "size-11 shrink-0 rounded-xl border border-[rgba(185,167,230,0.30)] bg-[var(--bg-input)]",
            "mono-num text-center text-18 text-text",
            "transition-colors ease-verio hover:border-border-2 focus-visible:border-accent",
          )}
        />
      ))}
    </div>
  );
}
