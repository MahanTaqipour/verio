import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * AmbientGlassCanvas — true translucent Midnight Violet acrylic shell.
 *
 * In Tauri desktop runtime, `html, body, #root` stay 100% transparent so the OS
 * Windows 10/11 DWM Acrylic blur bleeds softly through the `rgba(20,14,34,0.62)`
 * layer and floating glass islands. In browser-only `?mock=1` preview mode,
 * a subtle multi-tone wallpaper mesh renders *behind* the frosted blur layer so
 * the translucent refraction is immediately visible during visual review.
 */
export interface AmbientGlassCanvasProps extends HTMLAttributes<HTMLDivElement> {
  children: ReactNode;
}

function isBrowserMockPreview(): boolean {
  if (typeof window === "undefined") return false;
  return new URLSearchParams(window.location.search).get("mock") === "1";
}

export function AmbientGlassCanvas({
  children,
  className,
  ...rest
}: AmbientGlassCanvasProps) {
  const showMockWallpaper = isBrowserMockPreview();

  return (
    <div
      className={cn(
        "relative flex h-screen w-screen flex-col overflow-hidden select-none text-text",
        className,
      )}
      {...rest}
    >
      {/* Browser-only `?mock=1` desktop backdrop behind the translucent window */}
      {showMockWallpaper && (
        <div
          aria-hidden="true"
          className="pointer-events-none fixed inset-0 z-0"
          style={{
            background: [
              "radial-gradient(64% 68% at 14% 18%, rgba(185, 167, 230, 0.42) 0%, transparent 70%)",
              "radial-gradient(56% 60% at 86% 84%, rgba(232, 255, 58, 0.24) 0%, transparent 70%)",
              "radial-gradient(52% 54% at 80% 16%, rgba(16, 185, 129, 0.24) 0%, transparent 68%)",
              "radial-gradient(60% 60% at 22% 86%, rgba(139, 92, 246, 0.36) 0%, transparent 72%)",
              "linear-gradient(135deg, #1b1230 0%, #0e0a1a 50%, #19132e 100%)",
            ].join(", "),
          }}
        />
      )}

      {/* Translucent Midnight Violet frosted canvas layer + floating outer padding */}
      <div className="relative z-10 flex min-h-0 flex-1 flex-col gap-2.5 overflow-hidden bg-[var(--bg-window-glass)] p-2.5 backdrop-blur-2xl">
        {children}
      </div>
    </div>
  );
}
