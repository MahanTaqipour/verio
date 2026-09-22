import { useId, type ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * Field — a labelled control, used across Settings.
 *
 * The label is wired to the control with a real `id`/`htmlFor` pair, so clicking
 * the label focuses the control and screen readers announce the pairing. A
 * caller can pass its own `id`; otherwise one is generated.
 *
 * `hint` is supporting text and is linked with `aria-describedby`, so it is
 * announced with the control rather than being decorative text that only sighted
 * users benefit from. `error` replaces the hint and sets `aria-invalid` — a
 * field is never shown as errored without the state being on the control.
 */

export interface FieldProps {
  label: ReactNode;
  /** The control. Receives the generated id via a render prop. */
  children: (props: {
    id: string;
    "aria-describedby": string | undefined;
    "aria-invalid": boolean | undefined;
  }) => ReactNode;
  hint?: ReactNode;
  error?: ReactNode;
  /** Lays the label and control out on one row, control right-aligned. */
  inline?: boolean;
  className?: string;
}

export function Field({
  label,
  children,
  hint,
  error,
  inline = false,
  className,
}: FieldProps) {
  const id = useId();
  const messageId = `${id}-message`;
  const message = error ?? hint;

  const labelEl = (
    <label
      htmlFor={id}
      className="label-caps shrink-0 text-text-dim"
    >
      {label}
    </label>
  );

  return (
    <div
      className={cn(
        inline ? "flex items-center justify-between gap-4" : "flex flex-col gap-2",
        className,
      )}
    >
      {labelEl}
      <div className={cn("flex flex-col gap-1", inline ? "min-w-0 flex-1" : "")}>
        {children({
          id,
          "aria-describedby": message ? messageId : undefined,
          "aria-invalid": error ? true : undefined,
        })}
        {message && (
          <p
            id={messageId}
            className={cn("text-13", error ? "text-danger" : "text-text-faint")}
          >
            {message}
          </p>
        )}
      </div>
    </div>
  );
}

/**
 * SettingsRow — label (with optional hint) on the left, control on the right.
 * This is the dominant layout in the Settings surface and in the peer cards.
 */
export interface SettingsRowProps {
  label: ReactNode;
  hint?: ReactNode;
  children?: ReactNode;
  className?: string;
}

export function SettingsRow({ label, hint, children, className }: SettingsRowProps) {
  return (
    <div className={cn("flex items-center justify-between gap-4", className)}>
      <div className="flex min-w-0 flex-col">
        <span className="text-14 text-text">{label}</span>
        {hint && <span className="text-13 text-text-faint">{hint}</span>}
      </div>
      {children}
    </div>
  );
}

/**
 * SectionLabel — the uppercase tracked heading used above every group.
 * A real heading element so it appears in the document outline; `level` picks
 * the tag without the caller having to think about the visual style.
 */
export interface SectionLabelProps {
  children: ReactNode;
  level?: 2 | 3 | 4;
  className?: string;
}

export function SectionLabel({ children, level = 3, className }: SectionLabelProps) {
  const Tag = `h${level}` as "h2" | "h3" | "h4";
  return (
    <Tag className={cn("label-caps text-text-dim", className)}>{children}</Tag>
  );
}
