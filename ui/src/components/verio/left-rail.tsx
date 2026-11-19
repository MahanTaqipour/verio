import {
  IconMusic,
  IconSliders,
  IconUsers,
} from "@/components/verio/icons";
import { cn } from "@/lib/utils";

export type NavView = "stage" | "music" | "settings";

export interface LeftIconRailProps {
  activeView: NavView;
  onSelectView: (view: NavView) => void;
  /** Shows a live Acid Lime dot on Music while playback is active. */
  musicPlaying?: boolean;
  /** Highlights whether an active call is underway in Room. */
  isInCall?: boolean;
  className?: string;
}

const NAV_ITEMS: ReadonlyArray<{
  id: NavView;
  label: string;
  title: string;
  icon: typeof IconUsers;
}> = [
  { id: "stage", label: "Room", title: "Lobby & voice room", icon: IconUsers },
  { id: "music", label: "Music", title: "Music player & playlist", icon: IconMusic },
  { id: "settings", label: "Settings", title: "Audio, hotkeys & server settings", icon: IconSliders },
];

export function LeftIconRail({
  activeView,
  onSelectView,
  musicPlaying = false,
  isInCall = false,
  className,
}: LeftIconRailProps) {
  return (
    <aside
      aria-label="Workspace navigation"
      className={cn(
        "relative z-20 flex h-full w-[72px] shrink-0 flex-col items-center justify-between",
        "rounded-2xl border border-border bg-[var(--bg-rail-glass)] px-2 py-3 backdrop-blur-2xl",
        className,
      )}
    >
      <nav aria-label="Primary sections" className="flex w-full flex-col items-center gap-2">
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const active = activeView === item.id;
          const showRoomDot = item.id === "stage" && isInCall;
          const showMusicDot = item.id === "music" && musicPlaying;

          return (
            <button
              key={item.id}
              type="button"
              onClick={() => onSelectView(item.id)}
              aria-label={item.title}
              aria-current={active ? "page" : undefined}
              title={item.title}
              className={cn(
                "relative flex w-full flex-col items-center justify-center gap-1.5 rounded-xl py-2.5",
                "border transition-colors ease-verio press",
                active
                  ? "border-[rgba(232,255,58,0.42)] bg-[rgba(232,255,58,0.14)] text-accent"
                  : "border-transparent text-text-dim hover:bg-[rgba(185,167,230,0.12)] hover:text-text",
              )}
            >
              {active && (
                <span
                  aria-hidden="true"
                  className="absolute top-2.5 bottom-2.5 left-0.5 w-[3px] rounded-full bg-accent shadow-[0_0_8px_#E8FF3A]"
                />
              )}

              <span className="relative inline-flex items-center justify-center">
                <Icon size={16} />
                {(showMusicDot || showRoomDot) && (
                  <span
                    aria-hidden="true"
                    className={cn(
                      "absolute -top-1 -right-1.5 size-1.5 rounded-full",
                      showMusicDot
                        ? "bg-accent shadow-[0_0_6px_#E8FF3A]"
                        : "bg-idle shadow-[0_0_6px_rgba(16,185,129,0.75)]",
                    )}
                  />
                )}
              </span>

              <span className="font-mono text-[9px] uppercase tracking-[0.05em] leading-none">
                {item.label}
              </span>
            </button>
          );
        })}
      </nav>

      {/* Bottom readiness indicator */}
      <div className="flex flex-col items-center gap-1.5 px-1 text-center">
        <span
          aria-hidden="true"
          className={cn(
            "size-2 rounded-full",
            isInCall ? "bg-idle shadow-[0_0_8px_rgba(16,185,129,0.7)]" : "bg-text-faint/60",
          )}
        />
        <span className="font-mono text-[9px] uppercase tracking-chip text-text-faint">
          {isInCall ? "Live" : "Ready"}
        </span>
      </div>
    </aside>
  );
}
