/**
 * Shared time formatting utilities.
 *
 * Extracted from NotesScreen and SessionsScreen to eliminate duplication.
 */

/**
 * Format a Unix timestamp (seconds) for display in lists.
 * - Today: shows time (e.g. "14:30")
 * - Older: shows date (e.g. "6月21日")
 */
export function fmtTime(ts: number): string {
  const d = new Date(ts * 1000);
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
  }
  return d.toLocaleDateString('zh-CN', { month: 'short', day: 'numeric' });
}
