/**
 * Lobby — the pre-call screen: create a room, or join one by code.
 *
 * PRESENTATION ONLY. All state and every command lives in App.tsx; this receives
 * `code`/`onCodeChange` and the two actions. The code string stays owned by
 * App.tsx deliberately: the original kept it there, so moving it would have
 * reset the field whenever this screen unmounted, which is a behaviour change
 * dressed up as a refactor.
 *
 * TWO DEFECTS IN THE LEGACY MARKUP, FIXED HERE
 * --------------------------------------------
 * 1. **The container scrolled inside `.main`, which already scrolls.**
 *    The old `.lobby-container` was `flex: 1` with `overflow-y: auto`. That inner
 *    scroller was not decoration — it was load-bearing. `flex: 1` means
 *    `flex: 1 1 0%`, so the container's height comes from its parent and its
 *    content overflows *inside* it without ever making `.main` scroll; the inner
 *    scroller was the only way to reach that content. Deleting it without fixing
 *    the growth would have made tall content unreachable.
 *
 *    The fix is `min-h-full`: the container is at least the height of `.main`
 *    (so `justify-center` still centres short content) but is free to grow past
 *    it (so `.main` does the scrolling). One scroller, and no clipping.
 *
 * 2. **The code boxes and the Join button fought over one row.** The legacy
 *    `.code-input-row` put a 4-box group and the button side by side, both
 *    `flex: 1`. The boxes declare 52px each and need 232px with gaps, but at the
 *    700px floor their half of the row is ~141px, so they were squeezed to about
 *    31px — measured on the captured baseline, not guessed. The rebuild stacks
 *    them instead, which is also consistent with the Host Call card's
 *    full-width action. Verified at 700 and 860.
 *
 * Sizing notes: the boxes are 40px (`size-10`) to match `Button size="lg"`, and
 * their digits are 18px — the legacy 52px/20px pair was off both design.md
 * scales (spacing is 4·8·12·16·20·24·32·40·60; type is 10·11·13·14·15·18·24).
 *
 * Both actions carry `mt-auto` so they sit on the cards' bottom edge. The grid
 * stretches the two cards to equal height, and without this the Host Call
 * button floated mid-card with ~70px of dead space under it at 860 (measured on
 * the capture, not guessed) — the two cards read as one row only when their
 * actions line up.
 *
 * THE CONTENT COLUMN IS CAPPED AT `max-w-256` (1024px)
 * ---------------------------------------------------
 * Measured uncapped, the two cards grew without limit: 302px each at 700, then
 * 382 / 402 / 502 / 652 as the window widened to 860 / 900 / 1100 / 1400. At
 * 1400 that is a 1324px row of two very wide cards with a lot of dead space in
 * each, and `design.md` states no maximum content width of its own.
 *
 * 1024px is where the design's own "standard desktop" band (901–1150px) tops
 * out: uncapped, the content reaches ~1020px at a 1100px window, so the cap
 * does not engage until just past the largest window the design was drawn for,
 * and above that the column simply centres. `256` is 256 × the 4px `--spacing`
 * baseline — the same convention as the existing `max-w-140` cap elsewhere in
 * the app, rather than an arbitrary `max-w-[1024px]`.
 *
 * THE PLAN'S "56px RAIL SIDEBAR" DOES NOT EXIST IN THE DESIGN
 * -----------------------------------------------------------
 * `REDESIGN-PLAN.md` §4.1 described this screen as "Direction 1 Split Workbench,
 * with the sidebar collapsing to a 56px rail below 900px and the two action
 * panels stacking below 900px". Neither half survives contact with the deck:
 * the lobby `code.html` has no sidebar at all (its only grids are `.action-grid`
 * and `.state-grid`, and `.state-grid` is the six-card states matrix — deck
 * presentation chrome, not app UI), and `.action-grid` is
 * `grid-template-columns: 1fr 1fr` with **no media query**, so the deck never
 * stacks the two panels. The plan's sentence appears to have read `.state-grid`'s
 * 1180px/768px breakpoints as the app's. The plan has been corrected to match
 * the deck; `sm:grid-cols-2` below is the deck's intent.
 */

import { Button } from "@/components/ui/button";
import { Panel } from "@/components/ui/surface";
import { RoomCodeInput } from "@/components/verio/room-code-input";
import { cn } from "@/lib/utils";

export interface LobbyProps {
  /** Current room-code entry, owned by the screen. */
  code: string;
  onCodeChange: (next: string) => void;
  onCreateRoom: () => void;
  onJoinRoom: () => void;
  /** A room action is in flight; both actions are blocked while true. */
  busy?: boolean;
  className?: string;
}

const CODE_LENGTH = 4;

export function Lobby({
  code,
  onCodeChange,
  onCreateRoom,
  onJoinRoom,
  busy = false,
  className,
}: LobbyProps) {
  const canJoin = code.trim().length === CODE_LENGTH && !busy;

  return (
    <div
      className={cn(
        // See defect 1: min-h-full centres short content and lets .main scroll
        // tall content. No overflow property here — that is the whole point.
        "flex min-h-full flex-col justify-center gap-6 px-5 py-8",
        // See the header note: the column stops growing past the design's own
        // largest intended window and centres instead.
        "mx-auto w-full max-w-256",
        className,
      )}
    >
      <div className="flex flex-col items-center gap-2 text-center">
        <h1 className="text-18 font-semibold text-text">Instant Voice</h1>
        <p className="text-14 text-text-dim">
          Create a room or enter a 4-digit code to start talking.
        </p>
      </div>

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
        <Panel className="flex min-w-0 flex-col gap-3">
          <div className="flex flex-col gap-1">
            <h2 className="text-15 font-semibold text-text">Host Call</h2>
            <p className="text-13 text-text-dim">
              Generate a 4-digit room code and share it.
            </p>
          </div>
          <Button
            variant="primary"
            size="lg"
            className="mt-auto w-full"
            onClick={onCreateRoom}
            disabled={busy}
          >
            Create Room
          </Button>
        </Panel>

        <Panel className="flex min-w-0 flex-col gap-3">
          <div className="flex flex-col gap-1">
            <h2 className="text-15 font-semibold text-text">Join Call</h2>
            <p className="text-13 text-text-dim">
              Enter the 4-digit room code shared by your host.
            </p>
          </div>
          <RoomCodeInput value={code} onChange={onCodeChange} length={CODE_LENGTH} />
          <Button
            variant="primary"
            size="lg"
            className="mt-auto w-full"
            onClick={onJoinRoom}
            disabled={!canJoin}
          >
            Join
          </Button>
        </Panel>
      </div>
    </div>
  );
}
