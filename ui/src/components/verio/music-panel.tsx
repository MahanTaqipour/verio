/**
 * MusicPanel — the music player surface.
 *
 * Presentation only. All state and command calls live in `useMusic`, so this file
 * can be replaced without touching behaviour.
 *
 * Two behaviour details carried over deliberately from the pre-redesign version:
 *
 *   - **The seek bar drafts, then commits.** `onChange` only moves the handle;
 *     the seek is sent on pointer-up or on a keyboard commit. Seeking on every
 *     `change` during a drag would issue a decoder seek per pixel.
 *   - **The filename is shown under the title.** The old markup put this in a
 *     class called `track-artist`, which was a misnomer — the backend reports no
 *     artist, only a path. It is labelled as a file here rather than dressed up
 *     as metadata the app does not have.
 *
 * The queue shows the queue's real shape: `queue_index` from the backend marks
 * the loaded entry. There is no artwork, no artist and no "shuffle" — the backend
 * has none of those, and inventing them would be a fake feature.
 */

import { Button, IconButton } from "@/components/ui/button";
import { SectionLabel } from "@/components/ui/field";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { EmptyState } from "@/components/verio/empty-state";
import { StateChip, TONE_DOT, type ChipTone } from "@/components/verio/state-chip";
import { Panel } from "@/components/ui/surface";
import {
  IconFileAudio,
  IconFolderOpen,
  IconHeadphones,
  IconMusic,
  IconPause,
  IconPlay,
  IconRepeat,
  IconSkipBack,
  IconSkipForward,
  IconStop,
  IconTrash,
  IconVolume,
  IconX,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";
import {
  fileNameOf,
  formatTime,
  type MusicController,
} from "@/lib/use-music";

export interface MusicPanelProps {
  music: MusicController;
  /** True when this client is deafened — the local monitor is silenced. */
  deafened: boolean;
  onClose: () => void;
  className?: string;
}

/** Playback state as one chip. `active` already means "loaded and playing". */
function statusChip(music: MusicController): { tone: ChipTone; label: string } {
  if (music.status?.active) return { tone: "accent", label: "Sending to room" };
  if (music.hasTrack) return { tone: "idle", label: "Paused" };
  return { tone: "neutral", label: "Idle" };
}

export function MusicPanel({ music, deafened, onClose, className }: MusicPanelProps) {
  const { status, queue, hasTrack } = music;
  const chip = statusChip(music);
  const durationMs = music.durationMs;
  const seekMax = Math.max(1, Math.round(durationMs));

  /** Send the seek for a raw slider value. No-op when nothing is loaded. */
  const commitSeek = (raw: number) => {
    void music.seekTo(raw / seekMax);
  };

  return (
    // Panel, not a hand-rolled `<section class="rounded-md border border-border
    // bg-surface p-4">`. That string WAS the Panel default, spelled out — the
    // Phase 4 rebuild re-implemented a Phase 3 atom. `role="region"` keeps the
    // labelled landmark that `<section aria-label>` gave implicitly.
    <Panel
      role="region"
      aria-label="Music player"
      className={cn("flex flex-col gap-4", className)}
    >
      <header className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <IconMusic size={16} className="shrink-0 text-text-dim" />
          <h2 className="truncate text-14 font-medium text-text">
            {status?.active ? "Now playing" : "Music"}
          </h2>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <StateChip
            tone={chip.tone}
            leading={
              <span className={cn("size-1.5 rounded-full", TONE_DOT[chip.tone])} />
            }
          >
            {chip.label}
          </StateChip>
          <IconButton label="Collapse music player panel" size="sm" onClick={onClose}>
            <IconX size={14} />
          </IconButton>
        </div>
      </header>

      {/* Track identity */}
      <div className="flex min-w-0 flex-col gap-1">
        {status?.title ? (
          <>
            <span className="truncate text-13 text-text">{status.title}</span>
            <span className="flex min-w-0 items-center gap-1.5 text-11 text-text-faint">
              <IconFileAudio size={14} className="shrink-0" />
              <span className="truncate">{fileNameOf(status.path)}</span>
            </span>
          </>
        ) : (
          <span className="text-13 text-text-faint">No track loaded</span>
        )}
      </div>

      {/* Seek */}
      <div className="flex items-center gap-3">
        <Slider
          className="min-w-0 flex-1"
          min={0}
          max={seekMax}
          value={Math.round(music.positionMs)}
          fillPercent={music.percent}
          disabled={durationMs <= 0}
          aria-label="Track playback position"
          valueText={`${formatTime(music.positionMs)} of ${formatTime(durationMs)}`}
          onChange={(e) => music.setSeekDraft(Number(e.currentTarget.value))}
          onPointerUp={(e) => commitSeek(Number(e.currentTarget.value))}
          onKeyUp={(e) => {
            // Only commit on keys that actually move the handle; otherwise Tab
            // and modifiers would fire a seek.
            if (e.key.startsWith("Arrow") || e.key === "Home" || e.key === "End") {
              commitSeek(Number(e.currentTarget.value));
            }
          }}
        />
        <span className="mono-num shrink-0 text-11 text-text-dim">
          {formatTime(music.positionMs)} / {formatTime(durationMs)}
        </span>
      </div>

      {/* Transport */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-1">
          <IconButton
            label="Previous track"
            onClick={() => void music.prev()}
            disabled={queue.length === 0}
          >
            <IconSkipBack size={16} />
          </IconButton>
          <IconButton
            label={status?.playing ? "Pause track" : "Play track"}
            variant="secondary"
            size="lg"
            onClick={() => void music.playPause()}
            disabled={!hasTrack}
          >
            {status?.playing ? <IconPause size={16} /> : <IconPlay size={16} />}
          </IconButton>
          <IconButton
            label="Stop playback"
            onClick={() => void music.stop()}
            disabled={!hasTrack}
          >
            <IconStop size={16} />
          </IconButton>
          <IconButton
            label="Next track"
            onClick={() => void music.next()}
            disabled={queue.length === 0}
          >
            <IconSkipForward size={16} />
          </IconButton>
          <IconButton
            label="Repeat current track"
            aria-pressed={Boolean(status?.loop)}
            className={cn(status?.loop && "text-accent")}
            onClick={() => void music.setTrackLoop(!status?.loop)}
          >
            <IconRepeat size={16} />
          </IconButton>
        </div>

        <div className="flex min-w-0 items-center gap-2">
          <IconVolume size={14} className="shrink-0 text-text-dim" />
          <Slider
            className="w-24"
            min={0}
            max={200}
            value={Math.round(music.volumeDraft * 100)}
            fillPercent={(music.volumeDraft / 2) * 100}
            aria-label="Music volume"
            valueText={`${Math.round(music.volumeDraft * 100)}%`}
            onChange={(e) => music.setVolume(Number(e.currentTarget.value) / 200)}
          />
          <span className="mono-num w-8 shrink-0 text-right text-11 text-text-dim">
            {Math.round(music.volumeDraft * 100)}%
          </span>
        </div>
      </div>

      {/* Monitor + add */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div className="flex min-w-0 items-center gap-2">
          <Switch
            label="Local monitor playback"
            checked={Boolean(status?.monitor)}
            onChange={(e) => void music.setMonitor(e.currentTarget.checked)}
          />
          <span className="text-11 text-text-dim">Local monitor</span>
          {deafened && status?.active ? (
            <span
              className="flex items-center gap-1.5 text-11 text-warn"
              title="Your client is deafened; the music monitor is silenced locally. Music still reaches the room."
            >
              <IconHeadphones size={14} className="shrink-0" />
              Silenced locally
            </span>
          ) : null}
        </div>
        <Button
          variant="secondary"
          size="sm"
          icon={<IconFolderOpen size={14} />}
          onClick={() => void music.addFiles()}
        >
          Add files
        </Button>
      </div>

      <div className="h-px w-full bg-border" />

      {/* Queue */}
      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <SectionLabel level={4} className="text-text-dim">
            Queue{queue.length > 0 ? ` · ${queue.length}` : ""}
          </SectionLabel>
          {queue.length > 0 ? (
            <div className="flex items-center gap-2">
              <Button
                variant="ghost"
                size="sm"
                aria-pressed={Boolean(status?.queue_looping)}
                className={cn(status?.queue_looping && "text-accent")}
                onClick={() => void music.setQueueLooping(!status?.queue_looping)}
              >
                Repeat queue
              </Button>
              <Button variant="ghost" size="sm" onClick={() => void music.clearQueue()}>
                Clear
              </Button>
            </div>
          ) : null}
        </div>

        {queue.length === 0 ? (
          <EmptyState
            title="Queue is empty"
            description="Add audio files to build a playlist. Tracks play in order and advance on their own."
            className="px-2 py-6"
          />
        ) : (
          <ul className="flex max-h-40 flex-col overflow-y-auto">
            {queue.map((item, i) => {
              const isCurrent = status?.queue_index === i;
              return (
                <li
                  key={`${item.path}:${i}`}
                  className={cn(
                    "group flex items-center gap-2 rounded-sm px-2 py-1.5",
                    isCurrent && "bg-surface-2",
                  )}
                >
                  <button
                    type="button"
                    onClick={() => void music.jumpTo(i)}
                    title={`Play ${item.title}`}
                    className="flex min-w-0 flex-1 items-center gap-2 text-left"
                  >
                    <span
                      className={cn(
                        "mono-num w-5 shrink-0 text-right text-10",
                        isCurrent ? "text-accent" : "text-text-faint",
                      )}
                    >
                      {isCurrent ? "▶" : i + 1}
                    </span>
                    <span
                      className={cn(
                        "truncate text-13",
                        isCurrent ? "text-accent" : "text-text",
                      )}
                    >
                      {item.title}
                    </span>
                    <span className="mono-num ml-auto shrink-0 text-10 text-text-faint">
                      {formatTime(item.duration_ms)}
                    </span>
                  </button>
                  <IconButton
                    label={`Remove ${item.title} from queue`}
                    size="sm"
                    className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
                    onClick={() => void music.removeAt(i)}
                  >
                    <IconTrash size={14} />
                  </IconButton>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </Panel>
  );
}
