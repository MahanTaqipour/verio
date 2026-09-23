/**
 * Turning a `keydown` into a hotkey binding string.
 *
 * The binding string is the Tauri global-shortcut format (`"Ctrl+Shift+M"`),
 * which `global_hotkey` parses by splitting on `+` and matching each token
 * against a fixed name table. Two consequences drive everything here:
 *
 * 1. **A binding must never contain a `+` in its key name.** `"Ctrl++"` splits
 *    into `["Ctrl", "", ""]` and fails to parse.
 * 2. **The name must be one the parser knows.** Unknown names are rejected.
 *
 * `event.key` cannot satisfy either rule for the numeric keypad, so the numpad
 * is read from `event.code`:
 *
 * - `NumpadAdd` reports `key === "+"`, which would build `"Ctrl++"`.
 * - `NumpadMultiply` / `NumpadDivide` / `NumpadSubtract` report `*`, `/`, `-`.
 * - With NumLock off, `Numpad0`–`Numpad9` report `Insert`, `End`, `ArrowDown`,
 *   and so on — so a binding recorded with `key` would silently capture a
 *   *different* key than the one pressed.
 * - `NumpadDecimal` reports `.` or `Delete`, and `Delete` is already taken:
 *   it is the "clear this binding" key.
 *
 * `event.code` is layout- and NumLock-independent, and every name it produces
 * for the numpad is in the parser's table.
 */

/** The subset of `KeyboardEvent` this needs — so it can be unit-tested. */
export interface KeyEventLike {
  code: string;
  key: string;
  ctrlKey: boolean;
  shiftKey: boolean;
  altKey: boolean;
}

/** Pressing these alone is not a binding; wait for a main key. */
const MODIFIER_KEYS = new Set(["Control", "Shift", "Alt", "Meta"]);

/**
 * Numpad `event.code` values, in the spelling `global_hotkey` accepts.
 *
 * Kept as an explicit list rather than a `startsWith("Numpad")` test so the set
 * is a decision rather than an accident of matching.
 *
 * `NumpadEqual` is included even though it was not in the agreed minimum: it
 * would otherwise fall through to `event.key === "="`, making it
 * indistinguishable from the main keyboard's `=`. Every other numpad key would
 * then be distinct while this one aliased — the inconsistency the rest of this
 * module exists to remove.
 */
export const NUMPAD_CODES = new Set([
  "Numpad0",
  "Numpad1",
  "Numpad2",
  "Numpad3",
  "Numpad4",
  "Numpad5",
  "Numpad6",
  "Numpad7",
  "Numpad8",
  "Numpad9",
  "NumpadAdd",
  "NumpadSubtract",
  "NumpadMultiply",
  "NumpadDivide",
  "NumpadDecimal",
  "NumpadEnter",
  "NumpadEqual",
]);

/**
 * The main key's name, or `null` when the event carries no usable main key.
 *
 * Numpad is checked first: for those codes `event.key` is either a symbol that
 * would corrupt the binding or, under NumLock-off, the wrong key entirely.
 */
export function mainKeyName(e: KeyEventLike): string | null {
  if (NUMPAD_CODES.has(e.code)) {
    return e.code;
  }
  if (e.code.startsWith("Key")) {
    return e.code.slice(3).toUpperCase();
  }
  if (e.code.startsWith("Digit")) {
    return e.code.slice(5);
  }
  if (e.code === "Space") {
    return "Space";
  }
  if (e.key.length === 1) {
    return e.key.toUpperCase();
  }
  return e.key;
}

/**
 * Build the binding string for a keydown, or `null` if it should be ignored
 * (a bare modifier press, or nothing usable).
 *
 * `Meta` is deliberately not emitted as a modifier — unchanged from the
 * behaviour this replaced.
 */
export function comboFromEvent(e: KeyEventLike): string | null {
  if (MODIFIER_KEYS.has(e.key)) {
    return null;
  }

  const main = mainKeyName(e);
  if (main === null || main === "") {
    return null;
  }

  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.shiftKey) parts.push("Shift");
  if (e.altKey) parts.push("Alt");
  parts.push(main);

  return parts.join("+");
}
