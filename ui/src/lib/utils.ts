import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/**
 * Merge conditional class names and resolve Tailwind conflicts.
 *
 * `clsx` handles falsy/conditional values; `tailwind-merge` makes the last
 * utility in a conflicting group win, so a caller can override a component's
 * default padding with `className="p-4"` without `!important`.
 *
 * Standard shadcn/ui helper — every primitive and Verio component takes a
 * `className` prop and passes it through this.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
