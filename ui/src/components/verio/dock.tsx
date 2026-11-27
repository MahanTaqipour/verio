import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { Button, IconButton } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import { KbdChip } from "@/components/verio/kbd-chip";
import { StateChip } from "@/components/verio/state-chip";
import { MicLevelMeter, type LevelMeterHandle } from "@/components/verio/level-meter";
import {
  IconActivity,
  IconCheck,
  IconHeadphoneOff,
  IconHeadphones,
  IconLeave,
  IconMic,
  IconMicOff,
  IconMusic,
  IconPause,
  IconPencil,
  IconPlay,
  IconScreen,
  IconScreenOff,
  IconSkipForward,
  IconX,
} from "@/components/verio/icons";
import {
  SCREEN_PRESET_ORDER,
  SCREEN_QUALITY_PRESETS,
  type ScreenPresetId,
} from "@/lib/screen-share-utils";
import { cn } from "@/lib/utils";

export interface DockMusic {
  active: boolean;
  playing: boolean;
  title?: string | null;
  path?: string | null;
}

export interface DockProps {
  nickname: string;
  mode: "open" | "ptt";
  pttHotkey: string;
  speaking: boolean;
  isInCall: boolean;

  isEditingNickname: boolean;
  nicknameInput: string;
  onNicknameInputChange: (value: string) => void;
  onStartEditNickname: () => void;
  onSaveNickname: () => void;
  onCancelEditNickname: () => void;

  muted: boolean;
  deafened: boolean;
  loopback: boolean;
  onToggleMute: () => void;
  onToggleDeafen: () => void;
  onToggleLoopback: () => void;
  muteHotkey: string;
  deafenHotkey: string;

  isSharingScreen?: boolean;
  screenWatcherCount?: number;
  onStartScreenShare?: (presetId: ScreenPresetId, includeAudio: boolean) => void;
  onStopScreenShare?: () => void;

  musicPanelOpen: boolean;
  onToggleMusicPanel: () => void;
  onOpenMusicPanel: () => void;
  music: DockMusic | null;
  musicPercent: number;
  hasTrack: boolean;
  onMusicPlayPause: () => void;
  onMusicNext: () => void;
  onLeaveCall?: () => void;

  meterRef: RefObject<LevelMeterHandle | null>;
}

function shortKey(hotkey: string): string {
  return hotkey.replace(/^Ctrl\+Shift\+/, "");
}

export function Dock({
  nickname,
  mode,
  pttHotkey,
  speaking,
  isInCall,
  isEditingNickname,
  nicknameInput,
  onNicknameInputChange,
  onStartEditNickname,
  onSaveNickname,
  onCancelEditNickname,
  muted,
  deafened,
  loopback,
  onToggleMute,
  onToggleDeafen,
  onToggleLoopback,
  muteHotkey,
  deafenHotkey,
  isSharingScreen = false,
  screenWatcherCount = 0,
  onStartScreenShare,
  onStopScreenShare,
  musicPanelOpen,
  onToggleMusicPanel,
  onOpenMusicPanel,
  music,
  musicPercent,
  hasTrack,
  onMusicPlayPause,
  onMusicNext,
  onLeaveCall,
  meterRef,
}: DockProps) {
  const initial = (nickname || "User").slice(0, 1).toUpperCase();
  const showStrip = Boolean(music && (music.active || music.playing || hasTrack));
  const playing = Boolean(music?.playing);

  const [shareMenuOpen, setShareMenuOpen] = useState(false);
  const [selectedPreset, setSelectedPreset] = useState<ScreenPresetId>("720p30");
  const [includeAudio, setIncludeAudio] = useState(true);
  const sharePopoverWrapRef = useRef<HTMLDivElement>(null);
  const shareTriggerRef = useRef<HTMLButtonElement>(null);
  const systemAudioSwitchId = useId();
  const sharePopoverId = useId();

  // Close popover if user leaves the room or begins sharing.
  useEffect(() => {
    if (!isInCall || isSharingScreen) {
      setShareMenuOpen(false);
    }
  }, [isInCall, isSharingScreen]);

  // Light-dismiss on Escape (returning focus to trigger) or click outside.
  useEffect(() => {
    if (!shareMenuOpen) return;
    const onPointerDown = (e: PointerEvent) => {
      if (
        sharePopoverWrapRef.current &&
        !sharePopoverWrapRef.current.contains(e.target as Node)
      ) {
        setShareMenuOpen(false);
      }
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        setShareMenuOpen(false);
        shareTriggerRef.current?.focus();
      }
    };
    window.addEventListener("pointerdown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("pointerdown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [shareMenuOpen]);

  return (
    <footer
      className={cn(
        "@container relative z-30 flex h-[60px] shrink-0 items-center justify-between gap-4",
        "rounded-2xl border border-border bg-[var(--bg-titlebar-glass)] px-5 backdrop-blur-2xl",
      )}
    >
      {/* Left Cluster: Identity + Mode + Imperative 20 Hz Mic Meter */}
      <div className="flex min-w-0 flex-1 items-center gap-3">
        <div
          aria-hidden="true"
          className={cn(
            "grid size-9 shrink-0 place-items-center rounded-full font-mono text-13 font-bold transition-colors ease-verio",
            speaking
              ? "bg-accent text-on-accent shadow-[0_0_12px_rgba(232,255,58,0.45)]"
              : "border border-border-2 bg-surface-2 text-lavender-bright",
          )}
        >
          {initial}
        </div>

        {isEditingNickname ? (
          <div className="flex shrink-0 items-center gap-1">
            <input
              type="text"
              value={nicknameInput}
              onChange={(e) => onNicknameInputChange(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") onSaveNickname();
                if (e.key === "Escape") onCancelEditNickname();
              }}
              aria-label="Display name"
              autoFocus
              className="h-8 w-28 rounded-xl border border-border-2 bg-[var(--bg-input)] px-3 text-13 text-text outline-none focus-visible:border-accent"
            />
            <IconButton label="Save display name" size="sm" onClick={onSaveNickname}>
              <IconCheck size={14} />
            </IconButton>
          </div>
        ) : (
          <div className="flex min-w-0 items-center gap-2">
            <button
              type="button"
              onClick={onStartEditNickname}
              title="Click to edit display name"
              className="group flex min-w-0 items-center gap-1.5 rounded-xl px-1.5 py-1 text-left hover:bg-[rgba(185,167,230,0.12)]"
            >
              <span className="max-w-28 truncate text-13 font-semibold text-text">
                {nickname}
              </span>
              <IconPencil
                size={14}
                className="shrink-0 text-text-faint opacity-0 transition-opacity group-hover:opacity-100"
              />
            </button>

            <StateChip
              tone={mode === "ptt" ? "accent" : "neutral"}
              title={
                mode === "ptt"
                  ? `Push-to-Talk (${pttHotkey || "Unbound"})`
                  : "Voice Activity Detection (Open Mic)"
              }
            >
              {mode === "ptt" ? (pttHotkey ? `PTT ${shortKey(pttHotkey)}` : "PTT") : "VAD"}
            </StateChip>
          </div>
        )}

        <MicLevelMeter
          ref={meterRef}
          className="@max-[880px]:hidden max-w-28 min-w-16"
          fillClassName={speaking ? "bg-accent" : "bg-[#b9a7e6]"}
        />
      </div>

      {/* Center Cluster: Tactile Audio & Screen Hardware Controls */}
      <div className="flex shrink-0 items-center gap-2">
        <Button
          variant="secondary"
          size="md"
          className={cn(
            "h-9 px-3.5",
            muted && "border-warn/45 bg-warn/15 text-warn hover:border-warn hover:bg-warn/20",
          )}
          onClick={onToggleMute}
          aria-pressed={muted}
          aria-label="Toggle Mic mute"
          title={`Mic mute (${muteHotkey || "Unbound"})`}
          icon={muted ? <IconMicOff size={16} /> : <IconMic size={16} />}
          trailingIcon={muteHotkey ? <KbdChip decorative>{shortKey(muteHotkey)}</KbdChip> : undefined}
        >
          <span>{muted ? "Unmute" : "Mic"}</span>
        </Button>

        <Button
          variant="secondary"
          size="md"
          className={cn(
            "h-9 px-3.5",
            deafened &&
              "border-danger/50 bg-danger/15 text-[#fca5a5] hover:border-danger hover:bg-danger/25",
          )}
          onClick={onToggleDeafen}
          aria-pressed={deafened}
          aria-label="Toggle Deafen"
          title={`Deafen (${deafenHotkey || "Unbound"})`}
          icon={deafened ? <IconHeadphoneOff size={16} /> : <IconHeadphones size={16} />}
          trailingIcon={
            deafenHotkey ? <KbdChip decorative>{shortKey(deafenHotkey)}</KbdChip> : undefined
          }
        >
          <span>{deafened ? "Deafened" : "Deafen"}</span>
        </Button>

        <Button
          variant="secondary"
          size="md"
          className={cn(
            "h-9 px-3.5",
            loopback &&
              "border-[rgba(232,255,58,0.45)] bg-[rgba(232,255,58,0.14)] text-accent hover:border-accent",
          )}
          onClick={onToggleLoopback}
          aria-pressed={loopback}
          aria-label="Toggle microphone loopback test"
          title="Hear your own microphone locally (Loopback test)"
          icon={<IconActivity size={14} />}
        >
          <span>Loopback</span>
        </Button>

        {isInCall && (onStartScreenShare || onStopScreenShare) && (
          <div ref={sharePopoverWrapRef} className="relative">
            {shareMenuOpen && !isSharingScreen && (
              <div
                id={sharePopoverId}
                role="region"
                aria-label="Screen share quality options"
                className="glass-card animate-lift absolute bottom-full left-1/2 z-50 mb-3 w-72 -translate-x-1/2 rounded-2xl p-4 shadow-[var(--shadow-overlay)]"
              >
                <div className="mb-3 flex items-center justify-between gap-2">
                  <div className="flex flex-col">
                    <span className="text-13 font-semibold text-text">
                      Share Screen
                    </span>
                    <span className="text-11 text-text-dim">
                      Watch-on-Demand · 0 B/s until watched
                    </span>
                  </div>
                  <IconButton
                    label="Close share options"
                    size="sm"
                    onClick={() => {
                      setShareMenuOpen(false);
                      shareTriggerRef.current?.focus();
                    }}
                  >
                    <IconX size={14} />
                  </IconButton>
                </div>

                <div className="mb-3 grid grid-cols-2 gap-1.5">
                  {SCREEN_PRESET_ORDER.map((presetId) => {
                    const preset = SCREEN_QUALITY_PRESETS[presetId];
                    const active = selectedPreset === presetId;
                    return (
                      <button
                        key={presetId}
                        type="button"
                        aria-pressed={active}
                        onClick={() => setSelectedPreset(presetId)}
                        className={cn(
                          "flex min-h-9 flex-col items-start justify-center rounded-xl border px-3 py-1.5 text-left transition-colors ease-verio press",
                          active
                            ? "border-accent/55 bg-accent/14 text-accent"
                            : "border-border bg-surface-2/85 text-text-dim hover:border-border-2 hover:text-text",
                        )}
                      >
                        <span className="mono-num font-mono text-11 font-semibold">
                          {preset.label}
                        </span>
                        {preset.badge && (
                          <span className="text-10 text-lavender-bright">
                            {preset.badge}
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>

                <div className="mb-3 flex items-center justify-between gap-3 rounded-xl border border-border bg-[var(--bg-input)] px-3 py-2">
                  <label
                    htmlFor={systemAudioSwitchId}
                    className="cursor-pointer text-13 font-medium text-text"
                  >
                    Share system audio
                  </label>
                  <Switch
                    id={systemAudioSwitchId}
                    checked={includeAudio}
                    onChange={(e) => setIncludeAudio(e.target.checked)}
                  />
                </div>

                <button
                  type="button"
                  onClick={() => {
                    setShareMenuOpen(false);
                    onStartScreenShare?.(selectedPreset, includeAudio);
                  }}
                  className="h-9 w-full rounded-xl bg-accent px-4 text-13 font-bold text-[#120D1D] transition-colors hover:bg-accent-hi active:bg-accent-lo press"
                >
                  Start Sharing
                </button>
              </div>
            )}

            <Button
              ref={shareTriggerRef}
              variant="secondary"
              size="md"
              className={cn(
                "h-9 px-3.5",
                isSharingScreen &&
                  "border-[rgba(232,255,58,0.45)] bg-[rgba(232,255,58,0.14)] text-accent hover:border-accent",
              )}
              onClick={() => {
                if (isSharingScreen) {
                  setShareMenuOpen(false);
                  onStopScreenShare?.();
                } else {
                  setShareMenuOpen((v) => !v);
                }
              }}
              aria-pressed={isSharingScreen}
              aria-expanded={isSharingScreen ? undefined : shareMenuOpen}
              aria-controls={isSharingScreen ? undefined : sharePopoverId}
              aria-label={isSharingScreen ? "Stop screen sharing" : "Share screen"}
              title={
                isSharingScreen
                  ? "Stop screen sharing"
                  : "Share screen (Watch-on-Demand)"
              }
              icon={
                isSharingScreen ? (
                  <IconScreenOff size={16} />
                ) : (
                  <IconScreen size={16} />
                )
              }
            >
              <span className="mono-num">
                {isSharingScreen ? `Sharing · ${screenWatcherCount}` : "Share"}
              </span>
            </Button>
          </div>
        )}
      </div>

      {/* Right Cluster: Mini Music Transport & Single Leave Call Action */}
      <div className="flex min-w-0 flex-1 items-center justify-end gap-2">
        <IconButton
          label="Open music studio"
          title={playing ? "Music studio (playing)" : "Music studio"}
          onClick={onToggleMusicPanel}
          aria-pressed={musicPanelOpen}
          variant={musicPanelOpen ? "secondary" : "ghost"}
          size="md"
          className={cn(
            (musicPanelOpen || playing) && "text-accent",
            musicPanelOpen && "border-[rgba(232,255,58,0.45)] bg-[rgba(232,255,58,0.14)]",
          )}
        >
          <IconMusic size={16} />
        </IconButton>

        {showStrip && (
          <div className="@max-[940px]:hidden flex min-w-0 items-center gap-1.5 rounded-xl border border-border bg-[var(--bg-input)] p-1 pr-1.5">
            <button
              type="button"
              onClick={onOpenMusicPanel}
              title={music?.title ? `${music.title} — open Music studio` : "Open Music studio"}
              className="flex min-w-0 items-center gap-2 rounded-lg px-2 py-0.5 text-left hover:bg-surface-2"
            >
              <span className="flex w-24 min-w-0 flex-col gap-0.5">
                <span className="truncate text-11 font-medium text-text">
                  {music?.title || "Music loaded"}
                </span>
                <span className="h-1 overflow-hidden rounded-full bg-surface-3">
                  <span
                    className="block h-full rounded-full bg-[#b9a7e6] transition-[width] duration-200 ease-verio"
                    style={{ width: `${musicPercent}%` }}
                  />
                </span>
              </span>
            </button>
            <IconButton
              label={playing ? "Pause track (dock)" : "Play track (dock)"}
              size="sm"
              onClick={onMusicPlayPause}
              disabled={!hasTrack}
              className="text-accent hover:bg-[rgba(232,255,58,0.14)]"
            >
              {playing ? <IconPause size={14} filled /> : <IconPlay size={14} filled />}
            </IconButton>
            <IconButton
              label="Next track (dock)"
              size="sm"
              onClick={onMusicNext}
              disabled={!hasTrack}
            >
              <IconSkipForward size={14} />
            </IconButton>
          </div>
        )}

        {isInCall && onLeaveCall && (
          <Button
            variant="danger-outline"
            size="md"
            onClick={onLeaveCall}
            icon={<IconLeave size={14} />}
            className="h-9 shrink-0 px-4 font-semibold"
          >
            Leave Call
          </Button>
        )}
      </div>
    </footer>
  );
}
