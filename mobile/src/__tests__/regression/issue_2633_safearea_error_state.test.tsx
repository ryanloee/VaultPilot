// @ts-nocheck
/**
 * Regression test for #2633 — SafeAreaView rendered outside SafeAreaProvider
 * in App.tsx error state (notch overlap).
 *
 * The fix: wrap the error-state return in <SafeAreaProvider> so that
 * SafeAreaView has a valid ancestor and can provide correct safe-area insets.
 *
 * This test uses source scanning to verify the fix pattern is present,
 * avoiding the complex mock infrastructure needed to render App.tsx
 * (which imports navigation, safe-area-context, splash-screen, zustand,
 *  async-storage, db, API client, and 5 screen components).
 */
import fs from 'fs';
import path from 'path';

describe('#2633 — App.tsx error-state SafeAreaProvider wrapping', () => {
  const appPath = path.resolve(__dirname, '../../../App.tsx');

  it('error-state return contains SafeAreaProvider wrapping SafeAreaView', () => {
    const source = fs.readFileSync(appPath, 'utf-8');

    // Verify all key indicators exist
    expect(source).toContain("initState === 'error'");

    // Find the error-state block location
    const errorBlock = source.indexOf("if (initState === 'error')");
    expect(errorBlock).toBeGreaterThan(-1);

    // SafeAreaProvider should wrap SafeAreaView in the error block
    const providerOpen = source.indexOf('<SafeAreaProvider>', errorBlock);
    const safeAreaViewPos = source.indexOf('<SafeAreaView', errorBlock);
    const providerClose = source.indexOf('</SafeAreaProvider>', errorBlock);

    // SafeAreaProvider opens before SafeAreaView and closes after it
    expect(providerOpen).toBeGreaterThan(-1);
    expect(safeAreaViewPos).toBeGreaterThan(-1);
    expect(providerClose).toBeGreaterThan(-1);
    expect(providerOpen).toBeLessThan(safeAreaViewPos);
    expect(providerClose).toBeGreaterThan(safeAreaViewPos);

    // Also verify normal path has SafeAreaProvider (regression guard)
    const normalReturn = source.indexOf('return (', errorBlock + 100); // after error block
    const normalProviderOpen = source.indexOf('<SafeAreaProvider>', normalReturn);
    expect(normalProviderOpen).toBeGreaterThan(-1); // normal path has SafeAreaProvider too
  });
});
