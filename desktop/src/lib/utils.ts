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

/** Format an ISO timestamp safely; returns "" for invalid/absent input. */
export function formatDate(iso?: string | null): string {
  if (!iso) return "";
  try {
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return "";
    return d.toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" });
  } catch {
    return "";
  }
}

/** Clamp a numeric-ish value to a safe integer (avoid NaN in inputs). */
export function toNumber(v: unknown, fallback: number): number {
  if (v === null || v === undefined) return fallback;
  if (typeof v === "string" && v.trim() === "") return fallback;
  const n = typeof v === "number" ? v : Number(v);
  return Number.isFinite(n) ? n : fallback;
}

