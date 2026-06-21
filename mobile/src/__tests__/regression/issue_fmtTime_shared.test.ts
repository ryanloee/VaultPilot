import { fmtTime } from '../../utils/timeFormat';

describe('fmtTime', () => {
  it('shows time for today\'s timestamp', () => {
    const now = Math.floor(Date.now() / 1000);
    const result = fmtTime(now);
    // Should contain a colon (time format like "14:30")
    expect(result).toMatch(/\d{1,2}:\d{2}/);
  });

  it('shows date for older timestamp', () => {
    // 2020-01-15T00:00:00Z — definitely not today
    const result = fmtTime(1579046400);
    // Should NOT contain a colon (date format like "1月15日")
    expect(result).not.toMatch(/\d{1,2}:\d{2}/);
  });

  it('handles zero timestamp (epoch)', () => {
    const result = fmtTime(0);
    expect(typeof result).toBe('string');
    expect(result.length).toBeGreaterThan(0);
  });

  it('handles future timestamp', () => {
    // 2030-01-01
    const result = fmtTime(1893456000);
    expect(typeof result).toBe('string');
  });
});
