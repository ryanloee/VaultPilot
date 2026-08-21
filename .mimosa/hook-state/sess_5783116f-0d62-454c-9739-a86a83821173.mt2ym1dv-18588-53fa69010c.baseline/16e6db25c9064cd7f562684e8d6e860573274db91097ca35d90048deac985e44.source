import { describe, it, expect } from "vitest";
import { toCron, fromCron, detectPreset } from "./TriggerView";

// The cron backend evaluates in UTC while the picker collects local
// wall-clock time; toCron/fromCron convert between the two. These tests
// assert timezone-independent properties: valid expression shape and
// local→UTC→local round-trips (exact for constant-offset zones — UTC CI
// runners and UTC+8 dev machines both qualify).

describe("toCron / fromCron", () => {
  it("produces a 5-field cron expression", () => {
    const expr = toCron(30, 8, [1, 2, 3, 4, 5]);
    expect(expr.split(/\s+/)).toHaveLength(5);
    expect(expr).toMatch(/^\d+ \d+ \* \* [\d,]+$/);
  });

  it("daily selection collapses the weekday field to *", () => {
    const expr = toCron(0, 8, [0, 1, 2, 3, 4, 5, 6]);
    expect(expr).toMatch(/^\d+ \d+ \* \* \*$/);
  });

  it("round-trips every-day local time through UTC and back", () => {
    const expr = toCron(30, 8, [0, 1, 2, 3, 4, 5, 6]);
    const back = fromCron(expr);
    expect(back.hour).toBe(8);
    expect(back.minute).toBe(30);
    expect(back.days).toHaveLength(7);
  });

  it("round-trips a weekday set through UTC and back", () => {
    const days = [1, 3, 5];
    const back = fromCron(toCron(0, 9, days));
    expect(back.hour).toBe(9);
    expect(back.minute).toBe(0);
    expect([...back.days].sort((a, b) => a - b)).toEqual(days);
  });

  it("round-trips a cross-midnight time (weekdays may shift, come back)", () => {
    // 00:30 local in UTC+8 becomes 16:30 UTC on the previous weekday —
    // shifting there and back must restore the original local days.
    const days = [1, 2, 3, 4, 5];
    const back = fromCron(toCron(30, 0, days));
    expect(back.hour).toBe(0);
    expect(back.minute).toBe(30);
    expect([...back.days].sort((a, b) => a - b)).toEqual(days);
  });

  it("parses an explicit UTC expression back to local state", () => {
    // On a UTC machine this is the identity; elsewhere the time shifts but
    // stays internally consistent (same offset in fromCron).
    const back = fromCron("0 9 * * 1-5");
    expect(back.minute).toBe(0);
    expect(back.days).toHaveLength(5);
  });
});

describe("detectPreset", () => {
  it("classifies all seven days as every", () => {
    expect(detectPreset([0, 1, 2, 3, 4, 5, 6])).toBe("every");
  });

  it("classifies Mon-Fri as weekdays", () => {
    expect(detectPreset([1, 2, 3, 4, 5])).toBe("weekdays");
  });

  it("classifies Sun+Sat as weekends", () => {
    expect(detectPreset([0, 6])).toBe("weekends");
  });

  it("classifies an arbitrary subset as custom", () => {
    expect(detectPreset([1, 3, 5])).toBe("custom");
    expect(detectPreset([0])).toBe("custom");
  });
});
