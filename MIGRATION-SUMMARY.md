# Verio — Frontend Redesign: Complete Change Summary

**Scope:** Phases 0–8 of the Tailwind v4 + shadcn migration, plus the cleanup that followed.
**Sessions:** 17 → 26 (2026-09-20 → 2026-09-23).
**Nature of the work:** frontend redesign and styling-system migration. **Not** a backend rewrite —
the Rust changes below are limited to one separate feature and a set of lint fixes.

---

## 1. Headline numbers

| Metric | Value |
| --- | --- |
| Files changed vs. the initial commit | **54** — 37 added, 14 modified, 3 deleted |
| Line delta vs. the initial commit | **+12,626 / −5,454** |
| `ui/src` source files (`.ts`/`.tsx`/`.css`) | 36 |
| `App.css` | **455 lines, 7 class families** (down from the whole app's stylesheet) |
| App logo | **863,517 → 5,315 bytes** (−99.4%) |
| Unreachable modules | **0** (`dead-modules.mjs`: 33 reachable) |
| Rust tests | **71 passed, 0 failed** |
| Clippy findings cleared | **7** (across 3 crates) |
| Commits | 10 |

---

## 2. What the task was

Verio is a Windows desktop voice-chat app: Tauri v2, React 19, TypeScript, Vite, a Rust backend,
WebRTC over a self-hosted relay, no accounts, and a built-in music player.

The work was a **frontend redesign plus a migration to Tailwind v4 and shadcn** — replacing a
hand-written `App.css` with a token-driven utility system, screen by screen, without changing
what the app can do.

Four constraints governed every step:

1. **The 860×640 window is the target, not desktop.** "860×640 → make excellent → larger windows
   get breathing room." 700–900 is the default band, not the edge case.
2. **One styling authority per element.** Unlayered `App.css` beats Tailwind's `@layer utilities`
   regardless of specificity, so a migrated component had to lose its legacy rules in the *same*
   change.
3. **Never build fake functionality.** No dead buttons, no fake toggles, no invented
   capabilities. The UI exposes only what the app actually knows.
4. **A check that cannot fail is not a check.** Every guard was made to fail on purpose before
   being trusted, and results are reported as measurements, not assertions.

---

## 3. The phases

| Phase | Content | Closed in |
| --- | --- | --- |
| **0** | Analysis: inventory of the existing CSS, screens, states, and hazards | Session 17 |
| **1** | Foundation: Tailwind v4 + `@theme` tokens + base layer + `cn()` + icon module | Session 17 |
| **2** | shadcn primitives (`components/ui/`) | Session 18 |
| **3** | App shell + shared structure (header, dock, layout) | Sessions 18–19 |
| **4** | Music panel — self-contained; proved the meter / well / fader primitives under load | Session 20 |
| **5** | Lobby | Session 21 |
| **6** | Settings | Session 22 |
| **7** | Room — the most important screen; depends on everything above | Session 23 |
| **8** | Cleanup: per-screen CSS deletion, packaging, dead code | Sessions 24–25 |

All four screens are migrated. `App.css` is down to seven class families: `.app`,
`.app-container`, `.main`, `.music`, `.ctx-menu`, `.ctx-item`, `.ctx-sep`.

---

## 4. Defects found — including in my own earlier work

This is the part worth reading. Every one of these was found by a check that was *made* to fail,
not by inspection.

### 4.1 A foundation defect that invalidated every Tailwind spacing utility

Found in Session 18. A root-level rule was overriding the spacing scale, so Tailwind's spacing
utilities silently computed the wrong values app-wide. A class could say `px-5` and compute
`padding: 0`. Caught by measurement, not by reading the CSS.

### 4.2 The `.peer` collision — a name that was two classes at once

`App.css` defined `.peer` (the participant card) while Tailwind defines `peer` (the variant
marker for `peer-checked:*`). `ui/segmented.tsx` and `ui/switch.tsx` carry that marker, so the
legacy card rule was landing on those primitives. Measured before the fix: `radio.peer` computed
`position: relative` (the legacy value) instead of `absolute` (from `sr-only`), and `display:
flex` instead of `block`. `.peer` was **unlayered**, so it outranked `@layer utilities`.

The visible damage was small — the switch's `h-full w-full` compensated — which is why it
survived four phases. Generalisation: **an unlayered legacy rule with a generic name can be
landing on a primitive right now, and no existing check is shaped to find it.**

### 4.3 `dead-modules.mjs` reported 18 unreachable modules; the truth was 1

Three bugs in my own scanner: a lazy `[\s\S]*?` regex that crossed statement boundaries; the `@/`
path alias being ignored; and `/index.ts` entries producing mixed-separator paths that never
string-matched the `join()`-built set, silently dropping every directory-index module. Fixed and
mutation-tested. Final verdict: **33 reachable, 0 unreachable.**

### 4.4 A probe bug reported as a regression

`verify-harness.mjs` read a still-pending status message at 700 ms while the scenario set a
1200 ms latency. The probe's timing was wrong, not the app. Fixed by waiting 2000 ms.

### 4.5 A guard that failed on a *correct* change

`verify-room.mjs` asserted the diagnostics drawer contained a "transport path" row — the row that
had just been deliberately removed. Replaced with a matched pair: the first and last *measured*
rows must be present **and** the unmeasured row must be gone. Mutation-tested both ways.

### 4.6 My own inaccurate claim, retracted

I had recorded that `sounds/*.wav` were "the shipping sound assets in no commit". Measurement
showed **zero code references** — they are orphaned prototypes for a specified but unimplemented
feature. The claim was retracted in `PROGRESS.md`.

*(Per instruction, the `sounds/` item is parked — it is for later work.)*

---

## 5. The four decisions taken

Put to the user at the end of Phase 8, and executed in Session 25:

| Question | Decision | Why |
| --- | --- | --- |
| **Speaking colour** | **Keep `--accent` (amber)** | A genuine contradiction between design sources: `design.md` reserves `--accent` for "primary actions and the speaking indicator"; `REDESIGN-PLAN.md` §4.0 and the deck specify mossy green. `design.md` was made the tie-breaker. Also, the deck renders *almost every* avatar neutral and only the speaker green — so adopting green would require moving the default avatar off `--idle`, which already means the Connected dot, the Paused chip, and the non-PTT badge. |
| **`components/ui/dialog.tsx`** | **Delete** (after committing) | The one proven-unreachable module. No deck contains a modal and Settings is full-screen. |
| **`tauri.conf.json` Q6** | **pnpm only, no `minWidth`** | Q6 proposed ~720 while the verification protocol renders at **700** — enforcing it would make a verified width unreachable. |
| **Diagnostics "Encrypted relay" row** | **Drop the row** | `TransportDiagnostics` exposes no crypto field, so the app cannot know it. Every other row in that drawer is measured. |

---

## 6. Phase 8 cleanup and the packaging pass

- **Logo: 863,517 → 5,315 bytes.** Measured 1254×1254, RGB, no alpha, with a baked gradient
  background. Verified against the master (`design/logo/verio-logo-primary.png`, byte-identical),
  then compared with controls: negative control 4.643 mean / 157 max difference; asset-level
  **0/576 perceptible pixels** at 1×; the real header **0/48,160** at 1× and **12/192,640
  (0.01%)** at 2×. Kept because it is byte-identical to the source the Tauri icons came from.
- **`ui/src/lib/transport.ts` deleted.** Its only consumer was the dropped drawer row; after that
  it had no caller and all four of its exports were internal.
- **Mock harness kept after proving it cannot ship.** 0 occurrences of `mock`,
  `installMockHarness`, or any scenario id in the production bundle — with a **positive control**
  (`participant`, `Verio`, `set_peer_volume`, `Leave` all present) and no source maps.
- **Dead config removed.** `serverReachable` was set in 26 scenarios and read by nothing; the
  `room-connection-lost` scenario's note was false (only the panel button emitted `room_error`).
  Both fixed, and `verify-harness.mjs` now guards the boot path.
- **`tauri.conf.json`** hooks corrected from `npm run …` to `pnpm …` in a pnpm project.
- **`LICENSE`** corrected from `Mahan [Mahan Taqipour]` to `Mahan Taqipour`.

---

## 7. The Rust side

Two items only — this was not a backend rewrite.

### 7.1 A music play queue (a separate feature)

`4175802`: `QueueItem`, `queue_index/len/looping` on `MusicStatus`, and ten commands
(`music_queue_add/remove/move/jump/clear/list/set_loop`, `music_prev`, `music_next`,
`music_pick_files`) plus the `music_queue_changed` event. The frontend's queue UI depends on it.

### 7.2 The gate that had never passed

`verify.bat` is the project's declared gate. It had **never once succeeded**:

- `cargo test --workspace` can never pass here. `build.rs` sets `requireAdministrator` for global
  hotkeys, so the `verio` **bin** test harness dies with `os error 740` and cargo aborts before
  the later crates.
- `cargo clippy --workspace -- -D warnings` was red on a clean checkout.
- It had not been run either: the three artifacts it writes (`clippy7.log`, `test7.log`,
  `verify_done.flag`) do not exist.
- It discarded both exit codes and wrote `ALLDONE` unconditionally — so the flag meant "the
  script reached its last line". A check that cannot fail.

**The clippy errors were a chain, not a set.** Cargo fails fast, so each run revealed exactly one
more problem than the last. It took **seven runs** to reach green:

| Run | Crate | Lint |
| --- | --- | --- |
| 1 | `verio-transport` | `too_many_arguments` (8/7) |
| 2 | `verio-transport` | `op_ref` |
| 3 | `verio-transport` | `op_ref` |
| 4 | `verio-server` | `type_complexity` |
| 5 | `verio` | `manual_is_multiple_of` |
| 6 | `verio-transport` (test) | `empty_line_after_outer_attr` |
| 7 | `verio-transport` (test) | `collapsible_match` |

Generalisation: **"the build is clean" measured by a fail-fast tool means "the first failing crate
is clean."** All seven date from the initial commit — the migration never touched them, and none
affected the shipped binary (`-D warnings` fails clippy, not `cargo build`).

Fixes: mechanical ones where clippy's own replacement was provably equivalent, a pair of type
aliases, and **one documented suppression** — `#[allow(clippy::too_many_arguments)]` on
`spawn_relay_session_with_candidates`, where two of eight parameters are genuinely unused but are
kept for signature interchangeability. Reported as a suppression, not a fix.

`verify.bat` was rewritten: three achievable invocations, failures accumulated across all, and
`exit 1` with `VERIFY FAILED` on any failure. It now uses `--all-targets` — the right flag here
because clippy only *checks* targets and never launches them, so it can lint the `verio` bin
whose test harness `cargo test` cannot run at all.

---

## 8. Verification

### The frontend invariant suite

| Check | Result |
| --- | --- |
| `tsc --noEmit` | clean |
| `vite build` | clean |
| `check-phantom-classes.mjs` | PHANTOM: none |
| `check-collisions.mjs` | LEAK / MIX clean |
| `dead-css.mjs` | DEAD: none · ORPHAN KEYFRAMES: none |
| `dead-modules.mjs` | 33 reachable, **0 unreachable** |
| `verify-music.mjs` | all states render, all controls wired |
| `verify-settings.mjs` | structure, sections, real `(cmd, args)` pairs, inertness, Escape |
| `verify-room.mjs` | 67 assertions |
| `verify-contents-wrapper.mjs` | 40/40 |
| `verify-harness.mjs` | 8/8 |
| `assert-fresh.mjs` | 36/36 |
| `probe-room-contrast.mjs` | WCAG ratio of every text/background pair |

**Every one of these has, at some point, passed while the app was broken.** That is why each
guard is mutation-tested — inject a bug, confirm the specific check fails, revert, confirm
byte-exact via `git hash-object`.

### The Rust gate

```
cargo clippy --workspace --all-targets -- -D warnings   →  exit 0
cargo test --workspace --exclude verio                  →  69 passed, 0 failed
cargo test -p verio --lib                               →   2 passed, 0 failed
```

`VERIFY OK`, `EXITCODE=0`. **Made to fail on purpose:** with one `op_ref` re-injected the script
printed `VERIFY FAILED` and exited 1 **while all 71 tests still passed** — proving a green test
suite cannot mask a red lint step. Reverted byte-exact.

---

## 9. Commits

| Commit | Message |
| --- | --- |
| `98f9143` | `chore: migrate ui to pnpm` |
| `4175802` | `feat(music): add a play queue to the Rust backend` |
| `6f0faae` | `refactor(ui): migrate the frontend to Tailwind v4 + shadcn` (44 files) |
| `54d9ebf` | `docs: add the redesign plan, the design tokens, and the licence holder` |
| `bf35fc5` | `chore(ui): apply the four Phase 8 cleanup decisions` |
| `3eadfe7` | `fix(rust): clear the clippy gate, which had never been green` |
| `ef02c76` | `chore: rewrite verify.bat as a gate that can actually pass` |
| `d6146da` | `docs(progress): record Session 26` |
| `b3810e9` | `fix(rust): extend the clippy gate to --all-targets, and clean its findings` |
| `20a43fe` | `docs(progress): record the --all-targets follow-up` |

The migration sat uncommitted for two days (last commit `98f9143`, 2026-09-20, with 35 untracked
files under `ui/src`). It was committed in Session 25 — **commit before deleting anything
untracked**, because an untracked file has no history.

---

## 10. Still open

- **`identifier: "ir.verio.app"`** ends in `.app`; `tauri build` warns it conflicts with the
  macOS bundle extension. Changing it moves the Windows app-data path, so it needs a deliberate
  decision.
- **`prepare_hotkey_capture`** is a live Rust command with no caller. Its contract is a *pair* —
  wiring it alone would leave a hotkey permanently unregistered. Do not wire it alone.
- **`too_many_arguments` suppression** in `verio-transport/src/lib.rs` — a documented `#[allow]`,
  not a fix. Removing the two unused parameters properly cascades into `verio-app`.
- **`App.css` defines `.ctx-menu` / `.ctx-item` twice, with identical declarations** (lines
  239–269 and 418–426). The second block re-declares `background`, `border`, `box-shadow` and
  `padding` on `.ctx-menu`, and `font-size` on `.ctx-item`, using exactly the values the first
  block already set — so it is **redundant, not a split cascade**; deleting it would be a no-op.
  It survives because the values match and because `dead-css.mjs` cannot see this: that check asks
  "is this class used?", never "is it defined twice?". Harmless, but real duplication — and the
  first block's own comment calls itself a "placeholder — replaced in Pass 4" that was evidently
  never removed.
- **Untracked, deliberately left alone:** `.workbuddy-ai/` (local agent state) and
  `stitch_verio_desktop_interface_suite.zip` (1.5 MB design export). Neither is in `.gitignore`.
- **Parked by instruction:** `sounds/*.wav` — orphaned earcon prototypes for a
  specified-but-unimplemented feature, for later work.

---

## 11. Where the detail lives

| File | Contents |
| --- | --- |
| `PROGRESS.md` | The full session-by-session record, Sessions 17–26. The authoritative log. |
| `REDESIGN-PLAN.md` | The plan: phase order, rationale, and the decisions requested. |
| `design.md` | The design tokens and their rules of use. |
| `~/.workbuddy-ai/skills/verio-frontend-migration/SKILL.md` | The runnable workflow: the invariant suite, its scripts, and the hard-won lessons. |
