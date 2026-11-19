import type { HTMLAttributes, ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * Separator — design.md: a 1px rule in --border, never a shadow, never a
 * gradient. Used between settings sections and between list rows.
 *
 * `decorative` (the default) renders a plain div, which is correct when the
 * separation is purely visual. Set `decorative={false}` only when the rule
 * genuinely marks a boundary in the content structure, which is rare — a
 * separator announced to a screen reader usually adds noise.
 */

export interface SeparatorProps extends HTMLAttributes<HTMLDivElement> {
  orientation?: "horizontal" | "vertical";
  decorative?: boolean;
}

export function Separator({
  orientation = "horizontal",
  decorative = true,
  className,
  ...rest
}: SeparatorProps) {
  return (
    <div
      role={decorative ? "presentation" : "separator"}
      aria-orientation={decorative ? undefined : orientation}
      className={cn(
        "shrink-0 bg-border",
        orientation === "horizontal" ? "h-px w-full" : "h-full w-px",
        className,
      )}
      {...rest}
    />
  );
}

/**
 * Panel — the standard raised surface. design.md defines four surface tiers and
 * exactly one shadow, and the shadow is reserved for floating overlays. A panel
 * is a bordered --surface block; it never gets elevation.
 */
export interface PanelProps extends HTMLAttributes<HTMLDivElement> {
  /** Surface tier. `surface` is the card default; `surface-2` reads as nested. */
  tier?: "surface" | "surface-2" | "surface-3";
  /** Adds the 1px border. Off for panels that sit flush inside another panel. */
  bordered?: boolean;
  /** Enables the refracted Lavender/Violet glass gradient + specular bevel shadow. */
  glass?: boolean;
  padding?: "none" | "sm" | "md" | "lg";
  children?: ReactNode;
}

const TIERS = {
  surface: "bg-surface/90",
  "surface-2": "bg-surface-2/90",
  "surface-3": "bg-surface-3/95",
} as const;

const PADDING = {
  none: "",
  sm: "p-3",
  md: "p-4",
  lg: "p-5",
} as const;

export function Panel({
  tier = "surface",
  bordered = true,
  glass = false,
  padding = "md",
  className,
  children,
  ...rest
}: PanelProps) {
  return (
    <div
      className={cn(
        "rounded-2xl",
        glass ? "glass-card" : cn(TIERS[tier], bordered && "border border-border"),
        PADDING[padding],
        className,
      )}
      {...rest}
    >
      {children}
    </div>
  );
}
