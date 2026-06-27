import {
  isTabletWidth,
  gridColumnsForWidth,
  TABLET_BREAKPOINT,
} from '../../hooks/useIsTablet';

describe('isTabletWidth (issue #2121 responsive breakpoint)', () => {
  it('returns false on phone-sized widths', () => {
    expect(isTabletWidth(360)).toBe(false);
    expect(isTabletWidth(599)).toBe(false);
    expect(isTabletWidth(TABLET_BREAKPOINT - 1)).toBe(false);
  });

  it('returns true at and above the default tablet breakpoint', () => {
    expect(isTabletWidth(TABLET_BREAKPOINT)).toBe(true);
    expect(isTabletWidth(834)).toBe(true);
    expect(isTabletWidth(1024)).toBe(true);
    expect(isTabletWidth(1366)).toBe(true);
  });

  it('honours a custom threshold', () => {
    expect(isTabletWidth(500, 500)).toBe(true);
    expect(isTabletWidth(499, 500)).toBe(false);
    expect(isTabletWidth(600, 768)).toBe(false);
  });
});

describe('gridColumnsForWidth (Material responsive grid tiers)', () => {
  it('returns 1 column on phone widths', () => {
    expect(gridColumnsForWidth(320)).toBe(1);
    expect(gridColumnsForWidth(414)).toBe(1);
    expect(gridColumnsForWidth(599)).toBe(1);
  });

  it('returns 2 columns for small tablet / landscape widths', () => {
    expect(gridColumnsForWidth(600)).toBe(2);
    expect(gridColumnsForWidth(768)).toBe(2);
    expect(gridColumnsForWidth(834)).toBe(2);
    expect(gridColumnsForWidth(959)).toBe(2);
  });

  it('returns 3 columns for large tablet / desktop widths', () => {
    expect(gridColumnsForWidth(960)).toBe(3);
    expect(gridColumnsForWidth(1024)).toBe(3);
    expect(gridColumnsForWidth(1280)).toBe(3);
    expect(gridColumnsForWidth(1920)).toBe(3);
  });

  it('is monotonically non-decreasing across the width range', () => {
    let prev = 0;
    for (let w = 280; w <= 2000; w += 40) {
      const cols = gridColumnsForWidth(w);
      expect(cols).toBeGreaterThanOrEqual(prev);
      prev = cols;
    }
  });
});
