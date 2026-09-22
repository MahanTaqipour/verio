import React from "react";
import ReactDOM from "react-dom/client";
import "./styles/index.css";
import App from "./App";

function mount() {
  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <React.StrictMode>
      <App />
    </React.StrictMode>,
  );
}

/**
 * Development-only bootstrap.
 *
 * `import.meta.env.DEV` is statically replaced by Vite, so a production build
 * takes the `else` branch: one synchronous `mount()`, identical behaviour to
 * before the harness existed. The dynamic `import("./dev")` lives inside the
 * dead branch, so the harness and its fixtures are removed from the bundle
 * entirely by dead-code elimination.
 *
 * In development the harness is awaited before mounting, so the application's
 * first `invoke()` already has a transport. When the harness declines to
 * install (no `?mock=1`, a real Tauri runtime, or the kill switch) it resolves
 * immediately and mounting proceeds unchanged.
 *
 * Safety model: see `src/dev/index.ts`.
 */
if (import.meta.env.DEV) {
  void import("./dev").then((dev) => {
    dev.installMockHarness();
    mount();
  });
} else {
  mount();
}
