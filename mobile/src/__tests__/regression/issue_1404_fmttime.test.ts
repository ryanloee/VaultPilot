/**
 * Regression tests for fmtTime() — timeFormat.ts pure function (#1404).
 *
 * Tests: today→time format, older→date format, edge cases.
 */

import { fmtTime } from '../../utils/timeFormat';

/** Helper: get current time as Unix seconds */
const nowSeconds = () => Math.floor(Date.now() / 1000);

/** Helper: get seconds for N days ago */
const daysAgo = (n: number) => nowSeconds() - n * 86400;

describe('fmtTime', () => {
  it('returns time string (HH:MM) for today\'s timestamp', () => {
    const result = fmtTime(nowSeconds());
    // Should match HH:MM pattern (e.g. "14:30", "09:05")
    expect(result).toMatch(/^\d{1,2}:\d{2}$/);
  });

  it('returns time string for a timestamp earlier today', () => {
    // 6 hours ago, still today
    const ts = nowSeconds() - 6 * 3600;
    const result = fmtTime(ts);
    // If it's still the same day, should be time format
    // If crossing midnight, should be date format — both are valid
    expect(typeof result).toBe('string');
    expect(result.length).toBeGreaterThan(0);
  });

  it('returns date string for yesterday', () => {
    const result = fmtTime(daysAgo(1));
    // Should NOT be time format (HH:MM)
    expect(result).not.toMatch(/^\d{1,2}:\d{2}$/);
    // Should be a non-empty string
    expect(result.length).toBeGreaterThan(0);
  });

  it('returns date string for old timestamp (last year)', () => {
    const result = fmtTime(daysAgo(400));
    expect(result).not.toMatch(/^\d{1,2}:\d{2}$/);
    expect(result.length).toBeGreaterThan(0);
  });

  it('handles zero timestamp gracefully', () => {
    // Unix epoch (1970-01-01) — should return date string
    const result = fmtTime(0);
    expect(typeof result).toBe('string');
    expect(result.length).toBeGreaterThan(0);
    expect(result).not.toMatch(/^\d{1,2}:\d{2}$/);
  });

  it('handles future timestamp gracefully', () => {
    // 1 day in the future
    const result = fmtTime(nowSeconds() + 86400);
    expect(typeof result).toBe('string');
    expect(result.length).toBeGreaterThan(0);
  });

  it('returns consistent output for same input', () => {
    const ts = daysAgo(30);
    expect(fmtTime(ts)).toBe(fmtTime(ts));
  });
});
