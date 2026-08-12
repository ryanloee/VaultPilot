import { describe, it, expect } from "vitest";
import { cn, formatDate, toNumber, numberHeadings } from "./utils";

describe("cn (className combiner)", () => {
  it("joins string classes with a space", () => {
    expect(cn("a", "b", "c")).toBe("a b c");
  });

  it("skips falsy values", () => {
    expect(cn("a", "", false, null, undefined, "b")).toBe("a b");
  });

  it("includes object keys whose value is truthy", () => {
    expect(cn("base", { active: true, hidden: false })).toBe("base active");
  });

  it("returns empty string for no inputs", () => {
    expect(cn()).toBe("");
    expect(cn(false, null)).toBe("");
  });
});

describe("formatDate (ISO timestamp → MM/DD)", () => {
  it("formats a valid ISO date", () => {
    expect(formatDate("2026-08-12T10:00:00Z")).toMatch(/^\d{2}\/\d{2}$/);
  });

  it("returns empty string for null/undefined", () => {
    expect(formatDate()).toBe("");
    expect(formatDate(null)).toBe("");
  });

  it("returns empty string for invalid input", () => {
    expect(formatDate("not-a-date")).toBe("");
    expect(formatDate("")).toBe("");
  });
});

describe("toNumber (safe numeric conversion)", () => {
  it("passes through finite numbers", () => {
    expect(toNumber(42, 0)).toBe(42);
    expect(toNumber(-1.5, 0)).toBe(-1.5);
  });

  it("converts numeric strings", () => {
    expect(toNumber("10", 0)).toBe(10);
  });

  it("returns fallback for non-numeric input", () => {
    expect(toNumber("abc", 7)).toBe(7);
    expect(toNumber(NaN, 7)).toBe(7);
    expect(toNumber(undefined, 7)).toBe(7);
    expect(toNumber(null, 7)).toBe(7);
  });

  it("returns fallback for Infinity", () => {
    expect(toNumber(Infinity, 7)).toBe(7);
  });

  it("returns fallback for empty/whitespace strings (#4038)", () => {
    expect(toNumber("", 7)).toBe(7);
    expect(toNumber("   ", 7)).toBe(7);
    expect(toNumber("\t\n", 7)).toBe(7);
  });
});

describe("numberHeadings (render-layer heading numbering, #4062)", () => {
  it("numbers headings hierarchically: 1 / 1.1 / 1.1.2", () => {
    const md = [
      "# Title",
      "intro",
      "## Section A",
      "### Sub A1",
      "### Sub A2",
      "## Section B",
      "# Second part",
    ].join("\n");
    expect(numberHeadings(md)).toBe(
      [
        "# 1 Title",
        "intro",
        "## 1.1 Section A",
        "### 1.1.1 Sub A1",
        "### 1.1.2 Sub A2",
        "## 1.2 Section B",
        "# 2 Second part",
      ].join("\n")
    );
  });

  it("leaves non-heading lines untouched", () => {
    expect(numberHeadings("plain text\nnot # a heading\n")).toBe("plain text\nnot # a heading\n");
  });

  it("skips headings inside fenced code blocks", () => {
    const md = [
      "# Real heading",
      "```",
      "# fake heading inside code",
      "```",
      "## Next",
    ].join("\n");
    expect(numberHeadings(md)).toBe(
      ["# 1 Real heading", "```", "# fake heading inside code", "```", "## 1.1 Next"].join("\n")
    );
  });

  it("handles tilde fences", () => {
    const md = ["~~~", "## inside", "~~~", "## after"].join("\n");
    expect(numberHeadings(md)).toBe(["~~~", "## inside", "~~~", "## 1 after"].join("\n"));
  });
});
