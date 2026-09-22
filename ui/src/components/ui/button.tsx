import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import { IconSpinner } from "@/components/verio/icons";

/**
 * Button — design.md §"Focus and interaction states".
 *
 * Rules this encodes:
 *   - accent fill is reserved for primary actions only
 *   - one step brighter on hover, no glow
 *   - 1px vertical translate on press (no scale, no bounce)
 *   - disabled is --text-faint at 50% with cursor not-allowed
 *   - focus-visible is handled globally in the base layer (2px accent outline)
 *   - radii come from --r-sm (5px); no shadows on buttons
 */

export type ButtonVariant =
  | "primary"
  | "secondary"
  | "ghost"
  | "danger"
  | "danger-outline";

export type ButtonSize = "sm" | "md" | "lg";

const VARIANTS: Record<ButtonVariant, string> = {
  primary:
    "bg-accent text-bg font-medium hover:bg-accent-hi active:bg-accent-lo",
  secondary:
    "bg-surface-2 text-text border border-border hover:bg-surface-3 hover:border-border-2 active:bg-surface-3",
  ghost:
    "bg-transparent text-text-dim hover:bg-surface-2 hover:text-text active:bg-surface-3",
  danger:
    "bg-danger text-text font-medium hover:brightness-110 active:brightness-95",
  "danger-outline":
    "bg-transparent text-danger border border-danger/60 hover:bg-danger/10 hover:border-danger active:bg-danger/20",
};

const SIZES: Record<ButtonSize, string> = {
  sm: "h-7 px-3 text-13 gap-1.5",
  md: "h-8 px-4 text-13 gap-2",
  lg: "h-10 px-5 text-14 gap-2",
};

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  /** Shows a spinner and blocks interaction without changing layout. */
  loading?: boolean;
  /** Rendered before the label. */
  icon?: ReactNode;
  /** Rendered after the label. */
  trailingIcon?: ReactNode;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  {
    variant = "secondary",
    size = "md",
    loading = false,
    icon,
    trailingIcon,
    className,
    children,
    disabled,
    type = "button",
    ...rest
  },
  ref,
) {
  const isDisabled = disabled || loading;
  return (
    <button
      ref={ref}
      type={type}
      disabled={isDisabled}
      aria-busy={loading || undefined}
      className={cn(
        "inline-flex select-none items-center justify-center rounded-sm whitespace-nowrap",
        "transition-[background-color,border-color,color,transform] ease-verio",
        "press",
        VARIANTS[variant],
        SIZES[size],
        isDisabled &&
          "pointer-events-none cursor-not-allowed text-text-faint opacity-50",
        className,
      )}
      {...rest}
    >
      {loading ? <IconSpinner size={size === "lg" ? 16 : 14} /> : icon}
      {children}
      {!loading && trailingIcon}
    </button>
  );
});

/**
 * IconButton — square, icon-only control. design.md: 16px icon in buttons.
 *
 * `label` is required and becomes the accessible name; the icon itself is
 * decorative. This is the only sanctioned way to render an icon-only control.
 */
export interface IconButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "aria-label"> {
  /** Accessible name. Required — an icon-only button with no name is a bug. */
  label: string;
  variant?: ButtonVariant;
  size?: "sm" | "md" | "lg";
  /** Renders the icon as filled — only for filled states (e.g. muted). */
  filled?: boolean;
  /** The icon. Pass its `size` explicitly (14 sm / 16 md / 20 lg). */
  children: ReactNode;
}

const ICON_BUTTON_SIZES = {
  sm: "h-7 w-7",
  md: "h-8 w-8",
  lg: "h-10 w-10",
} as const;

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(
  function IconButton(
    { label, variant = "ghost", size = "md", className, children, type = "button", ...rest },
    ref,
  ) {
    return (
      <button
        ref={ref}
        type={type}
        aria-label={label}
        title={label}
        className={cn(
          "inline-flex shrink-0 select-none items-center justify-center rounded-sm",
          "transition-[background-color,border-color,color,transform] ease-verio",
          "press",
          VARIANTS[variant],
          ICON_BUTTON_SIZES[size],
          className,
        )}
        {...rest}
      >
        {children}
      </button>
    );
  },
);
