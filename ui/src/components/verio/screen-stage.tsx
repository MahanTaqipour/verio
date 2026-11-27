import { useEffect, useRef, useState } from "react";
import { Button, IconButton } from "@/components/ui/button";
import { Slider } from "@/components/ui/slider";
import {
  IconCollapse,
  IconExpand,
  IconSpinner,
  IconVolume,
  IconVolumeOff,
} from "@/components/verio/icons";
import type { ScreenStreamStats } from "@/lib/screen-share-utils";
import { cn } from "@/lib/utils";

export interface ScreenStageProps {
  stream: MediaStream | null;
  sharerName: string;
  isLocalPreview: boolean;
  deafened: boolean;
  stats: ScreenStreamStats | null;
  onStopWatching: () => void;
  className?: string;
}

export function ScreenStage({
  stream,
  sharerName,
  isLocalPreview,
  deafened,
  stats,
  onStopWatching,
  className,
}: ScreenStageProps) {
  const stageRef = useRef<HTMLDivElement>(null);
  const videoRef = useRef<HTMLVideoElement>(null);

  const [streamMuted, setStreamMuted] = useState(false);
  const [streamVolume, setStreamVolume] = useState(1);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [videoDims, setVideoDims] = useState<{ w: number; h: number } | null>(null);

  // Attach MediaStream to <video> cleanly.
  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    if (el.srcObject !== stream) {
      el.srcObject = stream;
    }
    setVideoDims(null);
  }, [stream]);

  // Apply mute/Deafen/volume rules (local preview and deafened always mute audio).
  useEffect(() => {
    const el = videoRef.current;
    if (!el) return;
    const forceMuted = isLocalPreview || deafened || streamMuted;
    el.muted = forceMuted;
    if (!forceMuted) {
      el.volume = Math.max(0, Math.min(1, streamVolume));
    }
  }, [deafened, isLocalPreview, streamMuted, streamVolume, stream]);

  // Sync fullscreen indicator when user exits fullscreen via Escape.
  useEffect(() => {
    const onFsChange = () => {
      setIsFullscreen(document.fullscreenElement === stageRef.current);
    };
    document.addEventListener("fullscreenchange", onFsChange);
    return () => document.removeEventListener("fullscreenchange", onFsChange);
  }, []);

  const handleToggleFullscreen = async () => {
    const el = stageRef.current;
    if (!el) return;
    try {
      if (document.fullscreenElement === el) {
        await document.exitFullscreen();
      } else {
        await el.requestFullscreen();
      }
    } catch {
      // Ignore fullscreen denial in restricted contexts
    }
  };

  // Only display real measured resolution/FPS (never invented constants).
  const measuredW = stats?.width ?? (isLocalPreview ? videoDims?.w ?? null : null);
  const measuredH = stats?.height ?? (isLocalPreview ? videoDims?.h ?? null : null);
  const measuredFps = stats?.fps ?? null;
  const resolutionLabel =
    measuredW && measuredH
      ? measuredFps !== null
        ? `${measuredW}×${measuredH} · ${measuredFps}fps`
        : `${measuredW}×${measuredH}`
      : null;

  const effectiveAudioMuted = isLocalPreview || deafened || streamMuted;
  const effectiveVolume = effectiveAudioMuted ? 0 : streamVolume;

  return (
    <section
      ref={stageRef}
      aria-label={
        isLocalPreview
          ? "Local screen share preview"
          : `Watching ${sharerName}'s screen`
      }
      className={cn(
        "flex min-h-0 flex-1 flex-col overflow-hidden rounded-[22px] border border-border-2 bg-[#0B0814]/90",
        className,
      )}
    >
      {/* Video Viewport */}
      <div className="relative flex min-h-0 flex-1 items-center justify-center overflow-hidden bg-[#0B0814]/95">
        <video
          ref={videoRef}
          autoPlay
          playsInline
          onLoadedMetadata={() => {
            const el = videoRef.current;
            if (el && el.videoWidth > 0 && el.videoHeight > 0) {
              setVideoDims({ w: el.videoWidth, h: el.videoHeight });
            }
          }}
          className="h-full w-full object-contain"
        />

        {!stream && (
          <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-3 text-text-dim">
            <IconSpinner size={20} className="animate-spin text-lavender" />
            <span className="text-13 font-medium">
              {isLocalPreview
                ? "Initializing local preview…"
                : `Connecting to ${sharerName}'s screen…`}
            </span>
          </div>
        )}
      </div>

      {/* Bottom Theater Control Bar */}
      <div className="flex h-12 shrink-0 items-center justify-between gap-3 border-t border-border bg-[rgba(18,13,29,0.75)] px-4 backdrop-blur-xl">
        <div className="flex min-w-0 items-center gap-2.5">
          <span
            aria-hidden="true"
            className="size-2 shrink-0 rounded-full bg-accent shadow-[0_0_8px_rgba(232,255,58,0.65)]"
          />
          <span className="truncate text-13 font-semibold text-text">
            {isLocalPreview ? "Local Preview" : `Watching ${sharerName}`}
          </span>

          {resolutionLabel && (
            <span className="mono-num shrink-0 rounded-full border border-border bg-surface-2/85 px-2.5 py-0.5 font-mono text-11 text-text-dim">
              {resolutionLabel}
            </span>
          )}

          {isLocalPreview ? (
            <span className="shrink-0 rounded-full border border-border bg-surface-2/85 px-2.5 py-0.5 font-mono text-10 uppercase tracking-chip text-text-dim">
              Local
            </span>
          ) : stats?.transport === "P2P" ? (
            <span className="shrink-0 rounded-full border border-idle/40 bg-idle/14 px-2.5 py-0.5 font-mono text-10 font-semibold uppercase tracking-chip text-[#10B981]">
              P2P
            </span>
          ) : stats?.transport === "Relay" ? (
            <span className="shrink-0 rounded-full border border-[#B9A7E6]/45 bg-[#B9A7E6]/14 px-2.5 py-0.5 font-mono text-10 font-semibold uppercase tracking-chip text-[#d4c7f7]">
              Relay
            </span>
          ) : (
            <span className="shrink-0 rounded-full border border-border bg-surface-2/85 px-2.5 py-0.5 font-mono text-10 uppercase tracking-chip text-text-dim">
              Connecting…
            </span>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {!isLocalPreview && (
            <div className="flex items-center gap-2 pr-1">
              <IconButton
                label={
                  deafened
                    ? "Stream audio muted (Deafened)"
                    : streamMuted
                      ? "Unmute stream audio"
                      : "Mute stream audio"
                }
                size="sm"
                disabled={deafened}
                aria-pressed={effectiveAudioMuted}
                onClick={() => setStreamMuted((m) => !m)}
                className={cn(effectiveAudioMuted && "text-warn")}
              >
                {effectiveAudioMuted ? (
                  <IconVolumeOff size={14} />
                ) : (
                  <IconVolume size={14} />
                )}
              </IconButton>

              <Slider
                min={0}
                max={1}
                step={0.01}
                disabled={deafened}
                value={effectiveVolume}
                fillPercent={effectiveVolume * 100}
                valueText={`${Math.round(effectiveVolume * 100)} percent`}
                aria-label="Stream volume"
                onChange={(e) => {
                  const next = parseFloat(e.target.value);
                  setStreamVolume(next);
                  if (streamMuted && next > 0) setStreamMuted(false);
                }}
                className="w-24"
              />
            </div>
          )}

          <IconButton
            label={isFullscreen ? "Exit fullscreen" : "Enter fullscreen"}
            size="sm"
            aria-pressed={isFullscreen}
            onClick={() => void handleToggleFullscreen()}
          >
            {isFullscreen ? <IconCollapse size={14} /> : <IconExpand size={14} />}
          </IconButton>

          <Button
            variant="secondary"
            size="sm"
            onClick={onStopWatching}
            className="rounded-xl"
          >
            {isLocalPreview ? "Close Preview" : "Stop Watching"}
          </Button>
        </div>
      </div>
    </section>
  );
}
