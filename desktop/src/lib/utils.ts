/**
 * className combiner — minimal clsx replacement (no dependency).
 * Accepts strings, falsy values, and objects of {class: boolean}.
 */
export type ClassValue = string | false | null | undefined | { [key: string]: boolean | undefined | null };

export function cn(...inputs: ClassValue[]): string {
  const out: string[] = [];
  for (const v of inputs) {
    if (!v) continue;
    if (typeof v === "string") {
      out.push(v);
    } else if (typeof v === "object") {
      for (const [key, val] of Object.entries(v)) {
        if (val) out.push(key);
      }
    }
  }
  return out.join(" ");
}
