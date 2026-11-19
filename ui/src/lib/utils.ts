import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

const twMerge = extendTailwindMerge({
  extend: {
    classGroups: {
      "font-size": [
        { text: ["10", "11", "13", "14", "15", "18", "24", "32"] },
      ],
    },
  },
});

/**
 * Merge conditional class names and resolve Tailwind conflicts.
 *
 * Extended with Verio's numeric `--text-*` font-size tokens so sizing classes
 * (e.g. `text-14`) never strip color utilities (e.g. `text-on-accent`) that
 * precede them in component variants.
 */
export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
