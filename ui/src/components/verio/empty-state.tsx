/**
 * EmptyState — the standard "there is nothing here yet" block.
 *
 * design.md forbids decorative illustration and ambient motion, so this is
 * deliberately text-only: a short title, an optional one-sentence explanation,
 * and an optional action. No artwork, no animation, no placeholder skeleton.
 *
 * Sizing follows the 860x640-first rule: the block is compact by default and the
 * *container* decides how much room it gets, so it reads correctly both in the
 * 280px diagnostics drawer and in the full-height room view.
 *
 * Copy rule: the title states the fact ("Queue is empty"), the description says
 * what to do about it. Do not use the description to restate the title.
 */

import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export interface EmptyStateProps {
  /** One short line. States the fact, e.g. "Queue is empty". */
  title: string;
  /** Optional single sentence on what to do next. */
  description?: ReactNode;
  /** Optional leading icon, rendered at 20px in the muted text colour. */
  icon?: ReactNode;
  /** Optional action, typically a Button. */
  action?: ReactNode;
  className?: string;
}

export function EmptyState({
  title,
  description,
  icon,
  action,
  className,
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center gap-2 px-5 py-8 text-center",
        className,
      )}
    >
      {icon ? <div className="text-text-faint">{icon}</div> : null}
      <p className="text-13 font-medium text-text">{title}</p>
      {description ? (
        <p className="max-w-60 text-11 leading-relaxed text-text-dim">
          {description}
        </p>
      ) : null}
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}
