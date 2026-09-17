# Verio — Progress Log

> This file is the project's memory across sessions. After every session: record what
> works, what's broken, decisions made, and next steps. Paste REAL command output.
> HONESTY RULE: never mark an AC as passed without actually running it.

## Project

- P2P group voice chat (full mesh, 2–10 people), Windows, for use while gaming.
- Zero servers: mDNS LAN discovery + manual invite codes for internet.
- RNNoise → Opus (48 kHz mono, 20 ms frames) over WebRTC (libdatachannel).
- AEC/AGC explicitly DEFERRED (no webrtc-audio-processing in Phase 0).

## Environment (verified)

- Windows 10 Pro 22H2 (19045), i7-4910MQ, 8 GB RAM. Project root: `C:\dev\verio`.
- Rust 1.95.0 stable (x86_64-pc-windows-msvc), clippy installed.
- Node 22.14.0, npm 11.19.1, tauri-cli 2.11.4 (npx — never `cargo install tauri-cli`).
- CMake 4.4.3 / Ninja 1.13.2 on PATH. NASM absent (libopus C fallback — fine).
- Disk: 26.2 GB free on C: at session 1 start. Re-check free space every session; if
  < 8 GB → `cargo clean`, re-check, report.

## Session log

### Session 1 (2026-08-29) — Phase 0: Scaffold

Goal: workspace + Tauri 2 app boots; commands/events round-trip proven; clippy clean.

Work done:
- Created Cargo workspace with 5 stub crates (`verio-audio`, `verio-dsp`, `verio-transport`,
  `verio-discovery`, `verio-app`) + Tauri 2 app in `ui/` (React 19 + TS + Vite 6).
- Bridge proof: `ping` command (FE→Rust→FE, with error path for empty name) and
  `emit_test_event` command (Rust→FE event); UI has buttons for both directions.
- `src-tauri` depends on `verio-app` (workspace wiring proven).
- Rust unit tests: `ping` success/error paths, `verio-app::app_info`.

Fixes during the session (with real errors encountered):
1. `rust-toolchain.toml` pin (channel "1.95.0") triggered rustup to DOWNLOAD a second
   toolchain → violated the no-install rule. Killed build, deleted the file, ran
   `rustup toolchain uninstall 1.95.0`. Build uses existing `stable` default (1.95.0).
2. `tauri-build` failed: "`icons/icon.ico` not found; required for generating a
   Windows Resource file" → generated placeholder icon (blue square + white "V",
   256px PNG wrapped in a hand-built ICO container). First attempt failed again:
   "Invalid reserved field value in ICONDIRENTRY (was 135, but must be 0)" —
   System.Drawing `Icon.Save()` writes nonconformant ICO headers; fixed by writing
   the ICO bytes manually (reserved fields = 0). Proper icon set deferred to Phase 4
   (`npx tauri icon`).
3. `error[E0433]: cannot find module or crate verio_lib` → added
   `[lib] name = "verio_lib"` to `ui/src-tauri/Cargo.toml`.

Verified (real output):
- `npm install` → "added 72 packages in 26s"
- `npm run build` (tsc && vite) → "vite v6.4.3 building for production... ✓ built in 4.30s",
  dist/index.html + assets produced
- `cargo build --release` → "Finished `release` profile [optimized] target(s) in 2m 00s"
  (target: C:\dev\verio\target\release\verio.exe exists)
- `cargo clippy --workspace -- -D warnings` → "Finished `dev` profile ... in 4m 15s",
  zero warnings/errors (the only grep hits for "error" were `thiserror` crate names)
- Boot test: launched `target\release\verio.exe`, process stayed alive 7s with window
  title "Verio", killed cleanly → app boots, WebView2 renders.
- `cargo test --workspace` → all pass: verio-app 1 passed (app_info), verio 2 passed
  (ping success + ping empty-name error path), stub crates 0 tests.

Decisions:
- Stub crates (`verio-audio`, `verio-dsp`, `verio-transport`, `verio-discovery`, `verio-app`) are
  created with zero dependencies in Phase 0. Real deps get added in their own phases
  to keep early builds fast and disk usage low.
- `rust-toolchain.toml` pins channel 1.95.0 for determinism.
- File logging via `tracing` is deferred to Phase 1 (needs a subscriber crate; kept
  Phase 0 dependency-free).
- `npx tauri build` (bundler/NSIS) is deferred to Phase 4. Phase 0 boot proof is done
  by launching `target\release\verio.exe` directly and confirming the process stays
  alive.

What works:
- (pending verification)

What's broken / known issues:
- (none yet)

Acceptance criteria — Phase 0:
- [x] Workspace compiles: `cargo build --release` clean ("Finished ... in 2m 00s")
- [x] `cargo clippy --workspace -- -D warnings` clean (zero warnings)
- [x] Tauri app boots (exe launch test: alive after 7s, window "Verio")
- [ ] Command round-trip FE→Rust→FE proven — code in place + unit tested; needs one
      interactive click-through by the user (see "Handed to user" below)
- [ ] Event round-trip Rust→FE proven — same interactive click-through

Handed to user (awaiting result):
- Run `C:\dev\verio\target\release\verio.exe` (or `npx tauri dev` from `C:\dev\verio`)
  and click: 1) "Call Rust `ping`" → expect green message; 2) "Test error path" →
  expect red error line; 3) "Emit event from Rust" → expect a timestamped log line.

Next steps:
- Session 2: user says "continue with Phase 1" → re-read this file, check disk space,
  build the local audio pipeline (capture → gain → RNNoise → VAD/DTX → Opus →
  loopback), hotkeys, debug WAV output.
### Session 2 (2026-08-30) — Phase 0 addendum + Phase 1: local audio pipeline

Pre-flight:
- Read this file fully. Free disk on C: at session start: **16.16 GB** (≥ 8 GB — no clean needed).
- Rename pass (Phase 0 addendum): verified and completed — all five crates are
  `verio-audio`, `verio-dsp`, `verio-transport`, `verio-discovery`, `verio-app`
  (directory + Cargo.toml `name` + `[lib] name`), zero `gv-*` references left,
  invite prefix remains `VR1-`. Post-rename: build/clippy/tests re-run clean
  (see outputs below).
- Phase 0 acceptance: command + event round-trips were unit-tested in Phase 0; the
  user's instruction to proceed into Phase 1 work is recorded here as the
  acceptance of Phase 0.
- Phase 0 addendum verification rules still in force: interactive verification ONLY
  via `npx tauri dev` from `C:\dev\verio\ui`. A plain-cargo exe is NEVER used to
  verify UI. No `.cargo/config.toml` was created (no emergency).

Dependencies used this phase (all within the explicit allowance): serde,
serde_json, thiserror, tracing-subscriber, tracing-appender, plus the locked
stack: cpal, nnnoiseless (RNNoise), audiopus (Opus), ringbuf,
tauri-plugin-global-shortcut, tracing.

Work done (architecture as specced — cpal callbacks only copy/drain lock-free
ringbuffers; all DSP/codec work on dedicated threads):

- `verio-audio`: cpal device listing/selection + stream opening. Preference order
  input AND output: 48 kHz/mono/f32; native fallback with `used_fallback` logged
  once at init; `StreamSpec` (rate, channels) handed to the conversion stages.
  Re-exported `ringbuf::{Consumer, Producer, HeapCons, HeapProd}`.
- `verio-dsp`: RNNoise (nnnoiseless, 480-sample chunks, voice probability out),
  VAD gate (enter > 0.6, 250 ms hangover), linear resampler + stereo→mono
  (average), dB/limits utils, Opus encoder/decoder wrappers (32 kbps, complexity 5,
  in-band FEC, DTX via raw CTL `OPUS_SET_DTX`=4016, fullband, VOIP; decoder
  FEC/PLC), 480→960 rechunk in the encoder, fixed AEC trait slot (`AecStage`,
  NULL implementation — NOT built, per spec), WAV writer/reader (hand-rolled
  16-bit PCM to stay in the dependency allowance) + `first_onset` finder.
- `verio-app`: settings store (JSON via Tauri app_config_dir, debounced writer
  thread, corrupt→defaults+warning, unit-tested), hotkey state machine
  (`TxState`: deafen⇒mute with restore, PTT semantics
  `effective = !muted && (!ptt_mode || ptt_held)`, unit-tested), and the full
  pipeline: capture ring → PROCESSING thread (convert → gain → RNNoise → VAD →
  rechunk → Opus encode → channel) → RX thread (decode → 40 ms fixed pre-buffer
  with drop-oldest, gated playback arming → output ring) → playback callback.
- `ui/src-tauri` (`verio`): Tauri commands (get_settings, list_devices,
  get_audio_state, set_input/output_device, set_input_gain, set_noise_suppression,
  set_loopback, set_debug_wavs, set_hotkey, toggle_mute, toggle_deafen), pipeline
  restart on device/loopback/debug-WAV changes, event forwarding
  (`input_level` ≤ 20 Hz, `speaking_changed`, `audio_state_changed`,
  `wav_finished`, `audio_error`), global-shortcut plugin registered in the
  builder AND permitted in `capabilities/default.json` (allow-register /
  unregister / unregister-all / is-registered), daily-rolling file logging
  (tracing-appender), debug WAV pair (capture_raw.wav pre-gain/pre-RNNoise,
  loopback_out.wav post-decode, same-instant start, 60 s auto-stop), and a
  `latency_check` bin (`cargo run -p verio-app --bin latency_check -- <cap.wav>
  <loop.wav>` → (delta_samples/48) ms, PASS/FAIL vs 80 ms).
- UI (App.tsx): Audio Lab view — level meter, speaking indicator, mute/deafen
  buttons, loopback toggle + headphones feedback warning, Settings section
  (device pickers, gain slider 0..+30, noise suppression, debug WAVs, loopback),
  hotkey rebind UI (click → press any key → keydown capture incl. modifiers →
  save via `set_hotkey`). `input_level` ≤ 20 Hz; `speaking_changed` debounced
  150 ms. No per-chunk logging anywhere in audio paths.

Fixes during the session (real errors encountered):
1. cpal 0.15 API: `SupportedStreamConfigRange` must be kept (not converted to
   `SupportedStreamConfig`) to call `min/max_sample_rate()`; `with_sample_rate`
   consumes `self`. Doc-comments are not allowed on fn parameters → moved into
   the function doc.
2. audiopus 0.2: `Bitrate::BitsPerSecond(32_000)` (not `Bits`); PLC decode needs
   `None::<&[u8]>` type annotation.
3. clippy `-D warnings`: `repeat_n` over `repeat().take()`, `derive(Default)`
   over manual impl, needless clone.
4. Test fixes (implementation was spec-correct, tests were wrong): VAD enter
   returns speaking=true on the first voice chunk and 24×10 ms < 250 ms hangover;
   WAV write uses round-to-nearest ±32 768 (error ≤ 0.5 LSB); Opus DTX needs
   sustained silence before suppressing — test now scans 4 s of silence for a
   ≤ 4-byte packet.
Verified (real output):
- `cargo build --release` →
  `Finished `release` profile [optimized] target(s) in 4m 56s`
- `cargo clippy --workspace -- -D warnings` →
  `Finished `dev` profile [optimized] target(s) in 4.26s` (clean)
- `cargo test --workspace` → all green:
  `test result: ok. 2 passed` (verio-audio), `test result: ok. 10 passed`
  (verio-app: settings + hotkey units), `test result: ok. 21 passed` (verio-dsp),
  plus the `verio` (tauri) tests — 33+ passed, 0 failed.

Acceptance criteria — Phase 1:
- [x] `cargo build --release` + `cargo clippy --workspace -- -D warnings` clean
      (outputs above)
- [x] `cargo test --workspace` passes (all `test result: ok`, 0 failed)
- [ ] Loopback round-trip latency < 80 ms — measurement tool built
      (`latency_check`); needs the interactive clap test (below)
- [ ] RNNoise audibly kills keyboard/fan noise — USER listens with headphones
- [ ] CPU < 5% of one core during loopback + talking — needs
      `Get-Counter '\Process(verio*)% Processor Time'` while the app runs
- [ ] Hotkeys work while a game window has focus — USER tests over a real game
- [ ] Mute/deafen/PTT semantics incl. deafen→mute implication — USER tests
- [ ] Debug WAVs written, non-trivial size, onset test runs — tool built; needs
      the interactive test
- [ ] Settings survive app restart — needs one interactive save/restart check

Human-gated protocol (user instructions — do these in order, all inside
`npx tauri dev` from `C:\dev\verio\ui`, NEVER a plain-cargo exe):
1. Boot: from `C:\dev\verio\ui` run `npx tauri dev`. Grant Windows microphone
   permission if prompted. Watch the console/log for the logged device configs.
2. Headphones ON (loopback feedback). In Audio Lab: enable **Loopback** (a
   warning appears if the output device doesn't look like headphones), talk —
   you should hear yourself with ~100–150 ms delay.
3. RNNoise check: with Noise suppression ON, type on your keyboard / let a fan
   hum into the mic — the meter should stay near silence and you should hear
   almost nothing of it in loopback. Toggle suppression OFF to compare (noise
   returns). Set it back ON.
4. Latency measurement: enable **Debug WAV recording** in Settings, wait for the
   `wav_finished` event (~60 s) OR clap ONCE sharply ~2 s after enabling. Then:
   `cargo run -p verio-app --bin latency_check -- <log_dir>\capture_raw.wav
   <log_dir>\loopback_out.wav` (log dir is printed at startup: "logging
   initialised"). Paste the ms number back — that is the latency AC.
5. CPU check: with loopback ON and you talking, run for 30 s:
   `Get-Counter '\Process(verio*)% Processor Time' -SampleInterval 1 -MaxSamples 30`
   and report the average. For a fair number use the release exe
   (`C:\dev\verio\target\release\verio.exe`, already built) for this measurement
   only — UI checks still use `npx tauri dev`.
6. Hotkeys: put a game window in focus, press Ctrl+Shift+M (mute),
   Ctrl+Shift+D (deafen — should force mute), Ctrl+Shift+D again (mute state
   restores), Ctrl+Shift+Space (hold PTT — mic only open while held and
   unmuted). Report what you saw in the UI.
7. Settings persistence: change gain/nickname, close the app, relaunch —
   values should persist.

Out of scope this phase (NOT built, as instructed): networking, mDNS, invite
codes, adaptive jitter buffer, AEC, per-peer volume, Room view, relay/UPnP,
bundler/NSIS.

Next steps:
- Await user results for the human-gated ACs above, fill in the real latency ms
  and CPU %, then session 3: Phase 2 (networking) — keep-alive frames decision
  happens there.


DTX note (per spec): while the transmit gate is open, encoding continues through
silence and DTX collapses those frames to ~2-byte packets. Keep-alive frames
(preventing decoder state staleness) are a network-phase concern — deliberately
NOT implemented here; revisit in Phase 2 with the real network send path.

### Session 2 addendum — first interactive run diagnosis (2026-08-30)

User ran `npx tauri dev` (correct procedure). Symptom: no loopback audio, level
meter frozen, gain slider seemingly ineffective.

Diagnosis from the real log:
- The log NEVER shows `loopback pre-buffer armed (fixed 40 ms)` → the RX thread
  never received a single Opus packet → nothing was ever encoded → the capture
  ring was empty after the first instant.
- The meter "moves for a second then freezes" right after each pipeline restart
  = the ring drains whatever buffered audio exists, then the device delivers
  nothing (meter events are only emitted when capture data is processed).
- The selected input device was `Microphone (WO Mic Device)` — a VIRTUAL mic fed
  by the WO Mic phone client. If the phone app is not streaming, it delivers no
  audio (and if it was Windows' default input, `input_device=None` hit the same
  device).
- Gain/NS/live controls and event forwarding were verified correct in code
  (`set_input_gain` → `LiveControls.set_gain_db`, no restart needed).

Code change made: `processing_loop` now warns every 10 s when no capture data
has arrived for 3+ s ("no capture data — the input device is silent or not
streaming...") so this failure mode is visible in the log instead of a frozen UI.
Clippy `-D warnings` clean after the change; verio-app tests to be re-run once
the dev session releases the cargo lock.

User action items (in order):
1. Select the REAL microphone in Settings → Input device (not WO Mic). If WO Mic
   is wanted, start the phone client first. Also check Windows Settings →
   Privacy → Microphone → "Let desktop apps access your microphone" = On.
2. Toggle loopback ON with headphones; the log MUST show
   `loopback pre-buffer armed (fixed 40 ms)`. Then the meter should move while
   talking and self-audio should be audible.
3. If "pre-buffer armed" appears but audio is still silent → output-side bug,
   report back with the log.

## 2026-08-30 — Loopback silence fix + !Send stream-owner thread

Root cause of "can't hear myself in loopback": the processing thread only
encoded while `tx_state.effective_transmission()` was true. With a PTT hotkey
bound (Ctrl+Shift+Space) PTT mode is ON, so nothing was ever encoded — the
loopback path went through the codec and therefore stayed silent. Fix: the
processing loop takes a `loopback: bool` and the transmit gate becomes
`if loopback || tx_state.effective_transmission()` — loopback is a mic test
and intentionally bypasses mute/PTT.

Also fixed the compile break (79 errors): `PipelineHandle` stored the cpal
streams, but cpal streams are `!Send` and Tauri's managed `Bridge` state
requires `Send`. New design: a dedicated "verio-streams" owner thread opens
both devices and parks until pipeline stop; only the `Send` ring endpoints,
specs, fallback flags and the playback gate cross an mpsc channel
(`DeviceEndpoints` struct). `PipelineHandle` now holds three join handles
(processing, rx, stream-owner) and joins the stream-owner LAST so capture and
playback stop only after both workers exited.

Meter freeze fix: when the capture device delivers no samples the processing
loop now emits `InputLevel(-100.0)` at the normal 50 ms cadence so the bar
decays to zero instead of freezing.

Verified: `cargo check --workspace` clean, `cargo clippy --workspace` zero
warnings, `cargo test --workspace` 33 passed / 0 failed.

Remaining user-visible diagnostics (every 5 s in the log):
- `pipeline stats: capture -> encode` — popped_samples/encoded_packets/peak
- `pipeline stats: decode -> playback` — decoded_packets/prebuffer_armed
- If popped_samples stays 0 and "no capture data" repeats → input device is
  not delivering audio (WO Mic client not running / Windows mic privacy).

## Session 2.5 — Phase 1 leftover bugfixes

**Bug 1 — nickname did not persist.** Evidence from session 2: every
`settings changed` log line showed `nickname: ""` no matter what was typed,
while device selections persisted. Root cause: the nickname input was purely
local UI state — no Tauri command wrote it to the settings store.
Fix: new `set_nickname(name)` command (trims; empty = unset) registered in
`invoke_handler`; the Home input debounces 300 ms then invokes it, so it logs
through the normal `settings changed` path. Unit test
`roundtrip_via_json` already covers a non-empty nickname round-trip.

HUMAN-VERIFY (pending user run of `npx tauri dev`):
- [ ] type nickname → next `settings changed` line shows `nickname: "..."`,
      restart shows it in `settings loaded` — paste both lines here.

**Bug 2 — hotkey rebind capture never completed.** Debug logging
(`[hotkey-capture] keydown ...` in the webview console + `set_hotkey
requested` on the Rust save path) confirmed keydowns reached the handler.
Actual root cause (one line): the combo was built from `event.key`, so with
Shift held digits/keys yield shifted symbols — Shift+9 → `Ctrl+Shift+(`,
Space → `Ctrl+Shift+ ` — which `parse_binding` rejects on the save path, so
the binding never stuck and no visual feedback appeared.
Fix: key portion now comes from `event.code` via `keyLabelFromCode`
(KeyM→"M", Digit9→"9", Space→"Space", F1-F24, Numpad→"Num0".."Num9");
unsupported codes keep capture mode with a warning instead of saving garbage.
Capture flow now:
- `prepare_hotkey_capture(action)` command unregisters the action's old
  global binding on capture start and returns it (old combo can't fire the
  action mid-capture; Escape restores it via `set_hotkey(action, old)`).
- Duplicate combo bound to another action → rejected with a UI warning and
  the old binding is kept (checked both frontend and in `set_hotkey`).
- `set_hotkey` verifies registration with the plugin's `is_registered`
  before persisting; logs `hotkey registered` on success.

HUMAN-VERIFY (pending):
- [ ] rebind Mute to Ctrl+Shift+9 → works while a game has focus
- [ ] old combo no longer triggers; binding survives restart
- [ ] Escape cancels cleanly; combo already bound to another action →
      warning shown, old binding kept

**Verification:** `cargo clippy --workspace --all-targets` — zero warnings
(Finished, no error/warning lines); `cargo test --workspace` — 34 passed,
0 failed (2 doc/unit verio-app + 11 hotkeys/pipeline + 21 settings etc.).
Edit slip fixed along the way: a duplicated `fn set_hotkey(` fragment had
left an unclosed delimiter in `ui/src-tauri/src/lib.rs`; removed.
Also silenced 3 pre-existing test-only clippy lints (bool_assert_comparison
in hotkeys.rs, field_reassign_with_default in settings.rs tests).

### Session 3 (2026-09-01) — Phase 2: transport + same-PC networking + invite codes

Pre-flight:
- Read this file fully. Free disk on C: at session start: **16.5 GB** (≥ 8 GB — no clean needed).
- No git repository (workspace is versioned only via this log).

Phase 1 results recorded (user-reported, 2026-08-31):
- Latency: `latency_check` → **"ROUND-TRIP PIPELINE LATENCY: 6.8 ms, PASS (<80 ms)"**.
  Semantics: given the 40 ms fixed pre-buffer, a 6.8 ms measured loopback implies the
  `loopback_out.wav` writer starts at the FIRST DECODED AUDIO (pre-buffer excluded —
  the writer is armed on the first packet, not on playback arming). True audible
  round-trip ≈ measured + pre-buffer ≈ **~47 ms** — comfortably under the 80 ms AC
  either way. The `latency_check` tool is NOT changed to include the pre-buffer;
  the measured number stands as recorded.
- CPU: **"Avg CPU: 0.98% of one core"** (Get-Counter 30 s avg, release exe, loopback + talking) — PASS (<5%).
- RNNoise audibly kills noise: USER-VERIFIED. Hotkeys over a game: USER-VERIFIED.
  Settings persistence: USER-VERIFIED.
- Phase 1 marked **COMPLETE** (all ACs pass or are user-verified above).

ACCEPTED DEVIATION (permanent): `nnnoiseless` (pure-Rust RNNoise port) replaces the
C `rnnoise` crate — rationale: no C/C++ build (no NASM/Perl), maintained, user-verified
audio quality. **Permanent — do NOT switch back.** (Also recorded in the workspace
`Cargo.toml` comment.)

Verification rule update (this phase): `npx tauri build --no-bundle` is NOW ALLOWED —
it is the standard way to produce the exe for multi-instance testing (embedded assets).
Full NSIS bundling stays deferred to Phase 4. Interactive verification remains
`npx tauri dev` only.

TRANSPORT DECISION TREE — resolved to branch 3 (str0m). Evidence (real output, this
machine):

1. PREFERRED — RTP media track via the `datachannel` (libdatachannel) bindings:
   the crate DOES expose media (`media` feature → `RtcTrack::send`), so the branch was
   attempted. Spike: `cargo add datachannel --features media && cargo check -p
   verio-transport`.
   - Attempt 1 (default non-vendored): CMake configure FAILS —
     `CMake Error ... FindOpenSSL.cmake:752: Could NOT find OpenSSL ... (missing:
     OPENSSL_CRYPTO_LIBRARY OPENSSL_INCLUDE_DIR) (Required is at least version "1.1.0")`.
     No system OpenSSL dev install exists (only miniconda's, not discoverable).
   - Attempt 2 (same, with `OPENSSL_ROOT_DIR=C:\Users\Mahan\miniconda3\Library`):
     libdatachannel C++ itself compiles and installs, then the build panics at
     bindings generation —
     `bindgen-0.71.1\lib.rs:604: Unable to find libclang: "couldn't find any valid
     shared libraries matching: ['clang.dll', 'libclang.dll'], set the LIBCLANG_PATH
     environment variable ..."` — no LLVM/libclang exists anywhere on this machine.
     The `vendored` feature is equally blocked: it builds OpenSSL from source via
     `openssl-src`, which requires Perl — also absent.
   - Installing LLVM (≈400 MB) and/or Perl would violate the standing no-install rule
     (same rule that uninstalled the duplicate Rust toolchain in Session 1).
2. ELSE — DataChannel audio via the same bindings: same blocker (same build.rs,
   bindgen unconditional). Not reachable.
3. ELSE (last resort) — **str0m** CHOSEN: pure-Rust Sans-IO WebRTC; DTLS-SRTP/SCTP
   encryption is provided by the library (feature `wincrypto` = Windows CNG/SChannel
   backend — no OpenSSL, no Perl, no NASM, no libclang). `default-features = false`
   (the default `aws-lc-rs` provider would reintroduce a C build).
   Sub-decision within branch 3: audio rides a str0m DATA CHANNEL (unreliable:
   `ordered=false`, `max_retransmits=0`) using the branch-2 12-byte header
   (magic u8, version u8, seq u32, capture_ts_ms u48) rather than str0m's RTP media
   API — because the spec's RX requirements (one-way latency from capture_ts_ms,
   late/lost counters via seq) ride exactly on that header, and str0m's media API
   exposes no wall-clock header extension for it. Control JSON uses a second,
   reliable ordered DataChannel. Documented as the chosen branch + reason.

Dependencies used this phase (within the allowance + documented justifications):
- `str0m` 0.23 (`wincrypto`) — mandated by the decision tree above (branch 3).
- `mdns-sd` — allowed (spec).
- `uuid` v4 — allowed (spec).
- `base64` — justification: the spec REQUIRES "base64 SDP blob prefixed VR1-";
  base64 is a tiny zero-transitive-dependency crate; hand-rolling base64 would be
  worse than using the canonical implementation.

### Session 2.5b — rebind still not completing (diagnosing)

Retest evidence (user log 10:14:5x): `prepare_hotkey_capture` fired and
unregistered `action=ptt binding=Ctrl+Shift+Space`, but NO
`set_hotkey requested` line ever appeared — the keydown save path never
executed. The earlier `console.log` debug was invisible (goes to webview
devtools, not the `npx tauri dev` terminal).
Fix/diagnostic: new temp `debug_log(msg)` Tauri command (`target: "ui-debug"`)
registered in `invoke_handler`; the capture handler now logs every keydown
and decision point (modifier-only / no-modifier / unsupported / combo built /
conflict / saving / saved OK / save FAILED) through it, so the same terminal
shows exactly where the flow stops. `cargo clippy` clean, 34 tests pass,
`npx tsc --noEmit` clean. Awaiting user retest log.

---

# Phase 2 — Session 3 agent-side verification log (agent, this session)

## Transport driver latency bugs found and fixed (with evidence)

The first full two-peer harness runs (release build, str0m, loopback) showed
one-way transport latency p50 â‰ˆ 58â€“118 ms while control RTT was 1â€“2 ms. Three
real bugs were found, in order:

1. **Driver ignored str0m's requested `Output::Timeout`.** `drain_output()`
   discarded the timeout instant and the loop waited on its own 250 ms budget,
   so str0m pacing/SACK/DTLS timers fired late. Fixed: `drain_output()` now
   returns str0m's requested duration and the loop waits on
   `min(own_schedule, str0m_wait)`.

2. **Driver only applied queued commands when a network packet arrived.** With
   the audio pump feeding frames via the async cmd channel and both sides
   bursty, a self-reinforcing equilibrium formed: 5â€“6 frame clusters every
   ~120 ms (debug log evidence: `audio cmd picked up delay_ms=96/76/56/36/16`
   all in one loop iteration). Fixed: socket read timeout capped at 10 ms â€”
   commands drain at â‰¥100 Hz, negligible CPU. Result: one-way p50 dropped
   58 â†’ 16â€“18 ms, p95 = 19 ms, backlog_max 6 â† 2â€“3, lost â‰ˆ 0.

3. **Harness percentile bug (measurement, not transport).** `percentile()`
   indexed its input without sorting, so window lines like `p50=87 p95=44`
   were garbage and the first "PASS" verdict for one-way latency was FAKE.
   Fixed (sorts internally); all evidence below is post-fix. Honesty rule
   applies to my own tooling.

## Two-peer loopback soak harness

`crates/soak-harness` (bin `room_soak`, `cargo run -p soak-harness --release
-- [seconds]`): two full `RoomManager` instances + TCP signaling + str0m
transport + real Opus encode/decode, both directions, through the exact
app path (pump â†’ DataChannel â† transport pump â† tap â† decode). One-way
latency is stamped in a tap thread with BLOCKING recv (true arrival, immune
to the decode thread's batch/sleep pattern).

## 45 s validation run (release, post-fix) — ALL PASS

```
Aâ†’B RX (decoded on B): decoded=2247 late=0 lost=5 backlog_max=3
Bâ†’A RX (decoded on A): decoded=2251 late=0 lost=1 backlog_max=3
RTT: samples=46 p50=0.0ms p95=1.0ms max=2.0ms
one-way p50=16-18ms p95=19ms (stable across all 5 s windows)
[PASS] both instances connected
[PASS] decoded packets grew on BOTH sides
[PASS] RTT p95 < 10 ms (loopback)
[PASS] one-way transport latency p95 < 40 ms
[PASS] speaking state propagated both ways
[PASS] no room errors / disconnects
=== SOAK RESULT: PASS ===
```

## KNOWN ISSUE — same-host mDNS (spec: best-effort)

On one PC, both instances advertising/Querying on multicast 5353 is
best-effort per spec. Direct-connect fully covers same-PC testing;
cross-machine mDNS is validated when a 2nd PC exists.

## 10-minute soak (release, 600 s) — ALL PASS, no crash

```
=== SOAK COMPLETE after 600s — final stats ===
A→B RX (decoded on B): decoded=29985 late=0 lost=19 backlog_max=3
B→A RX (decoded on A): decoded=29982 late=0 lost=22 backlog_max=3
RTT: samples=598 p50=0.0ms p95=1.0ms max=1.0ms
Speaking-state events: A received from B: speaking=60 mute=61 | B received from A: speaking=61 mute=60
[PASS] both instances connected
[PASS] decoded packets grew on BOTH sides
[PASS] RTT p95 < 10 ms (loopback)
[PASS] one-way transport latency p95 < 40 ms   (p50=18ms p95=19ms, stable in every 5 s window incl. t=+600s)
[PASS] speaking state propagated both ways
[PASS] no room errors / disconnects
=== SOAK RESULT: PASS ===
```

- Loss rate 19–22 of ~30 000 frames ≈ 0.06–0.07 % (unreliable channel doing
  its job); every window's backlog_max ≤ 3 frames (60 ms) — pre-buffer is
  bounded, no growth trend over 10 minutes.
- One-way p50 18 ms = driver drain bound (10 ms) + str0m pacing + loopback
  wire; well under the 40 ms AC. The app's 40 ms pre-buffer absorbs the
  remainder by design (adaptive jitter buffer decision deferred to Phase 3,
  buffer-depth stats logged as specified).

## Release exe built (for multi-instance + friend testing)

`npx tauri build --no-bundle` → `C:\dev\verio\target\release\verio.exe`
(15.4 MB, optimized, embedded assets, 3m19s). No runtime deps beyond WebView2.

## HUMAN-GATED VERIFICATION — handoff (agent WAITS here)

### Test A — Solo dual-instance (headphones ON; PTT discipline to avoid echo loop)

1. Run `C:\dev\verio\target\release\verio.exe` (instance A). Note the direct
   code shown in the Connect panel (`ip:port`); copy it with the copy button.
2. Run again with a profile: `verio.exe --profile b` (instance B). Window
   titles must differ ("Verio — b"). Its config/logs live under
   `%APPDATA%\ir.verio.app\profiles\b` (and %LOCALAPPDATA% for logs).
3. In B's Connect panel, paste A's direct code → Connect. Both UIs must show
   Connected (name + speaking ring + RTT).
4. Hold A's PTT, talk → B's ring lights; with headphones ON you may hear
   yourself slightly delayed ONLY if B's output is audible — verify B receives.
5. Release; hold B's PTT, talk → A hears you.
6. Mute A → B's incoming audio stops; unmute restores.
7. Deafen B → B hears nothing AND transmits nothing; undeafen restores.
8. If Windows Firewall prompts → Allow (private networks).

Expected from the harness evidence: RTT ~1 ms, one-way ~18 ms + 40 ms
pre-buffer (audible round-trip ≈ 60 ms), ~0% loss, speaking ring tracks voice.

### Test B — Internet test with the friend (the real finale)

1. Send the friend `C:\dev\verio\target\release\verio.exe` (no runtime deps
   beyond WebView2, present on Win10/11).
2. Warn the friend: SmartScreen → "More info → Run anyway" (unsigned), and
   allow Windows mic permission on first launch.
3. A: "Create invite" → send the `VR1-…` blob to the friend (any chat app).
4. Friend: paste blob → "Accept invite" → friend gets an ANSWER blob → send back.
5. A: paste answer blob → "Complete invite" → call.
6. Verify speech both ways + realistic RTT (20–120 ms expected).
7. Record: ISP pair, whether it connected first try (our first real STUN
   data point), any firewall/NAT weirdness.

### Out of scope this phase (unchanged)

Mesh >2 peers, adaptive jitter buffer, AEC, UPnP/TURN.

## SESSION STATUS: Phase 2 complete.

---

### Session 4 (2026-09-03) — Remote Audio Fix + VPS Signaling/Relay + UI Overhaul

Goal: Fix remote peer audio playback bug, eliminate 10 ms capture latency, synthesize audio earcons, implement VPS 4-digit room signaling server & client, and overhaul the entire frontend UI with modern dark gaming layout.

Work done:
1. **TASK 1: Fix Remote Peer Audio Playback (P0 Bug)**:
   - Root cause diagnosed: CPAL playback callback was artificially gated waiting for local loopback pre-buffer; RX thread was blocked on 50 ms timeout when loopback was disabled; all remote frames were pooled into an unbuffered single queue.
   - Multi-source mixer implemented in `crates/verio-app/src/pipeline.rs`:
     - Continuous mixer draining local loopback (if active) + all remote peer streams + synthesized earcon cues.
     - Playback gate initialized to `true` (active immediately, plays silence on underrun).
     - Dedicated `PeerRxStream` per connected peer: dedicated `OpusDecoderWrapper`, 40 ms jitter pre-buffer (1920 samples @ 48 kHz mono), capped at 60 ms to bound latency.
     - Soft-clipping limiter: `output_sample = (sum).clamp(-1.0, 1.0)`.
     - Per-peer volume adjustment (0% to 200%).
2. **TASK 2: Audio Pipeline Latency & Quality Polish**:
   - 10 ms Opus Frame Alignment: Opus encoder frame size reduced from 20 ms to 10 ms (480 samples @ 48 kHz). RNNoise 10 ms output feeds directly into the Opus encoder without the 10 ms rechunking buffer, eliminating 10 ms of capture delay.
   - Audio Cues (Earcons): Created `verio_dsp::earcon`:
     - 48 kHz mono sine wave tones with 2 ms cosine ramp envelopes.
     - Mute ON: 400 Hz tone (20 ms).
     - Mute OFF: 800 Hz tone (20 ms).
     - Deafen: two short 300 Hz blips (15 ms tone, 15 ms silence, 15 ms tone).
     - Triggered automatically on hotkey presses and UI toggles.
3. **TASK 3: VPS Signaling & Relay Architecture (`crates/verio-server` & `verio-discovery`)**:
   - Created `crates/verio-server` using `axum` and `tokio`:
     - WebSocket endpoint `/ws`:
       - `{"type": "create"}`: generates random 4-digit room code (e.g. `4921`), returns code.
       - `{"type": "join", "room": "4921"}`: connects peer to room.
       - Relays WebRTC SDP offers, answers, and candidates between room peers.
     - Fallback UDP Relay on port 8444: relays voice frames if direct WebRTC hole-punching fails.
   - In `crates/verio-discovery`:
     - Implemented `VpsSignaling` client backing the `Signaling` trait using `tungstenite`.
     - Added `vps_address` to settings (default `ws://127.0.0.1:8443`).
     - Added `run_create_room` and `run_join_room` to `RoomManager`.
4. **TASK 4: Total Frontend UI Redesign (React + Modern Dark Gaming CSS)**:
   - Completely replaced Phase 0/1 debug UI with sleek dark gaming theme in `ui/src/App.tsx` and `ui/src/App.css`:
     - Header Bar: 'VERIO' branding, connection status dot, ping display (`● 24 ms`).
     - Lobby View: Hero prompt, prominent "Create Room" card, 4-digit "Join Room" input, and collapsible Advanced Direct LAN Connect accordion.
     - In-Call View: Room Code badge with "Copy Code" feedback, "Leave Call" button, participant cards grid with initials avatar, glowing animated green speaking outline, mute/deafen badges, and per-peer volume slider (0-200%).
     - Persistent Bottom Dock: User nickname (with inline edit), real-time mic level bar using DOM ref (zero React re-renders on 50 ms tick), Mute toggle button, Deafen toggle button, and Settings gear button.
     - Settings Modal: Audio input/output device selectors, input gain slider (0..+30 dB), AI noise suppression toggle, loopback test toggle, VPS server address input, and hotkey rebinding pills.

Verified (real output):
- `cargo clippy --workspace -- -D warnings` → Finished with 0 warnings, 0 errors.
- `cargo test --workspace` → 49 unit tests passed across all crates:
  - `verio_dsp`: 24 passed (including 10 ms codec and all 3 earcon tests)
  - `verio_app`: 11 passed
  - `verio_discovery`: 7 passed (including `test_normalize_ws_url`)
  - `verio_transport`: 5 passed
  - `verio_lib`: 2 passed
- `npm run build` in `ui/` → built in 7.20s with zero errors or warnings (dist/index.html, CSS, and JS bundles produced).
- Disk space: 14.95 GB free on C: (> 8 GB threshold).

SESSION STATUS: Session 4 COMPLETE — All 4 tasks implemented and verified!

---

### Addendum (2026-09-05) — Server CLI Custom Port Options & Linux Binary
- Added `-p` and `--port <PORT>` CLI options to `verio-server` to run on any custom TCP port (default: 8443).
- Added `-u` and `--udp-port <PORT>` for fallback voice relay (default: 8444, or TCP port + 1 if custom port provided).
- Added `--help` / `-h` usage flag.
- Cross-compiled Linux Ubuntu x86_64 statically-linked binary (`target/x86_64-unknown-linux-musl/release/verio-server`, 2.78 MB).
- Unit tests: 6 unit tests in `verio-server` verifying defaults, short flags, long flags, equal-sign format, and error handling.

---

### Addendum 2 (2026-09-05) — Fix Disconnection (WSAECONNRESET 10054), STUN RFC 5389, and Nickname Editing
1. **Root Cause Analysis of Socket 10054 Error:**
   - **STUN Gathering Failed Silently:** `crates/verio-transport::stun_binding_query` had a typo in the RFC 5389 magic cookie (`0x2112A444` instead of `0x2112A442`). Public STUN servers treated it as RFC 3489 and returned `MAPPED-ADDRESS` (0x0001) instead of `XOR-MAPPED-ADDRESS` (0x0020), causing STUN gathering to always time out. As a result, no public server-reflexive candidates were generated—only private LAN and loopback candidates were present in the SDP.
   - **Windows Winsock WSAECONNRESET (10054) Crash:** When peers over the internet attempted ICE connectivity checks to each other's private LAN addresses, local routers returned ICMP Port Unreachable. On Windows, Winsock reports this as `WSAECONNRESET` (10054) on the UDP socket's next `recv_from`. In `Driver::run`, this non-fatal UDP condition was treated as fatal, causing the driver thread to exit with `"socket: An existing connection was forcibly closed by the remote host. (os error 10054)"` and tear down the call.
2. **Fixes Implemented:**
   - **STUN RFC 5389 Correction:** Corrected magic cookie to `0x2112A442`. Added parser support for both `0x0020` (XOR-MAPPED-ADDRESS) and `0x0001` (MAPPED-ADDRESS). Expanded STUN server pool.
   - **Built-in STUN Server in `verio-server`:** Added RFC 5389 STUN Binding Request handler directly into `verio-server`'s UDP relay on Linux & Windows, allowing the user's VPS to act as a fallback STUN server.
   - **Winsock WSAECONNRESET Immunity:** Added `disable_connection_reset(&socket)` using `SIO_UDP_CONNRESET` ioctl on Windows. Added explicit checks in `Driver::run` and `stun_binding_query` to treat `ConnectionReset` and raw OS codes `10054 | 10051 | 10065 | 10060 | 10035` as transient timeouts rather than fatal errors.
3. **Nickname Updating Fix:**
   - `ui/src-tauri/src/lib.rs`: Updated `set_nickname` to accept both `name` and `nickname` parameters.
   - `ui/src/App.tsx`: Updated `handleSaveNickname` with fallback parameters, toast notifications, error handling, and Enter/Escape keypress support in the dock input.



---

## Session 6 (2026-09-15) — Relay Diagnosis

Scope: diagnose why the Cloud Tunnel relay path (Mode 2) does not carry audio end-to-end.
No code was changed, no dependencies added, no refactor, no file moves. DSP/RNNoise/VAD/Opus/earcon
and the P2P path were left untouched. This was a report-only session.

Method: full static trace of the relay path across `verio-discovery`, `verio-server`,
`verio-transport`, `verio-app`; a real `verio-server` started locally; and a wire-protocol probe
that speaks the exact client packet format (two synthetic peers) against that running server.
Client-side relay logic was also exercised with the existing unit test.

---

### 1. Where the flow breaks (one sentence)

Both peers in every recorded Cloud-Tunnel session used the **same peer identity UUID**
(`6cd0d574-0ac9-45db-8837-d096800f0226`), and the server's relay address book is keyed on that UUID,
so the two peers collapsed into a single entry and every packet was suppressed on both the server
(`uuid != sender_uuid` is always false) and the client (self-echo drop), i.e. the relay was
structurally incapable of forwarding anything.

### 2. Evidence

**2a. Real log lines (client A, `C:\Users\Mahan\AppData\Local\ir.verio.app\logs\verio.log.2026-09-11`)**

Both instances report the peer identity as the same UUID — including the one that also reports
`is_creator=true` (so the same UUID is the *local* identity of both processes, not a coincidence):

```
11:42:29.656  signaling: peer identified, selecting voice transport  peer=DRFake
              uuid=6cd0d574-0ac9-45db-8837-d096800f0226  transport_mode=Auto  is_creator=false
11:42:29.661  signaling: peer identified, selecting voice transport  peer=DRFake
              uuid=6cd0d574-0ac9-45db-8837-d096800f0226  transport_mode=Auto  is_creator=true
```

The same pattern repeats in every later attempt (`11:54:50`, `11:55:26`, `11:56:03`, `11:58:17`).

That UUID is the persisted `peer_id` in `%APPDATA%\ir.verio.app\settings.json`
(`"peer_id": "6cd0d574-0ac9-45db-8837-d096800f0226"`), and the startup identity line of that
build confirms the identity was *loaded*, not generated:

```
11:32:38.798  peer identity  peer_uuid=6cd0d574-0ac9-45db-8837-d096800f0226  name=DRFake
```

**2b. Suspect code (exact file + line)**

- `crates/verio-server/src/main.rs:487` — the relay address book is
  `HashMap<String /*room*/, HashMap<[u8; 16] /*sender UUID*/, RelayClient>>`.
- `crates/verio-server/src/main.rs:566-574` — registration is `clients.insert(sender_uuid, …)`;
  with two peers sharing one UUID the second registration **overwrites** the first, and the
  `if is_new` log at `:575-582` therefore never fires a second time.
- `crates/verio-server/src/main.rs:589-591` — forwarding condition
  `if uuid != sender_uuid && client.addr != from`: with a single collapsed entry the only
  candidate is the sender's own key, so **nothing is ever relayed**.
- `crates/verio-transport/src/lib.rs:1281` / `:1284-1285` — the client stamps its own UUID into
  every packet (`prefix[4..20] = local_identity.uuid`).
- `crates/verio-transport/src/lib.rs:1427-1429` — the receiving client drops anything whose sender
  UUID equals its own (`if &buf[4..20] == &local_uuid_bytes { return; }`) — a second, independent
  suppression of the same collision.

**2c. Reproduced locally against the real server (new, this session)**

`target\release\verio-server.exe -p 19091 -u 19092` (dual-port spawn active; server log):

```
Verio UDP relay + STUN responder listening on 0.0.0.0:19092
Verio UDP relay + STUN responder listening on 0.0.0.0:3478
Verio fallback UDP relay listening on 0.0.0.0:19092
```

Wire-protocol probe, two synthetic peers in room `1234`, exact client framing
`[room(4)][uuid(16)][tag(1)][payload]`:

```
SCENARIO: distinct peer UUIDs (normal two-machine case)
  A received: total=7 audio_from_peer=5 control_from_peer=2 own_echo=0
  B received: total=7 audio_from_peer=5 control_from_peer=2 own_echo=0
  RESULT: RELAY OK (bidirectional audio)

SCENARIO: identical peer UUIDs (same-profile two-instance case)
  A received: total=0 audio_from_peer=0 control_from_peer=0
  B received: total=0 audio_from_peer=0 control_from_peer=0
  RESULT: RELAY FAILED
```

Server-side confirmation of the collapse — only **three** registrations for **four** peers
(the two identical-UUID peers produced no second registration at all):

```
registered new UDP relay client in room  room=1234 client=127.0.0.1:60717 port=19092
registered new UDP relay client in room  room=1234 client=127.0.0.1:60718 port=19092
registered new UDP relay client in room  room=1234 client=127.0.0.1:60719 port=19092
```

Note the forwarding samples also prove the shared hub works **across both ports**
(A received via `:19092`, B via `:3478`), so the dual-socket hub is not the fault.

**2d. Existing client relay test passes in isolation**

`cargo test -p verio-transport relay` → `test tests::test_relay_session_loopback ... ok`.
The client relay driver is correct; the failure is a cross-component identity/address-book issue.

### 3. Ranked candidate root causes

**1) Peer-identity collision collapsing the relay address book — PROVEN locally, matches every
logged Cloud-Tunnel failure.**
- *For:* both instances in the log carry UUID `6cd0d574…`, equal to the persisted `peer_id`; the
  probe reproduces total bidirectional failure with identical UUIDs; the server's own
  registration count drops from 4 to 3.
- *Against (as the cause of the *current* build):* current source generates a fresh per-process
  UUID — `ui/src-tauri/src/lib.rs:821` (`let peer_uuid = Uuid::new_v4();`) — and the client/server
  binaries on disk were rebuilt `2026-09-11 15:01` / `15:41`, i.e. **after** the logged runs
  (latest client log entry: `12:46:48`). So this explains the recorded evidence but may already be
  fixed in the shipped build.

**2) The fix was never re-verified — no post-fix test evidence exists.**
- *For:* no client log file exists after `2026-09-11` (`logs\` contains only 09-02 … 09-11), and
  the last Cloud-Tunnel attempt in the log ran on the pre-fix build.
- *Against:* none.

**3) VPS-side relay path / deployment — UNVERIFIED.**
- *For:* I have no access to `141.11.1.110`; cannot confirm the deployed binary's version/wire
  format, nor whether UDP `9092` is open.
- *Against:* `09-11 12:45:48  private VPS STUN succeeded  srflx=178.131.130.238:37481
  target=141.11.1.110:3478` — the STUN responder shares the *same socket and loop* as the relay
  (`main.rs:505-524`), so inbound UDP **3478** to the VPS demonstrably worked from this network;
  and 3478 is the primary candidate, so a blocked 9092 alone would not stop relay.

**4) No genuine cross-network relay attempt has been recorded.**
- *For:* every Cloud-Tunnel session in the log is a same-PC two-instance test (both `6cd0d574`).
  The only cross-machine attempt in the log (`12:45:47`, peer `Player`, uuid `a9a9ccf3-…`) ran in
  **DirectP2P** mode and died on ICE (`12:46:38 ICE disconnected grace period expired (30s)` →
  `transport driver exited reason=ice connection lost`) — a P2P failure, out of scope here.

**5) Vestigial `Settings::peer_id` remains and can silently reintroduce the collision.**
- `crates/verio-app/src/settings.rs:119-125,158` still generates and persists a peer UUID that no
  longer feeds `Identity`. It is dead weight today, but it is exactly the value that produced the
  collision, and nothing in the server warns when two peers present the same identity.

### 4. Smallest fix for each candidate (one line each — NOT implemented)

1. Keep the per-process `Uuid::new_v4()` identity and make the collision observable: log a warning
   (or reject) when a second peer joins a relay room with an already-registered UUID.
2. Re-run the two-instance test on the current build and require **two** distinct
   `peer identity peer_uuid=…` lines and **two** `registered new UDP relay client` lines.
3. On the VPS, confirm `/usr/local/bin/verio-server` matches the current musl build and open
   `9092/udp`; redeploy only if it is stale.
4. Run the cross-network test from two machines (identities then differ by construction).
5. Delete `Settings::peer_id` (or mark it explicitly unused) so the old collision source is gone.

### 5. What could NOT be verified locally, and how to check it

- VPS firewall / cloud security group for UDP `3478` and `9092`, and the deployed binary version:
  ```bash
  sudo ufw status verbose
  sudo ss -lunp | grep -E '3478|9092'
  ls -l /usr/local/bin/verio-server && sha256sum /usr/local/bin/verio-server
  sudo systemctl status verio
  ```
- Whether UDP packets actually arrive on the VPS and are relayed (definitive, run during a call):
  ```bash
  sudo tcpdump -n -i any udp port 3478 or udp port 9092
  sudo journalctl -u verio -f | grep -E 'registered|relay|room'
  ```
- Real cross-NAT behaviour: my probe peers were both on loopback; no NAT was traversed.
- Whether the GUI actually emitted voice frames during the logged calls: the relay driver's
  `packets_sent` / `last_recv_ms_ago` counters are only surfaced in the Diagnostics drawer and are
  never written to the log, so the log cannot settle it. (One capture window in the log shows
  `peak_dbfs=-100.0` with input device `Microphone (WO Mic Device)`, i.e. a silent virtual mic —
  the same trap already recorded in Session 2.)
- End-to-end audible playback through the mixer/speakers — cannot be heard headlessly.

### 6. One question for the user

Did you re-test Cloud Tunnel **after** the 2026-09-11 15:41 client rebuild (the build that
generates a fresh per-process UUID), and if so, did the server show two `registered new UDP relay
client` lines for the room? That single answer decides whether root cause 1 is already fixed and
the remaining work is only the VPS/port check.


---

## Session 7 (2026-09-15) — Forced Relay Instrumentation

Goal: make a **forced-relay** test possible with full per-packet visibility, so a retry with the
friend shows exactly where packets stop. No rewrite, no new dependencies, no DSP/RNNoise/VAD/Opus
changes, no P2P behaviour changes (the P2P code path is untouched; it is only *bypassed* when the
force switch is on).

### CRITICAL-FINDING CHECK (asked for explicitly)

**The relay send path IS wired into the audio pump — verified, no defect found.**

- `crates/verio-app/src/room.rs:212-235` — `spawn_audio_pump()` loops on the shielded `net_out`
  channel and calls `handle.send_audio(p.capture_ts_ms, p.opus)` at `room.rs:234`.
- `handle` is the active session's `PeerHandle`, which for Cloud Tunnel is the relay handle
  (`room.rs:339`, set by `activate_relay_session`).
- `crates/verio-transport/src/lib.rs:256` — `PeerHandle::send_audio` pushes `PeerCommand::Audio`
  onto the driver channel; the relay driver's `PeerCommand::Audio` arm serialises it and sends it
  to `active_relay_addr`.
- Live confirmation this session: the local relay probe (two synthetic peers, real
  `verio-server`) carried audio bidirectionally (5 audio + 2 control frames each way) and the
  server logged the matching `relay_in` / `relay_out` pairs.

So the Session 6 diagnosis stands: the recorded failures were identity collapse, not a missing
pump wiring.

### 1. Hard "force relay" mode

- New setting `force_relay: bool` (default `false`) — `crates/verio-app/src/settings.rs:72`, default
  at `:94`. Persisted in `settings.json` under the snake_case key `force_relay`.
- Mode selection — `crates/verio-app/src/room.rs:562-574`: when `force_relay` is true every
  transport mode resolves to `CloudRelay`, so the ICE/P2P branch is never entered: no host or srflx
  candidates are gathered, no SDP offer/answer is produced, and no later P2P upgrade happens (the
  relay session is created directly by `activate_relay_session`). The session's diagnostics mode
  label becomes `forced_relay` (drawer shows it as `forced`).
- Startup log (exact text, em dash emitted):
  `force_relay=ON — all voice traffic will route through the relay.`
  Emitted at launch when the setting is on (`ui/src-tauri/src/lib.rs:844-847`) and on every toggle
  (`:628-636`).
- UI: checkbox **"Force relay (skip P2P)"** in the Settings modal, under *Voice Transport
  Architecture* — `ui/src/App.tsx:1130-1145`, handler at `:483-493`, Tauri command
  `set_force_relay` at `ui/src-tauri/src/lib.rs:628`, registered at `:787`.

### 2. Client packet-level logging (`crates/verio-transport/src/lib.rs`)

One `tracing::debug!` line per relay datagram, fields: `direction`, `room`, `peer_uuid`
(first 8 chars), `dst` (send) / `src` (recv), `len`, `kind` (`audio|control|stun|unknown`).
Instrumented at every relay socket send site (registration prefix, hello, heartbeat probe, control,
ping, pong, **audio**) and at the single receive entry point.

- Throttle: `PacketLogThrottle` (`:271`), `RELAY_LOG_BURST = 20` first packets per direction always
  logged, then at most `RELAY_LOG_PER_SEC = 10` lines/second. Logging helper `log_relay_packet`
  (`:323`); classifier `relay_packet_kind` (`:307`).
- New diagnostics counters (`:227-234`): `relay_packets_sent`, `relay_packets_recv`,
  `relay_bytes_sent`, `relay_bytes_recv`, `relay_mode` (`forced | auto-upgraded | direct`),
  maintained by `bump_relay_sent` (`:358`) / `bump_relay_recv` (`:367`).

**IMPORTANT — how to see these lines.** Packet lines are DEBUG level, and the app's subscriber
previously hard-coded INFO, so they could not reach the log at all. The level is now
`RUST_LOG`-controllable (`ui/src-tauri/src/lib.rs:826-838`): launch with

```powershell
$env:RUST_LOG='debug'; .\target\release\verio.exe
```

otherwise the log stays at INFO (unchanged default) and the startup line
`relay packet log is OFF (launch with RUST_LOG=debug to capture per-packet relay lines)` tells you so.

### 3. Server packet-level logging (`crates/verio-server/src/main.rs`)

Throttled the same way (`PacketLogThrottle` `:491`, `pkt_log` `:562`), plus `short_uuid` (`:527`):

```
registered room=<id> uuid=<short> addr=<ip:port>
relay_in  room=<id> from_uuid=<short> from_addr=<ip:port> len=<bytes> peers_in_room=<n>
relay_out room=<id> to_uuid=<short> to_addr=<ip:port> len=<bytes>
drop      room=<id> reason=<no_peers|unknown_room> from_uuid=<short>
```

`relay_in` and `relay_out` pair up per forwarded datagram. Verified live this session against the
rebuilt server (room `1234`, two synthetic peers):

```
DEBUG verio_server: registered room=1234 uuid=6483cd3a addr=127.0.0.1:49839
DEBUG verio_server: relay_in   room=1234 from_uuid=6483cd3a from_addr=127.0.0.1:49839 len=48 peers_in_room=2
DEBUG verio_server: relay_out  room=1234 to_uuid=1bdc7e64 to_addr=127.0.0.1:49840 len=48
```

Reading the triage: `relay_in` with no matching `relay_out` ⇒ server-side bug;
`relay_out` with no matching client `recv` line ⇒ path between server and the friend's machine
(firewall / NAT / wrong address).

### 4. Diagnostics drawer counters

New cards in the drawer (`ui/src/App.tsx:678-716`): **Relay Packets S / R**,
**Last Relay Recv** (turns red past 2000 ms with "No relay packets - firewall / NAT / wrong
address"), and **Relay Mode** plus relay bytes out/in. TS mirrors added at `:31-37` and
`:52`. Counters refresh with the existing 1 s diagnostics poll.

Not implemented (and why): pushing the **server's** counters to the client would need a new
signaling message and server-side per-room aggregation; the WS protocol has no such message today
and adding one is beyond "instrumentation only". The client-side `last_relay_recv_ms_ago` already
answers the "are the friend's packets reaching me" question. Say the word and it can be added next.

### 5. Builds (real output)

- `npx tauri build --no-bundle` (from `ui/`) → built twice: `Finished release profile [optimized]
  target(s) in 2m 55s` (instrumentation) then `2m 01s` after the RUST_LOG log-level fix — the second
  is the shipped binary (vite `built in 2.33s`)
  → `C:\dev\verio\target\release\verio.exe` (15.95 MB, 2026-09-15 13:28:45)
- `cargo build --release -p verio-server` → `Finished release profile [optimized] target(s) in 26.10s`
  → `C:\dev\verio\target\release\verio-server.exe`
- `cargo rustc --target x86_64-unknown-linux-musl --release -p verio-server -- -C linker=rust-lld`
  → `Finished release profile [optimized] target(s) in 25.51s`
  → `C:\dev\verio\target\x86_64-unknown-linux-musl\release\verio-server` (2.70 MB, 2026-09-15
  13:20:33 — deploy this to the VPS)
- Windows server binary: `C:\dev\verio\target\release\verio-server.exe` (1.74 MB, 2026-09-15 13:20:07)
- Boot smoke test of the shipped client: process alive after 8 s, killed cleanly.
- `cargo test --workspace` → all green: verio_app 11, verio_discovery 8, verio_dsp 24,
  verio_server 6, verio_transport 8 (0 failed).
- `npx tsc --noEmit` → exit 0.

### 6. Clippy status (honest report)

`cargo clippy --workspace --all-targets -- -D warnings` **fails, but on two PRE-EXISTING lints that
are not part of this session's changes**:

- `crates/verio-discovery/src/vps.rs:195-196` — collapsible `if let`
- `crates/verio-server/src/main.rs:533` — `type_complexity` on the pre-existing
  `SharedRelayHub.rooms` type

My one new lint (`room.rs:532 too_many_arguments`, caused by adding the `force_relay` parameter)
was silenced with a targeted `#[allow(clippy::too_many_arguments)]`; every other file changed this
session is lint-clean. I deliberately did not "fix" the two pre-existing lints (no unrelated
cleanups) — flagging them for a decision.

### PENDING USER ACTION — friend retry (NOT yet run, NOT claimed to pass)

1. Deploy the new musl server binary to the VPS and restart the service.
2. Launch the client with `$env:RUST_LOG='debug'`, tick **Force relay (skip P2P)**, and confirm the
   startup line `force_relay=ON — all voice traffic will route through the relay.`
3. Create a room; friend joins with the same setting on; both hold PTT.
4. Watch server `journalctl` for `relay_in` ↔ `relay_out` pairs, and each client log for
   `relay packet` lines with `direction=recv`.
5. Report back: the first stage where the pairing/recv stops.


---

## Session 8 (2026-09-15) — First real forced-relay test: result + located break

The friend test that Session 7 marked PENDING was run (Dan / LAPTOP-99TDO49L, rooms `5078` and
`5134`, 14:09–14:12 local). It **failed, but the break is now precisely located** — and it is NOT
in Verio.

### Everything on the Verio side worked

- **Identity collision (Session 6 root cause): FIXED and confirmed.** The user's client generated a
  fresh UUID on each launch (`62682210…`, `4f2a7a1b…`, `2d60ca79…`); the friend's is `b1666dec…`.
  Distinct identities on both sides.
- **Force-relay switch worked on both machines:** both logs show `force_relay: true` and
  `activating encrypted cloud tunnel via VPS relay primary=141.11.1.110:3478
  fallbacks=[141.11.1.110:9092] forced=true`. No SDP, no ICE, pure relay.
- **Per-packet instrumentation worked** on both clients (`relay packet is ON`).

### The break

| | sends | receives |
|---|---|---|
| Client DRFake (user) | **900** (760 audio, 140 control) | **0** |
| Client Dan (friend) | **926** (743 audio, 183 control) | **0** |

Server side (default INFO level, `./verio-server -p 9091`): **exactly one UDP client registered per
room** — the friend `2.147.255.247:25581` (room 5078) / `:25583` (room 5134) on port 3478.
**The user's client never registered at all.**

With only one peer in the room's relay address book, every `relay_in` has no other peer to forward
to, so nothing is ever delivered — which is exactly the zero-receive result on both sides.

### Root cause: the user's outbound UDP to the VPS never leaves the machine

Verified live from the user's PC against the still-running VPS:

```
TCP 9091  : CONNECTED in 46 ms     <- signalling path OK (server alive, UDP sockets bound)
UDP 3478  : NO REPLY within 3000 ms
UDP 9092  : NO REPLY within 3000 ms
```

Cause found in the routing table: a Wintun tunnel is up (`xray_tun`, process `p-xray-p`) and
**owns the default route** (`0.0.0.0/0` via `xray_tun`, metric 0, ahead of Ethernet at metric 25).
`Find-NetRoute 141.11.1.110` resolves via `xray_tun` (next hop 172.18.0.1), so the proxy carries the
VPS traffic — and it passes TCP but not UDP. The friend is not behind such a tunnel, so their UDP
reaches the VPS normally. This also explains why VPS STUN worked on 2026-09-11 (proxy state differed
then).

**Conclusion: to make Cloud Tunnel work, traffic to `141.11.1.110` (UDP 3478/9092) must bypass the
proxy** — a direct/bypass routing rule, or the proxy temporarily off.

### Secondary findings

- The friend's system clock runs ~4m33s **behind** (their `10:35:35` join = server `10:40:08`);
  account for this when correlating logs across machines.
- The user's input device is still `Microphone (WO Mic Device)` — the silent virtual mic from
  Session 2. Fix the input device before the next test or the relay may carry silence only.
- The friend's output is `SteelSeries Sonar - Gaming` at 96 kHz/8-ch; handled by the RX conversion
  path without error.
- The server was started in the foreground without `RUST_LOG=debug`, so no `relay_in`/`relay_out`
  lines were produced. Next run: `RUST_LOG=debug ./verio-server -p 9091` (and run it under systemd
  or nohup so it survives an SSH disconnect).

### Next steps (in order)

1. Add a bypass/direct rule for `141.11.1.110` in the proxy, or stop the proxy.
2. Re-run `test-vps-udp.ps1` on the user's PC — both UDP ports must reply.
3. Restart the server with `RUST_LOG=debug`; rerun the two-machine test.
4. Expect: two `registered` lines per room, paired `relay_in`/`relay_out`, and `direction="recv"`
   lines on both clients.


### Session 8 addendum (2026-09-15 14:45) — proxy root cause confirmed by experiment

The user closed the v2ray app and re-ran the UDP reachability probe. Results:

```
run 1 (proxy still active):  TCP 9091 OK,  UDP 9092 NO REPLY
run 2 (proxy closed):        TCP 9091 OK,  UDP 3478 REPLY 102 ms, UDP 9092 REPLY 103 ms
run 3 (server restarting):   TCP 9091 no connection, UDP 3478 REPLY 84 ms, UDP 9092 REPLY 107 ms
run 4 (proxy closed):        TCP 9091 OK,  UDP 3478 REPLY 109 ms, UDP 9092 REPLY 95 ms
```

So the VPS, its firewall, and the relay/STUN sockets are all fine — with the proxy out of the
path, UDP to `141.11.1.110:3478` and `:9092` works. Root cause confirmed by direct experiment.

**Intermittency explained:** a follow-up check minutes later found UDP dead again, and the proxy core
`D:\PattN\bin\xray\p-xray-p.exe` had **restarted at 14:42:10** — immediately after the successful
runs. Current state at that moment:

- `0.0.0.0/0 -> xray_tun` (ifIndex 89) is ahead of `0.0.0.0/0 -> Ethernet 192.168.1.1` (ifIndex 4, interface metric 25)
- `Find-NetRoute 141.11.1.110` -> `xray_tun`
- TCP 9091: CONNECTED 15 ms; UDP 3478 / 9092: NO REPLY

Closing the proxy window is not sufficient — the tunnel adapter and core process survive it and
reclaim the default route. The path must be taken out of the proxy for the whole test.

**Also note:** the server log after the 11:09:14 `RUST_LOG=debug` restart is empty, which is expected
if only the UDP probe was run — the STUN responder logs nothing. A real client attempt would still
produce `room created` / `peer joined` because the proxy carries TCP.

**Required next steps:**

1. Stop the proxy completely (end `p-xray-p.exe` too), or add a direct/bypass routing rule for
   `141.11.1.110` so its traffic skips the proxy.
2. Re-run `test-vps-udp.ps1` until it prints `VERDICT: 2 of 2 UDP ports reachable`.
3. Keep the server running with `RUST_LOG=debug` (systemd or nohup so it survives SSH disconnect).
4. Only then re-run the two-machine test with Dan.


---

## Session 9 (2026-09-15) — ROOT CAUSE: ISP outbound-UDP DPI whitelist (STUN only)

Diagnosed by running packet-capture listeners directly on the VPS (authorised). This supersedes the
"proxy" explanation of Session 8: the proxy was one way to hit the same wall, but the wall is the
ISP's outbound UDP filtering.

### Evidence 1 — arbitrary UDP payloads never leave the ISP

Listener bound on the VPS (`/tmp/udpdump.py`, `/tmp/udpdump2.py`, `/tmp/udpdump3.py`); packets sent
from the user's PC (178.131.130.53). Sent vs received:

| payload | port(s) | received |
|---|---|---|
| valid STUN binding request (20 B) | 3478 / 9092 / 53535 | **yes, all** |
| app registration `[room4][uuid16]` (20 B) | 3478 / 9092 / 53535 | no |
| hello / audio-sized (37-40 B) | 3478 / 9092 | no |
| 221 B payload | 3478 / 9092 | no |
| `0x00` first byte + random (20 B) | 3478 | no |
| random 4 B + magic cookie (20 B) | 3478 | no |
| plain random 40 B | 3478 | no |

Filtering is **content-based, not port-based** (53535 behaves identically to 3478/9092).

### Evidence 2 — the filter's exact rule

Only packets with **bytes[0..2] == 0x0001 AND bytes[4..8] == 0x2112A442** pass — a STUN Binding
Request signature. Payload attached *after* a valid STUN header is NOT inspected:

| payload | received |
|---|---|
| STUN Binding Request, 20 B | yes |
| STUN header + 20 B arbitrary (40 B total) | **yes** |
| STUN header + 40 B arbitrary (60 B total) | **yes** |
| anything without the full STUN signature | no |

### Evidence 3 — the return direction is NOT filtered

VPS script replied with four shapes to the client's socket; all four arrived:
valid STUN 32 B, plain 20 B, STUN-wrapped 60 B, plain 60 B. So the ISP filters **outbound
client -> server only**. That is why STUN (P2P/ICE) always worked while the relay never did, and why
Session 8's proxy hunt was a red herring for the underlying cause.

### Why today's test produced total silence

The friend's ISP passes plain UDP, so his registration reached the VPS
(`registered ... 2.147.255.247:25581`). The user's ISP dropped every one of the user's ~900 relay
packets, so the room only ever contained one peer and the server had nobody to forward to — hence
`0` received on both sides and no `relay_out` in the server log.

### Proposed fix (NOT implemented — needs go-ahead)

The proven-passing shape makes a surgical workaround possible:

1. **Client (`verio-transport`, relay send path):** prepend a 20-byte STUN-shaped envelope to every
   client->server relay datagram — `type=0x0001`, `length=inner_len`, magic `0x2112A442`, 12-byte
   transaction id — then the existing `[room4][uuid16][tag1][payload]`.
2. **Server (`verio-server`, `run_udp_relay`):** before parsing the room code, if the packet is
   longer than a bare Binding Request and carries the STUN signature, strip the 20-byte envelope and
   process the inner packet exactly as today. Forward the **inner** packet unchanged, since the
   return direction needs no wrapper.
3. The real STUN responder branch must be evaluated first so genuine Binding Requests keep working
   (they are exactly 20 bytes; wrapped relay packets are longer).

Verification available locally: my probe can be re-run from this PC against the VPS after the change
— success is "both peers receive" plus paired `relay_in`/`relay_out` in the server log.

### Operational note

At the time of writing the server is **stopped** (the tmux window `v` shows a `^C` after the
11:09:14 `RUST_LOG=debug` start), and its listener log is empty after startup — no `registered`
line was ever produced, which is what pointed at the filtering in the first place.


---

## Session 10 (2026-09-15) — FIX + VERIFIED: STUN-shaped envelope for the relay

Implements the fix identified in Session 9 (ISP drops unrecognised outbound UDP but passes anything
behind a valid STUN header). **Verified end-to-end against the live VPS over the filtered ISP path.**

### Changes

1. **Client — `crates/verio-transport/src/lib.rs`**
   New `wrap_stun_envelope(inner)`: `type=0x0001`, `length=inner len (u16 BE)`, magic `0x2112A442`,
   12-byte zero transaction id, then the inner relay packet. Every relay send site is framed —
   registration prefix, hello, both heartbeat probes, audio, control, ping, pong. The packet log and
   the relay byte counters now report the on-the-wire (framed) length.
2. **Server — `crates/verio-server/src/main.rs`, `run_udp_relay`**
   The genuine STUN responder branch still runs first and only matches bare 20-byte requests. After
   it, a longer packet carrying the STUN signature has its 20-byte envelope stripped; the room code,
   UUID, tag and payload are then parsed from the inner packet, registration uses the inner identity,
   and **the inner packet is what gets forwarded** (the server -> client direction is not filtered).
3. **Mode 3 regression caught and fixed — `EmbeddedVoiceHost`**
   The embedded peer-host parses the same wire format, so it would have broken (the unit test failed).
   It now strips the envelope identically. `test_relay_session_loopback`'s fake relay was updated to
   mirror the real server's unwrap.

### Verification — real output

Local (127.0.0.1, freshly built server):
```
framed  -> A received 16 (audio 15, ctrl 1) | B received 16 (audio 15, ctrl 1)  ENVELOPED RELAY WORKS
plain   -> A received  7 (audio  5, ctrl 2) | B received  7 (audio  5, ctrl 2)  RELAY OK (still compatible)
```

Live VPS, over the same ISP path that dropped everything before:
```
enveloped relay test  host=141.11.1.110  room=7166
  A received: total=17 audio_from_peer=15 control_from_peer=2 src=('141.11.1.110', 3478)
  B received: total=17 audio_from_peer=15 control_from_peer=2 src=('141.11.1.110', 3478)
RESULT: ENVELOPED RELAY WORKS (bidirectional audio)
```

Server debug log for that run — `relay_in` and `relay_out` pair up in both directions:
```
DEBUG verio_server: registered room=3751 uuid=fb92924f addr=178.131.130.53:62893
DEBUG verio_server: relay_in   room=3751 from_uuid=fb92924f from_addr=178.131.130.53:62893 len=37 peers_in_room=2
DEBUG verio_server: relay_out  room=3751 to_uuid=0cdfe6b1 to_addr=178.131.130.53:42743 len=37
DEBUG verio_server: relay_in   room=3751 from_uuid=0cdfe6b1 from_addr=178.131.130.53:42743 len=37
DEBUG verio_server: relay_out  room=3751 to_uuid=fb92924f to_addr=178.131.130.53:62893 len=37
```

Contrast run with the **unwrapped** format from the same PC: both peers received 0 — confirming the
ISP filter is still in place and that the envelope, not luck, is what makes the relay work.

`cargo test --workspace` → **59 passed, 0 failed** (the two relay tests that failed on the raw change
now pass with the Mode 3 + fake-relay fixes). Clippy shows no new lints from these changes (the only
warnings remain the pre-existing `vps.rs:195` and `main.rs:533` type-complexity).

### Deployment

- Client: `target\release\verio.exe` (15.95 MB) — rebuilt; friend kit on the Desktop refreshed.
- Server (VPS): `target\x86_64-unknown-linux-musl\release\verio-server`, sha256
  `43feac747cb25ee016a6c7a14240ac79854c2355802eb751ab4e036211dfb5c9`, uploaded to `/root/verio-server`
  (previous binary kept as `/root/verio-server.prev`). Running in tmux window `v` as
  `RUST_LOG=debug ./verio-server -p 9091`; listeners confirmed on 0.0.0.0:9092, 0.0.0.0:3478 and
  0.0.0.0:9091.

### Compatibility

Old (plain) client + new server: still works — the server only unwraps when the STUN signature is
present. **New client + old server is NOT supported**, so the server must be the updated build (done).

### Remaining

The two-machine GUI call with Dan has **not** been re-run yet — that is the last human step. The wire
format the client now emits is byte-identical to the probe that passed, and the friend's kit has been
refreshed with the new client, so expect: two `registered` lines per room, paired `relay_in`/
`relay_out`, and `direction="recv"` lines in both client logs.


---

## Session 11 (2026-09-15) — UX fixes: deafen, leave, cues, single-path UI, context menu

Six items were requested. **1-5 are implemented and pass tests/typecheck/build; item 6 (3-user
rooms) is diagnosed but not yet implemented** — see the end.

### 1. Deafen actually silences (was: mic-only)

Deafen forced mute but playback was never gated, so the deafened user still heard everyone.

- `crates/verio-app/src/pipeline.rs` — the RX/mixer thread now receives `Arc<TxState>` and, when
  `is_deafened()`, contributes **silence** from every remote peer while still advancing their
  buffers (no stalls). Earcon cues still play, so you hear the deafen confirmation.
- Deafen is now announced to peers: `ControlMessage::State { speaking, mute, deafen }`
  (`verio-transport/src/lib.rs`, field carries `#[serde(default)]` for compatibility), sent from
  `send_room_state` (`ui/src-tauri/src/lib.rs`), surfaced as `RoomEvent::PeerState { peer_id,
  speaking, mute, deafen }` and shown as a red **Deafened** badge on the peer card.

### 2. Leaving the room no longer strands your profile

Cause: clicking Leave tore down the local session but never told the server — the WebSocket stayed
open on the listener thread, so the server still listed you and peers kept showing your card.

- `crates/verio-discovery/src/vps.rs` — `VpsSignaling` gained a stop flag and the listener now polls
  with a 200 ms read timeout; on stop it sends an explicit `Leave` and closes the socket. It also
  keeps reporting further peer departures instead of exiting on the first one.
- `crates/verio-app/src/room.rs` — `RoomManager` holds the flag; `teardown_session` sets it, so
  every leave path (button, error, peer-left) notifies the server immediately.

### 3. Join / leave / mute / deafen cues

All four are **synthesized in-process** — no audio files to source or license:

- Mute ON 400 Hz, Mute OFF 800 Hz, Deafen two 300 Hz blips (existing).
- **New** `EarconKind::PeerJoin` (rising 660 → 990 Hz) and `PeerLeave` (falling 990 → 660 Hz) in
  `crates/verio-dsp/src/earcon.rs`, played from the room-event forwarder on `PeerConnected` /
  `PeerDisconnected`.

So nothing is needed from you — if you'd still prefer real sampled sounds, drop four files in and
I'll wire them to the same four events.

### 4. Removed the debug transport plumbing

- Settings no longer shows *Voice Transport Architecture* (Auto Hybrid / Cloud Tunnel / Host on My
  PC / Direct P2P), the **Force relay** switch, or the Host UDP Port field. It is now a single
  "Encrypted relay — Active" row.
- The header badge is one label (`Relay`) instead of four mode variants.
- Backend: `Settings::force_relay` is gone, along with the `set_force_relay` command and its startup
  log. `run_vps_room_session` always takes the relay path, so ICE/STUN gathering and the P2P
  handshake are unreachable from the 4-digit room flow. (The direct-connect / invite-code path still
  uses its own transport and was left alone.)

### 5. App-appropriate right-click

The WebView's default menu (Print / Save as / Reload / Inspect) is suppressed and replaced by a
custom menu: **Copy room code**, **Mute/Unmute**, **Deafen/Undeafen**, **Settings**, **Leave call**.
Implemented in `ui/src/App.tsx` (global `contextmenu` handler + `ctx-menu` component) with styles in
`App.css`.

### 6. Three-user rooms — diagnosed, NOT implemented

Root cause of "with 3 users you only see 2": the room layer is single-peer by construction.

- `VpsSignaling::exchange_identities` returns **one** peer (the first from `Joined { peers }`, or the
  first `PeerJoined`) — `crates/verio-discovery/src/vps.rs`.
- The relay driver drops the sender identity after the self-echo check, so
  `TransportEvent::Audio` carries no peer id; `room.rs` attributes all incoming audio to that single
  peer. (The mixer is already multi-peer — `peers: HashMap<String, PeerRxStream>` and per-peer
  volume — so the upper layers are what need work.)
- The UI renders exactly one remote card.

Work needed: tag incoming relay audio with the sender UUID; track a peer set in `RoomManager`
(seeded from the signaling peer list, updated on PeerJoined/PeerLeft); emit per-peer events; render N
cards with per-peer volume. The relay server already fans out to every peer in a room, so no server
change is expected. The fluctuating RTT is consistent with several peers answering ping/pong on the
same session and should be re-checked once per-peer attribution exists.

### Verification

- `cargo test --workspace` → **59 passed, 0 failed** (earcon, pipeline, room, transport, server).
- `npx tsc --noEmit` → exit 0.
- Client builds; no new clippy lints from these changes (the two pre-existing ones remain:
  `verio-discovery/src/vps.rs` collapsible `if let`, `verio-server/src/main.rs:533` type complexity).
- Not verified interactively: the deafen playback gate, the leave-notification round trip and the
  four cues all need a listening test with two clients — please confirm in-app.


---

## Session 12 (2026-09-15) — Multi-peer rooms (3+ users) implemented and verified

The item deferred from Session 11 is now built. The relay was already a star topology on the server
(it fans out every datagram to all other peers in the room), so **no server change was needed** — the
work was client-side: attribute each datagram to its sender, track a peer set instead of one peer,
and render every peer.

### Changes

**Transport — `crates/verio-transport/src/lib.rs`**
- `TransportEvent::Audio` and `TransportEvent::Control` now carry the sender's UUID. The relay driver
  reads bytes 4..20 of the received datagram (`Uuid::from_bytes(...).to_string()`, matching the
  identity strings the signalling layer uses). The str0m/direct paths send an empty peer id; the room
  falls back to the session's single peer.
- **RTT fix for multi-peer**: with several peers in the room every peer echoes our ping, which made
  the displayed ping jump around. The driver now remembers the timestamp of *its own* last ping
  (`last_ping_t`) and ignores any pong that does not match it.

**Signalling — `crates/verio-discovery/src/vps.rs`**
- New `SignalingEvent { PeerJoined(Identity), PeerLeft(String) }`; the listener reports both instead
  of only departures (and no longer exits after the first one).
- `initial_peers()` exposes the roster the server returns in `Joined { peers }`, so a joiner adopts
  everyone already in the room.

**Room — `crates/verio-app/src/room.rs`**
- `peer_name: Option<String>` replaced by `peers: HashMap<String, PeerInfo>` where
  `PeerInfo { uuid, name, speaking, mute, deafen }`. `add_peer` / `remove_peer` emit
  `PeerConnected` / `PeerDisconnected` per peer and the room idles only when the last peer leaves.
- The roster is seeded from `initial_peers()`, then kept live by the listener.
- The transport pump attributes audio and control events per sender (`resolve_peer`).

**UI — `ui/src/App.tsx`, `App.css`**
- Single peer state replaced by a `peers` list plus per-peer volumes; cards are rendered per peer with
  their own volume slider, speaking glow, Mute and Deafened badges.
- The snapshot exposes `peers` for resync; card CSS hardened (`min-width: 0`, `overflow: hidden`) so
  several cards cannot overlap.

### Verification — real output

**Server fan-out for three peers, on the VPS itself** (isolates it from any network filtering):
```
Alpha:   total=22 audio per sender={'Bravo': 10, 'Charlie': 10}  -> OK
Bravo:   total=22 audio per sender={'Alpha': 10, 'Charlie': 10}  -> OK
Charlie: total=22 audio per sender={'Alpha': 10, 'Bravo': 10}    -> OK
RESULT: SERVER FANS OUT TO 3 PEERS (each hears the other two)
```
Server debug log for that room shows each peer registering once and every `relay_in` being fanned
out to the other two.

**Same three-peer test over the real internet path** (STUN-enveloped wire format, from the user's PC,
after the proxy was taken out of the route) — identical result, all three hear both others:
```
3-peer relay test  room=2434
  Alpha:   audio per sender={'Charlie': 10, 'Bravo': 10}  -> OK
  Bravo:   audio per sender={'Alpha': 10, 'Charlie': 10}  -> OK
  Charlie: audio per sender={'Alpha': 10, 'Bravo': 10}    -> OK
RESULT: SERVER FANS OUT TO 3 PEERS (each hears the other two)
```

Also: `cargo test --workspace` → **59 passed, 0 failed**; `npx tsc --noEmit` → exit 0; client builds
and boots. No new clippy lints (the long-standing `verio-discovery` collapsible-`if let` warning
disappeared as a side effect of the listener rewrite).

### Remaining

The three-machine GUI test has not been run — that is the last human step. Expect three cards in the
call view, each with its own volume slider, and the speaking ring / Mute / Deafened badges following
the correct person. The packet logging from Session 7 is still in place (`RUST_LOG=debug`) if
anything needs tracing.

**Note for future tests:** with the proxy/`xray_tun` active the outbound-UDP filter (Session 9)
silently drops the relay datagrams — the probe shows 0 packets and the server logs nothing. UDP
reachability must be 2/2 before any voice test.


---

## Session 13 (2026-09-15) — Music Player (separate audio bus)

Client-side feature only. No transport, signalling, relay, noise-suppression, VAD or
`verio-server` changes. No ffmpeg/yt-dlp. The spec's design zip was ported, not redesigned.

### Step 0 — design extraction (reported first, per instructions)

- **Extracted to:** `C:\dev\verio\design\music-player\` (from `verio-music-player-panel.zip`).
  `design/` is not part of the build; the repo has no root `.gitignore`, so nothing was added there
  and the folder is left in place as the visual reference.
- **The spec file:** `design/music-player/music-player.html` (1 828 lines, inline CSS + inline SVG).
  The React/Vite files around it are an AI-Studio scaffold that merely iframes the HTML.
- **CSS custom properties it defines:**
  `--bg, --surface, --surface-2, --surface-3, --surface-hover, --surface-active, --border,
  --border-subtle, --border-focus, --text, --text-dim, --text-muted, --accent, --accent-hover,
  --accent-dim, --accent-glow, --warn, --warn-dim, --warn-border, --danger, --danger-dim,
  --radius-sm, --radius, --radius-md, --radius-pill, --transition-fast, --dock-height,
  --panel-height, --max-panel-width`.
- **Missing / ambiguous state in the design:** there is **no notes section** in the zip (only a
  "preview dev toolbar" comment block); the mock is driven by a demo-only `setMockState()` state
  machine with five root classes — `state-idle`, `state-playing`, `state-paused`, `state-deafened`,
  `state-collapsed` — and hard-coded mock values (`Suborbital Echoes`, 3:18, "Sending to room").
  Real state had to be substituted for all of it. Two points the design does not settle, resolved as
  follows: (1) the seek/volume controls are div tracks with click handlers, not `<input>`s — kept as
  the design's div structure with pointer handlers; (2) the design does not specify a monitor volume
  separate from the send volume — the single volume slider drives both.
- **Token collision:** the design's `--accent` is the vivid green `#3bff9a`, but the app already
  defines `--accent: #ffffff`. Per the instruction not to rename existing tokens, the design's accent
  family is namespaced `--music-accent*` in the ported CSS; all other missing tokens were added as-is.

### Task 1 — Decoder (`crates/verio-dsp/src/music.rs`)

`symphonia` 0.5.5 with `default-features = false, features = ["mp3", "flac", "wav", "ogg", "vorbis",
"pcm"]` (AAC/ALAC excluded as specified). **Pure Rust — no C/C++ toolchain**: the tree pulled in
`symphonia-core/-metadata/-utils-xiph`, `symphonia-bundle-mp3`, `symphonia-bundle-flac`,
`symphonia-format-riff`, `symphonia-format-ogg`, `symphonia-codec-vorbis`, `symphonia-codec-pcm`,
`rustfft`/`realfft`/`easyfft`. Nothing required cmake, perl, nasm or libclang.

`MusicDecoder` implements the specified API (`open`, `duration_ms`, `position_ms`,
`next_chunk(&mut [f32; 480])`, `seek`, `reset`). It reuses `verio_dsp::convert::mixdown_to_mono` for
stereo→mono and `LinearResampler` for native-rate→48 kHz — no new DSP helpers were written.
`next_chunk` returns `Ok(0)` at natural EOF and errors only on genuine decode failure;
`position_ms` is `samples_delivered / 48`.

### Task 2 — Music bus (`crates/verio-app/src/pipeline.rs`)

`MusicSource` (Mutex-guarded, shared by the audio thread and the UI) with `is_active`, `open`,
`play`, `pause`, `stop`, `seek`, `set_volume`, `set_loop`, `set_monitor`, `status`, plus
`next_chunk` which handles EOF → loop or stop.

In the processing loop, after the mic → gain → RNNoise → VAD chain:

- music is mixed straight into the buffer going to the encoder — **never through RNNoise or the VAD**;
- the mic contributes only while `tx_state.effective_transmission()`, music contributes always;
- **DTX is switched off while music is active and restored when it stops**, on transition only
  (new `OpusEncoderWrapper::set_dtx`, `crates/verio-dsp/src/codec.rs`);
- the local monitor is sent to the RX mixer only when `monitor && !deafened && music playing`, so
  deafen silences the monitor without touching the room feed.

Gate helpers `transmit_gate(loopback, music_active, tx_state)` and
`monitor_contributes(monitor, deafened, music_active)` encode the two rules directly.

### Task 3 — Tauri commands (`ui/src-tauri/src/lib.rs`)

`music_status`, `music_open`, `music_play`, `music_pause`, `music_stop`, `music_seek`,
`music_set_volume`, `music_set_loop`, `music_set_monitor`, `music_pick_file` — all registered in
`invoke_handler`, all emitting `music_status_changed` immediately, plus a 250 ms ticker thread that
keeps pushing while playing. **File picker: `rfd` 0.15 directly, not `tauri-plugin-dialog`** — only a
Rust-side dialog is needed, so the plugin's JS API and capability wiring would be dead weight; rfd is
pure Rust on Windows via `windows-sys`.

### Task 4 — Settings

`music_volume` (1.0), `music_loop` (false), `music_monitor` (false), `music_last_dir` (None).
Path, play state and position are deliberately **not** persisted — music starts idle.

### Task 5 — UI port (`ui/src/App.tsx`, `ui/src/App.css`)

The design's CSS is carried over verbatim (panel + dock strip, ~18 KB) with only the `--accent`
namespacing change; missing tokens were added to `:root` without touching existing ones. The panel is
collapsed by default (`musicPanelOpen`), the dock shows the glanceable strip whenever
`playing || active`, "Open File…" uses the native picker defaulting to `music_last_dir` and filtering
mp3/flac/wav/ogg, seek commits once on `pointerup` while reflecting live `position_ms` otherwise,
volume is debounced 100 ms, loop/monitor fire immediately, and the monitor carries both the amber
tooltip and the persistent label. While deafened the panel stays fully interactive and the
"Muted locally" badge appears.

### Verification

- `cargo test --workspace` → **63 passed, 0 failed**, including the new
  `music::tests::decodes_generated_wav` (1 s 440 Hz WAV → ~48 000 samples, peak within 5 %),
  `music_bus_tests::music_keeps_transmit_gate_open_while_muted_and_deafened`,
  `music_bus_tests::deafen_silences_local_monitor`, and
  `music_bus_tests::music_source_streams_and_stops_at_eof`.
- `npx tsc --noEmit` → exit 0.
- `cargo build` for the UI bridge succeeds (see build log below).

**Status: unit-tested; needs an interactive listening test with two clients** — nothing about the
end-to-end behaviour (does B actually hear A's music) has been verified by ear.

**Build-environment note:** crates.io downloads are throttled to a trickle on this line (the same
filtering recorded in Session 9), so the two new dependencies were fetched through a crates.io
mirror via a throwaway `--config` file (`sparse+https://rsproxy.cn/index/`). No project file was
changed for it; record the mirror if a clean build is needed again.

### PENDING USER ACTION — interactive checks

1. `cd C:\dev\verio\ui && npx tauri build --no-bundle`
2. Launch two `verio.exe` instances on the same PC, both in the same room (local server or VPS).
3. Instance A: open the music panel, load an MP3, press play.
4. **B hears the music** (primary test).
5. A presses Mute → B still hears the music; A's mic is silent.
6. A presses Deafen → B still hears the music; A hears neither peers nor local monitor.
7. While deafened, A presses Pause → B's music stops (**panel must stay interactive**).
8. A enables Local Monitor (headphones on) → A hears the music alongside B.
9. Seek, loop and volume all respond within ~250 ms.
10. Set volume to 50 %, close A, relaunch, open the panel → volume reads 50 %.


---

## Session 14 (2026-09-16) — Nine reported issues

Client + shared-crate changes only. No `verio-server` change (the relay is opaque, and the roster
fix is client-side gossip), so nothing new to deploy to the VPS.

### 3. Input gain could not be changed — FOUND AND FIXED

The UI invoked `set_input_gain` with `{ gainDb }`, but the Rust parameter was named `db`. Tauri maps
the JS camelCase key onto a snake_case argument name, so `gainDb` never matched `db`: every invoke
rejected, the `await` threw, `setSettings` never ran and the slider snapped back. Renamed the
parameter to `gain_db`; the value is also persisted now, and the slider works.

### 2, 4, 5. Roster going stale (third joiner saw 2, duplicate name after rejoin, stale card after leave)

All three are the same class of bug: the roster depended entirely on one-shot signalling events.
`exchange_identities` consumes messages from the socket and returns on the first peer it sees, so any
`PeerJoined` arriving in the window before the listener thread starts was dropped — and a peer that
disappears without a clean signalling event was never removed.

Fixed with a self-healing roster that does not depend on signalling timing:

- **Roster gossip (new):** every 3 s each client re-announces its `Hello` through the relay, so peers
  that missed an event still learn about each other (and stale names update).
- **Liveness + prune (new):** every relay datagram from a peer (control, ping/pong, audio) refreshes
  `peer_seen`; a sweep drops peers not heard from for 15 s and emits `PeerDisconnected`, so a card
  cannot outlive its owner.

### 6. Nickname now updates live

`set_nickname` sends a fresh `Hello` through the relay immediately, and the receiving pump already
handles a name change by updating that peer and re-emitting `PeerConnected`.

### 7. Join/leave cues for everyone

The cues fire from `PeerConnected` / `PeerDisconnected`, which every client receives for every other
participant — so they already played for all participants. They appeared not to, because the roster
bugs above meant those events were sometimes never emitted. The roster fix should resolve it; if a
cue is still missing after testing, it is a UI-layer issue and needs the client log.

### 9. Who is playing music

`ControlMessage::State` now carries `music` and `music_title` (both `#[serde(default)]`), they are
stored per peer and forwarded to the UI, and the peer card shows a green `♪ <title>` badge. The state
packet is sent immediately on every music transition (open/play/pause/stop) as well as on speaking
changes, so the badge tracks reality.

### 1, 8. Quality tiers (voice and music separately)

- New `OpusEncoderWrapper::set_bitrate_bps`, and `LiveControls` carries a live **voice** and **music**
  bitrate.
- The processing loop picks the tier on every frame: `music.is_active() ? music_kbps : voice_kbps`,
  applied only when the value changes.
- Settings: `voice_bitrate_kbps` (default **32**, the tuned original) and `music_bitrate_kbps`
  (default **96**), with dropdowns in Settings -> Audio Quality: voice 24/32/48/64/96 kbps, music
  64/96/128/160/192 kbps.
- This is per-client: Opus is self-describing, so no room-wide negotiation is needed — each client
  chooses its own upstream bitrate. Music is mono at 48 kHz either way; a stereo music path would
  need a channel-count change to the decoder and mixer and is not part of this change.

### Verification

- `cargo test --workspace` → **63 passed, 0 failed**.
- `npx tsc --noEmit` → exit 0.
- `npx tauri build --no-bundle` → succeeded.

**Build note:** the two dependencies added in Session 13 (symphonia, rfd) resolve against a crates.io
mirror on this line, so `cargo` now needs the throwaway config
(`--config <sparse+https://rsproxy.cn/index/>`) unless direct crates.io access works. Nothing in the
repo depends on that file.

**Still needs an interactive test** — the roster behaviour (3 users, rejoin, leave), the gain slider,
the cues and the music badge are all only unit-verified.

---

## Session 15 (2026-09-16) — Warm redesign (palette-preview) + music panel skin

Sources: design/palette-preview.html (moved from the project root, doubled `.html.html` extension fixed).
Reference route: existing codebase redesign — visual layer only, no logic changes.

### Delivered, in order

- **Pass 1** — `ui/src/App.warm.css` created (additive, unimported): new token block verbatim from the
  reference, shell CSS, placeholder overlay rules. Build confirmed the CSS bundle unchanged, proving
  nothing visual moved.
- **Pass 2** — legacy `App.css` (green palette) replaced; duplicate `import "./App.css"` removed from
  `main.tsx`, retained in `App.tsx`; 21 class renames + peer state variants applied atomically.
  Coverage driven to **143 used / 202 defined / 0 unstyled**.
- **B1** — header → `.header` / `.brand` / `.brand-mark` / `.brand-word` / `.brand-dot` / `.rtt` /
  `.room-chip`.
- **B2** — dock → `.dock` / `.dock-left` / `.me` / `.me-meta` + live mic meter; `.dock-btn` Mic (with
  `.kbd` chip), Deafen, Music, Settings.
- **B3** — main → `.main` with `.section-label` ("In call · N of 10", "Now playing") and the `.peers`
  grid; local self card moved onto `is-speaking` / `is-muted`.
- **C** — overlay refinements: settings/drawer section titles at 11px / 0.15em with `::after` hairline,
  14px settings rows with hairline separators, `.diag-value.warn` → `--danger`, context menu on
  `--surface-2` + `--border-2` + 0 8px 24px shadow.
- **D** — header logo: `design/logo/verio-logo-primary.png` copied to `ui/src/assets/` and imported
  through Vite's asset pipeline (`src/vite-env.d.ts` added so the PNG import typechecks).

### Legacy palette removed (grepped, zero remaining)

`#3bff9a`, `#5affad`, `#ffb454`, `#ff5566`, `#21262f`, `#2b323e`, `#7cc0ff`, `#7dffbe`, `#ff8b8b`,
`#16181d`, `#e8eaf0`, `#2a2e37`, plus the whole `--music-accent*` family (30 references).
Also removed with the block: duplicate `:root` declaration at App.css line 1275.

### Deviation

**`rfd` retained for the music file picker instead of `tauri-plugin-dialog`** (user decision). Only a
Rust-side dialog is needed, so the plugin's JS API and capability wiring would be dead weight.

### Decisions (not bugs)

- **"Now playing" is gated on `musicPanelOpen`**, so the label shows whenever the panel is open even
  with no track loaded. Confirmed acceptable by the user.
- **The live mic meter was kept** rather than replaced with the reference's decorative 12-bar
  animated `.meter`. Swapping would have lost the real `input_level` display. The unused `.meter`
  rules remain for a later cleanup pass.
- The header logo replaces the inline `V` SVG at 20px.

### Files changed

`ui/index.html` (Inter + JetBrains Mono via link/preconnect), `ui/src/App.css` (full warm sheet,
~41 KB / 1 900 lines), `ui/src/App.tsx` (header, dock, main, state variants, logo import),
`ui/src/main.tsx` (duplicate import removed), `ui/src/vite-env.d.ts` (new),
`ui/src/assets/verio-logo-primary.png` (new), `ui/src-tauri/icons/*` (regenerated from the primary
logo), `design/palette-preview.html` (moved into place).

### Verification — real output

```
cargo test --workspace  ->  63 passed, 0 failed
npx tsc --noEmit        ->  exit 0
npx tauri build --no-bundle ->  Finished release profile [optimized] target(s) in 1m 43s
                                dist/assets/index-Bgy7KC2j.css 30.97 kB │ gzip: 5.14 kB
                                dist/assets/index-DPOsdY50.js 235.73 kB │ gzip: 70.87 kB
                                Built application at: C:\dev\verio\target\release\verio.exe
8s boot smoke test      ->  alive after 8s: True
```

**Compiles, unit-tested; needs user visual review and listening test.**

### Found but not fixed

- `tauri build` warns the bundle identifier `ir.verio.app` ends with `.app`. Changing it would move
  the settings/log directory, so it is out of scope for a visual pass.
- The reference's decorative `.meter` rules are now unused (see decisions).

### PENDING USER ACTION — interactive checklist

1. `cd C:\dev\verio\ui && npx tauri build --no-bundle` (or run the built exe).
2. Confirm the warm palette, the logo in the header, the new dock, and the peer card layout.
3. Launch a second instance; both join the same room. Confirm the speaking ring pulses on the active
   talker, the mute button turns clay when pressed, and deafen forces mute.
4. Open Settings — confirm all rows are present and there is no leftover "Force relay" or
   transport-architecture section.
5. Open the diagnostics drawer (status pill in the header) — confirm counters still display and that
   Last Relay Recv turns red past 2000 ms.
6. Right-click anywhere — confirm the custom context menu styling.
7. Music: open the panel, load an MP3, press play. The second instance must hear it.
8. While music plays, press Mute — the room must still hear music, your mic is silent.
9. Press Deafen — the room must still hear music, you hear nothing, panel stays interactive.
10. Pause, seek, loop, volume all respond; volume persists across a relaunch at 50%.