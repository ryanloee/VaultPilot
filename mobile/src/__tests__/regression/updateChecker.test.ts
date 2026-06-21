/**
 * Unit tests for updateChecker.ts — compareSemver pure logic.
 */

import { compareSemver } from '../../utils/updateChecker';

describe('compareSemver', () => {
  it('returns 0 for equal versions', () => {
    expect(compareSemver('1.0.0', '1.0.0')).toBe(0);
    expect(compareSemver('0.3.32', '0.3.32')).toBe(0);
  });

  it('returns 1 when a > b (major)', () => {
    expect(compareSemver('2.0.0', '1.0.0')).toBe(1);
  });

  it('returns -1 when a < b (major)', () => {
    expect(compareSemver('1.0.0', '2.0.0')).toBe(-1);
  });

  it('returns 1 when a > b (minor)', () => {
    expect(compareSemver('1.2.0', '1.1.0')).toBe(1);
  });

  it('returns -1 when a < b (minor)', () => {
    expect(compareSemver('1.1.0', '1.2.0')).toBe(-1);
  });

  it('returns 1 when a > b (patch)', () => {
    expect(compareSemver('1.0.2', '1.0.1')).toBe(1);
  });

  it('returns -1 when a < b (patch)', () => {
    expect(compareSemver('1.0.1', '1.0.2')).toBe(-1);
  });

  it('handles missing patch version', () => {
    expect(compareSemver('1.0', '1.0.0')).toBe(0);
    expect(compareSemver('1.1', '1.0.9')).toBe(1);
  });

  it('handles missing minor and patch', () => {
    expect(compareSemver('1', '1.0.0')).toBe(0);
    expect(compareSemver('2', '1.9.9')).toBe(1);
  });
});
