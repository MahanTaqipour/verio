/**
 * The harness control panel.
 *
 * Deliberately styled with inline styles rather than Tailwind classes:
 *   - it must render identically before and after the Tailwind migration, so it
 *     can be used to compare the two;
 *   - it is development tooling, not part of the Verio design system, and must
 *     never be mistaken for product UI;
 *   - it mounts into its own root outside #root, so it cannot affect the
 *     application's layout.
 */

import { useEffect, useReducer, useState, type CSSProperties } from "react";
import { SCENARIOS } from "./mock/fixtures";
import type { MockTransport } from "./mock/invoke";

const KILL_KEY = "verio:mock:kill";

const C = {
  panel: "#1b1b1f",
  border: "#3a3a42",
  text: "#e6e6ea",
  dim: "#9a9aa6",
  accent: "#c9873f",
  danger: "#b8544a",
  well: "#111114",
};

function navigate(params: Record<string, string | null>): void {
  const url = new URL(window.location.href);
  for (const [k, v] of Object.entries(params)) {
    if (v === null) url.searchParams.delete(k);
    else url.searchParams.set(k, v);
  }
  window.location.replace(url.toString());
}

function killHarness(): void {
  try {
    window.sessionStorage.setItem(KILL_KEY, "1");
  } catch {
    /* sessionStorage unavailable; the URL flag below is the real switch */
  }
  navigate({ mock: null, scenario: null, panel: null });
}

export interface MockHarnessProps {
  transport: MockTransport;
}

export function MockHarness({ transport }: MockHarnessProps) {
  const [open, setOpen] = useState(true);
  const [, force] = useReducer((n: number) => n + 1, 0);

  // Refresh the command log / counters a few times a second.
  useEffect(() => {
    const id = window.setInterval(force, 400);
    return () => window.clearInterval(id);
  }, []);

  const active = transport.scenario.id;

  const btn: CSSProperties = {
    background: C.well,
    color: C.text,
    border: `1px solid ${C.border}`,
    borderRadius: 4,
    padding: "3px 7px",
    font: "11px/1.3 ui-monospace, SFMono-Regular, Menlo, monospace",
    cursor: "pointer",
  };

  if (!open) {
    return (
      <button
        type="button"
        onClick={() => setOpen(true)}
        style={{
          ...btn,
          position: "fixed",
          right: 10,
          bottom: 10,
          zIndex: 2147483000,
          background: C.accent,
          color: "#171412",
          fontWeight: 700,
        }}
      >
        MOCK
      </button>
    );
  }

  return (
    <div
      style={{
        position: "fixed",
        right: 10,
        bottom: 10,
        width: 320,
        maxHeight: "82vh",
        overflowY: "auto",
        zIndex: 2147483000,
        background: C.panel,
        color: C.text,
        border: `1px solid ${C.border}`,
        borderRadius: 6,
        padding: 10,
        font: "11px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace",
        boxShadow: "0 8px 24px rgba(0,0,0,.5)",
      }}
    >
      <div
        style={{
          display: "flex",
          alignItems: "center",
          justifyContent: "space-between",
          marginBottom: 8,
        }}
      >
        <strong style={{ color: C.accent, letterSpacing: ".08em" }}>
          VERIO MOCK HARNESS
        </strong>
        <div style={{ display: "flex", gap: 4 }}>
          <button type="button" style={btn} onClick={() => setOpen(false)}>
            hide
          </button>
          <button
            type="button"
            style={{ ...btn, borderColor: C.danger, color: C.danger }}
            onClick={killHarness}
            title="Disable the harness for this browser session"
          >
            kill
          </button>
        </div>
      </div>

      <div style={{ color: C.dim, marginBottom: 6 }}>
        {transport.commandCount} commands handled · panel-only UI
      </div>

      <label style={{ display: "block", marginBottom: 8 }}>
        <span style={{ color: C.dim }}>scenario</span>
        <select
          value={active}
          onChange={(e) => navigate({ mock: "1", scenario: e.target.value })}
          style={{
            width: "100%",
            marginTop: 3,
            background: C.well,
            color: C.text,
            border: `1px solid ${C.border}`,
            borderRadius: 4,
            padding: "4px 6px",
            font: "inherit",
          }}
        >
          {SCENARIOS.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label}
            </option>
          ))}
        </select>
      </label>

      <div
        style={{
          background: C.well,
          border: `1px solid ${C.border}`,
          borderRadius: 4,
          padding: "5px 7px",
          color: C.dim,
          marginBottom: 8,
        }}
      >
        {transport.scenario.note}
      </div>

      <div style={{ color: C.dim, marginBottom: 4 }}>live transitions</div>
      <div
        style={{
          display: "grid",
          gridTemplateColumns: "1fr 1fr",
          gap: 4,
          marginBottom: 8,
        }}
      >
        <button type="button" style={btn} onClick={() => transport.addRandomPeer()}>
          + peer
        </button>
        <button type="button" style={btn} onClick={() => transport.removeLastPeer()}>
          − peer
        </button>
        <button
          type="button"
          style={btn}
          onClick={() => transport.toggleRandomPeerSpeaking()}
        >
          toggle speak
        </button>
        <button type="button" style={btn} onClick={() => transport.pulseMicLevel()}>
          pulse mic
        </button>
        <button
          type="button"
          style={btn}
          onClick={() => transport.setMusicPlaying(true)}
        >
          music play
        </button>
        <button
          type="button"
          style={btn}
          onClick={() => transport.setMusicPlaying(false)}
        >
          music pause
        </button>
        {/* Session 29b. Labelled statically on purpose: reading `state.bot` here
            would give a live value that never re-renders, so the label would go
            stale the moment the button was used — the same trap the mic button
            above falls into. */}
        <button
          type="button"
          style={btn}
          onClick={() => transport.toggleMusicBot()}
        >
          toggle bot
        </button>
        <button
          type="button"
          style={btn}
          onClick={() => transport.setMicLevel(!transport.scenario.behavior.micLevel.enabled)}
        >
          mic {transport.scenario.behavior.micLevel.enabled ? "off" : "on"}
        </button>
        <button
          type="button"
          style={btn}
          onClick={() =>
            transport.emitRoomError("ice connection lost (grace period expired)")
          }
        >
          room error
        </button>
        <button
          type="button"
          style={{ ...btn, gridColumn: "1 / -1" }}
          onClick={() =>
            transport.emitAudioError(
              "spawn audio pump failed: no default output device",
            )
          }
        >
          audio error
        </button>
      </div>

      <div style={{ color: C.dim, marginBottom: 4 }}>recent commands</div>
      <div
        style={{
          background: C.well,
          border: `1px solid ${C.border}`,
          borderRadius: 4,
          padding: "5px 7px",
          maxHeight: 120,
          overflowY: "auto",
        }}
      >
        {transport.log.slice(0, 14).map((e, i) => (
          <div key={`${e.at}-${i}`} style={{ color: i === 0 ? C.accent : C.dim }}>
            {e.cmd}
          </div>
        ))}
        {transport.log.length === 0 && <div style={{ color: C.dim }}>—</div>}
      </div>

      <div style={{ color: C.dim, marginTop: 8 }}>
        kill switch: <code>?mock=0</code> or the button above · re-enable with{" "}
        <code>?mock=reset</code>
      </div>
    </div>
  );
}
