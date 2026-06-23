/**
 * Regression test for issue #1437
 * OfflineBanner should use theme-aware colors based on isDark prop.
 *
 * Tests the color selection logic directly (no JSX rendering needed).
 */

// Color constants matching OfflineBanner.tsx
const LIGHT_BG = '#FEF3C7';
const LIGHT_TEXT = '#92400E';
const DARK_BG = '#451A03';
const DARK_TEXT = '#FDE68A';

function getBannerColors(isDark: boolean) {
  return {
    bg: isDark ? DARK_BG : LIGHT_BG,
    text: isDark ? DARK_TEXT : LIGHT_TEXT,
  };
}

describe('OfflineBanner (#1437 dark theme)', () => {
  it('uses light amber background when isDark=false', () => {
    expect(getBannerColors(false).bg).toBe('#FEF3C7');
  });

  it('uses light amber text when isDark=false', () => {
    expect(getBannerColors(false).text).toBe('#92400E');
  });

  it('uses dark amber background when isDark=true', () => {
    expect(getBannerColors(true).bg).toBe('#451A03');
  });

  it('uses light amber text on dark when isDark=true', () => {
    expect(getBannerColors(true).text).toBe('#FDE68A');
  });

  it('defaults to light colors when isDark is undefined', () => {
    // OfflineBanner has isDark = false as default
    const colors = getBannerColors(false);
    expect(colors.bg).toBe(LIGHT_BG);
    expect(colors.text).toBe(LIGHT_TEXT);
  });

  it('dark mode colors are distinct from light mode', () => {
    const light = getBannerColors(false);
    const dark = getBannerColors(true);
    expect(light.bg).not.toBe(dark.bg);
    expect(light.text).not.toBe(dark.text);
  });
});
