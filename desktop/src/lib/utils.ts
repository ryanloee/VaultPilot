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

/**
 * Auto-number markdown headings hierarchically (1 / 1.1 / 1.1.2…), matching
 * SiYuan v3.8.0 #522. Render-layer only: the returned string is a modified
 * copy — the original document is never touched (#4062).
 *
 * ATX headings (`#`…`######`) outside fenced code blocks are numbered in
 * document order; deeper levels reset when a shallower one appears. Lines
 * inside ``` fenced code blocks (including tildes) are left untouched so
 * code samples that look like headings are not renumbered.
 */
export function numberHeadings(markdown: string): string {
  const headingRe = /^(#{1,6})\s+(.+)$/;
  const fenceRe = /^\s*(```|~~~)/;
  const counters: number[] = [0, 0, 0, 0, 0, 0]; // per level 1..6
  let fence: string | null = null; // "```" or "~~~" while inside a block
  const lines = markdown.split("\n");
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fenceMatch = line.match(fenceRe);
    if (fenceMatch) {
      const marker = fenceMatch[1];
      if (!fence) {
        fence = marker;
      } else if (line.trim().startsWith(fence)) {
        fence = null;
      }
      continue; // fence lines are never numbered
    }
    if (fence) continue; // inside a fenced block — skip
    const m = line.match(headingRe);
    if (!m) continue;
    const level = m[1].length; // 1..6
    counters[level - 1] += 1;
    for (let l = level; l < 6; l++) counters[l] = 0; // reset deeper levels
    // Drop leading zero components so a document starting at H2 numbers
    // from "1" (Word-style) instead of "0.1".
    const raw = counters.slice(0, level);
    let start = raw.findIndex((c) => c > 0);
    if (start === -1) start = level - 1;
    const number = raw.slice(start).map((c) => String(c)).join(".");
    lines[i] = `${m[1]} ${number} ${m[2]}`;
  }
  return lines.join("\n");
}

