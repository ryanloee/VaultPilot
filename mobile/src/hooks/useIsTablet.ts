import { useWindowDimensions } from 'react-native';

/**
 * Tablet / large-screen breakpoint (in dp).
 * 768dp matches a common tablet portrait width and aligns with the
 * Material Design responsive layout grid "small tablet" tier.
 */
export const TABLET_BREAKPOINT = 768;

/**
 * Responsive breakpoint hook.
 *
 * Returns `true` when the window width is >= `threshold`
 * (default {@link TABLET_BREAKPOINT}). Use this to branch layout between
 * phone (portrait, single column) and tablet / landscape (multi-column,
 * side-by-side) presentation.
 *
 * Implemented on top of `useWindowDimensions`, so the value updates
 * live on orientation / window-size changes and triggers a re-render.
 */
export function useIsTablet(threshold: number = TABLET_BREAKPOINT): boolean {
  const { width } = useWindowDimensions();
  return isTabletWidth(width, threshold);
}

/** Pure helper: is `width` (dp) in the tablet tier? Exposed for unit testing. */
export function isTabletWidth(width: number, threshold: number = TABLET_BREAKPOINT): boolean {
  return width >= threshold;
}

/**
 * Recommended number of columns for a responsive grid of note cards.
 *
 * - phone (`width < 600`): **1** column
 * - small tablet / landscape (`600 <= width < 960`): **2** columns
 * - large tablet / desktop (`width >= 960`): **3** columns
 *
 * Tiers follow the Material Design Responsive Layout Grid spec.
 */
export function useGridColumns(): number {
  const { width } = useWindowDimensions();
  return gridColumnsForWidth(width);
}

/** Pure helper: recommended grid column count for `width` (dp). Exposed for unit testing. */
export function gridColumnsForWidth(width: number): number {
  if (width >= 960) return 3;
  if (width >= 600) return 2;
  return 1;
}
