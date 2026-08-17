export type Theme = "system" | "light" | "dark";

export const THEME_KEY = "vaultpilot.theme";

/** Resolve a theme mode to an actual dark/light decision. */
export function isDarkMode(mode: Theme): boolean {
  return (
    mode === "dark" ||
    (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches)
  );
}

/** Toggle Tailwind's `.dark` class on <html> for the given mode. */
export function applyTheme(mode: Theme): void {
  document.documentElement.classList.toggle("dark", isDarkMode(mode));
}

/** Read the persisted theme (null when unset/invalid). */
export function savedTheme(): Theme | null {
  try {
    const saved = localStorage.getItem(THEME_KEY);
    if (saved === "system" || saved === "light" || saved === "dark") return saved;
  } catch {
    /* ignore */
  }
  return null;
}

/** Apply the persisted theme (falling back to system), then persist `mode`. */
export function applyAndPersistTheme(mode: Theme): void {
  applyTheme(mode);
  try {
    localStorage.setItem(THEME_KEY, mode);
  } catch {
    /* ignore */
  }
}