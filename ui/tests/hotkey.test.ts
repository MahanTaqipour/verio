/**
 * Unit tests for the hotkey binding builder.
 *
 * Run with: `pnpm test` (Node's built-in runner; no test framework is
 * installed in this project). Lives outside `src/` deliberately: it imports
 * `node:test`, and `@types/node` is not a dependency, so keeping it out of the
 * `tsconfig.json` include list keeps `pnpm tsc --noEmit` green.
 *
 * The point of these tests is the *contract*: every binding this module can
 * produce has to be one that `global_hotkey`'s parser accepts. That parser
 * splits the string on `+` and matches each token against a fixed name table,
 * so the two ways to fail are an empty token and an unknown name.
 */

import test from "node:test";
import assert from "node:assert/strict";
import { comboFromEvent, mainKeyName, NUMPAD_CODES } from "../src/lib/hotkey.ts";
import type { KeyEventLike } from "../src/lib/hotkey.ts";

/** Build a keyboard event with only the fields that matter. */
function ev(over: Partial<KeyEventLike>): KeyEventLike {
  return {
    code: "",
    key: "",
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...over,
  };
}

/** The agreed minimum set, spelled as the parser spells it. */
const AGREED = [
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
];

test("every agreed numpad key is captured, under its own name", () => {
  for (const code of AGREED) {
    assert.ok(NUMPAD_CODES.has(code), `${code} missing from NUMPAD_CODES`);
    // `event.key` is deliberately hostile here: for NumpadAdd it is "+", which
    // is the character the parser splits on. The result must ignore it.
    assert.equal(comboFromEvent(ev({ code, key: "+" })), code);
  }
});

test("no numpad binding ever contains an empty token", () => {
  // The structural invariant: "Ctrl++".split("+") === ["Ctrl", "", ""].
  for (const code of NUMPAD_CODES) {
    const combo = comboFromEvent(ev({ code, key: "+", ctrlKey: true, shiftKey: true }));
    assert.ok(combo !== null, `${code} produced no combo`);
    for (const token of combo.split("+")) {
      assert.notEqual(token, "", `${code} produced "${combo}" with an empty token`);
    }
  }
});

test("NumpadAdd with Ctrl does not build Ctrl++", () => {
  // The concrete regression. Before this, `event.key` was used and the result
  // was "Ctrl++", which the parser rejects with EmptyToken.
  const combo = comboFromEvent(ev({ code: "NumpadAdd", key: "+", ctrlKey: true }));
  assert.equal(combo, "Ctrl+NumpadAdd");
  assert.ok(!combo!.includes("++"));
});

test("NumpadAdd without a modifier is still a single valid token", () => {
  assert.equal(comboFromEvent(ev({ code: "NumpadAdd", key: "+" })), "NumpadAdd");
});

test("digits bind as numpad even with NumLock off", () => {
  // With NumLock off, Numpad0 reports key === "Insert" and Numpad1 "End".
  // Using `key` would silently bind the wrong key.
  assert.equal(comboFromEvent(ev({ code: "Numpad0", key: "Insert" })), "Numpad0");
  assert.equal(comboFromEvent(ev({ code: "Numpad1", key: "End" })), "Numpad1");
  assert.equal(comboFromEvent(ev({ code: "Numpad2", key: "ArrowDown" })), "Numpad2");
});

test("NumpadDecimal does not collide with the Delete-to-clear path", () => {
  // Settings treats Delete as "clear this binding". NumpadDecimal reports
  // key === "Delete", so it must be resolved before that check can see it.
  assert.equal(comboFromEvent(ev({ code: "NumpadDecimal", key: "Delete" })), "NumpadDecimal");
  assert.equal(comboFromEvent(ev({ code: "NumpadDecimal", key: "." })), "NumpadDecimal");
});

test("NumpadEnter is distinct from the main Enter", () => {
  assert.equal(comboFromEvent(ev({ code: "NumpadEnter", key: "Enter" })), "NumpadEnter");
  assert.equal(comboFromEvent(ev({ code: "Enter", key: "Enter" })), "Enter");
});

test("NumpadEqual is distinct from the main = key", () => {
  assert.equal(comboFromEvent(ev({ code: "NumpadEqual", key: "=" })), "NumpadEqual");
  assert.equal(comboFromEvent(ev({ code: "Equal", key: "=" })), "=");
});

test("bare modifier presses are ignored", () => {
  for (const key of ["Control", "Shift", "Alt", "Meta"]) {
    assert.equal(comboFromEvent(ev({ code: `Left${key}`, key })), null);
  }
});

test("modifiers are collected in a fixed order", () => {
  assert.equal(
    comboFromEvent(ev({ code: "Numpad5", key: "5", altKey: true, ctrlKey: true, shiftKey: true })),
    "Ctrl+Shift+Alt+Numpad5",
  );
});

test("non-numpad behaviour is unchanged", () => {
  assert.equal(comboFromEvent(ev({ code: "KeyA", key: "a" })), "A");
  assert.equal(comboFromEvent(ev({ code: "KeyA", key: "A", ctrlKey: true })), "Ctrl+A");
  assert.equal(comboFromEvent(ev({ code: "Digit5", key: "5" })), "5");
  assert.equal(comboFromEvent(ev({ code: "Space", key: " " })), "Space");
  assert.equal(comboFromEvent(ev({ code: "F5", key: "F5" })), "F5");
  assert.equal(comboFromEvent(ev({ code: "ArrowUp", key: "ArrowUp" })), "ArrowUp");
  assert.equal(comboFromEvent(ev({ code: "Semicolon", key: ";" })), ";");
});

test("mainKeyName returns null only when there is nothing usable", () => {
  assert.equal(mainKeyName(ev({ code: "Numpad7", key: "Home" })), "Numpad7");
  assert.equal(mainKeyName(ev({ code: "KeyZ", key: "z" })), "Z");
  assert.equal(mainKeyName(ev({ code: "Digit0", key: "0" })), "0");
  assert.equal(mainKeyName(ev({ code: "Space", key: " " })), "Space");
  assert.equal(mainKeyName(ev({ code: "F12", key: "F12" })), "F12");
});

test("punctuation and OEM keys capture correctly", () => {
  assert.equal(comboFromEvent(ev({ code: "Slash", key: "/" })), "/");
  assert.equal(comboFromEvent(ev({ code: "Slash", key: "/", ctrlKey: true })), "Ctrl+/");
  assert.equal(comboFromEvent(ev({ code: "Period", key: "." })), ".");
  assert.equal(comboFromEvent(ev({ code: "Comma", key: "," })), ",");
  assert.equal(comboFromEvent(ev({ code: "Backquote", key: "`" })), "`");
  assert.equal(comboFromEvent(ev({ code: "Minus", key: "-" })), "-");
  assert.equal(comboFromEvent(ev({ code: "Equal", key: "=" })), "=");
  assert.equal(comboFromEvent(ev({ code: "BracketLeft", key: "[" })), "[");
  assert.equal(comboFromEvent(ev({ code: "BracketRight", key: "]" })), "]");
  assert.equal(comboFromEvent(ev({ code: "Backslash", key: "\\" })), "\\");
  assert.equal(comboFromEvent(ev({ code: "Quote", key: "'" })), "'");
});

