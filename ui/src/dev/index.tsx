/**
 * Dev mock harness entry point.
 *
 * ---------------------------------------------------------------------------
 * SAFETY MODEL — read before changing anything here
 * ---------------------------------------------------------------------------
 * The harness replaces Tauri's IPC transport with fixtures so the redesigned UI
 * can be rendered and inspected without a running Rust backend. It must be
 * impossible for that to happen in the shipped application. Four independent
 * guards, all of which must pass:
 *
 *   1. `import.meta.env.DEV` — Vite statically replaces this with `false` in a
 *      production build, so the `if` body and the dynamic import are removed by
 *      dead-code elimination. The harness and its fixtures are therefore absent
 *      from `dist/` entirely. This is verified by grepping the built bundle.
 *
 *   2. A real Tauri runtime must NOT be present. Inside `pnpm tauri dev` or the
 *      packaged app, `window.__TAURI_INTERNALS__.invoke` exists, and the harness
 *      refuses to install. The real IPC path can never be shadowed.
 *
 *   3. Explicit opt-in via the `?mock=1` URL flag. Absence of the flag means no
 *      install.
 *
 *   4. A sticky kill switch. `?mock=0` (or the panel's kill button) sets a
 *      session flag that blocks installation even if `?mock=1` is present, so
 *      the harness cannot be re-enabled by accident. `?mock=reset` clears it.
 *
 * `main.tsx` imports this module dynamically inside a `import.meta.env.DEV`
 * branch, so guard 1 is enforced at build time as well as at runtime.
 */

import { createRoot } from "react-dom/client";
import { MockHarness } from "./MockHarness";
import { DEFAULT_SCENARIO_ID } from "./mock/fixtures";
import { installMockTransport } from "./mock/invoke";

const KILL_KEY = "verio:mock:kill";

interface TauriGlobals {
  __TAURI_INTERNALS__?: { invoke?: unknown };
}

/** True when running inside a genuine Tauri webview (dev or packaged). */
function isRealTauriRuntime(): boolean {
  const w = window as unknown as TauriGlobals;
  return typeof w.__TAURI_INTERNALS__?.invoke === "function";
}

function readKillFlag(): boolean {
  try {
    return window.sessionStorage.getItem(KILL_KEY) === "1";
  } catch {
    return false;
  }
}

function writeKillFlag(on: boolean): void {
  try {
    if (on) window.sessionStorage.setItem(KILL_KEY, "1");
    else window.sessionStorage.removeItem(KILL_KEY);
  } catch {
    /* non-fatal: the URL flag still governs this load */
  }
}

function mountPanel(transport: ReturnType<typeof installMockTransport>): void {
  const host = document.createElement("div");
  host.id = "verio-mock-harness";
  document.body.appendChild(host);
  createRoot(host).render(<MockHarness transport={transport} />);
}

/**
 * Install the mock transport if, and only if, every guard passes.
 * Resolves either way; never throws into application startup.
 */
export function installMockHarness(): void {
  // Guard 1 — production builds never reach here (statically eliminated).
  if (!import.meta.env.DEV) return;

  // Guard 2 — never shadow a real backend.
  if (isRealTauriRuntime()) {
    // eslint-disable-next-line no-console
    console.info(
      "[verio mock] real Tauri runtime detected — mock harness not installed.",
    );
    return;
  }

  const params = new URLSearchParams(window.location.search);
  const flag = params.get("mock");

  if (flag === "reset") {
    writeKillFlag(false);
    // eslint-disable-next-line no-console
    console.info("[verio mock] kill switch cleared. Reload with ?mock=1 to install.");
    return;
  }

  if (flag === "0") {
    writeKillFlag(true);
    // eslint-disable-next-line no-console
    console.info("[verio mock] kill switch set. ?mock=reset clears it.");
    return;
  }

  // Guard 3 — explicit opt-in.
  if (flag !== "1") return;

  // Guard 4 — sticky kill switch.
  if (readKillFlag()) {
    // eslint-disable-next-line no-console
    console.warn(
      "[verio mock] blocked by the kill switch. Run with ?mock=reset to clear it.",
    );
    return;
  }

  const scenarioId = params.get("scenario") ?? DEFAULT_SCENARIO_ID;
  const transport = installMockTransport(scenarioId);

  // eslint-disable-next-line no-console
  console.info(
    `[verio mock] installed. scenario=${transport.scenario.id}. This is MOCK data — no Rust backend is running.`,
  );

  // `?panel=0` keeps the panel out of the way for screenshots.
  if (params.get("panel") !== "0") mountPanel(transport);
}
