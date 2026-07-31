import { I18n } from "i18n-js";
import * as Localization from "expo-localization";
import AsyncStorage from "@react-native-async-storage/async-storage";

import en from "./locales/en.json";
import zhCN from "./locales/zh-CN.json";

const i18n = new I18n({
  en,
  "zh-CN": zhCN,
});

i18n.enableFallback = true;
i18n.defaultLocale = "zh-CN";

// Store the user's explicit language choice key
const USER_LANG_KEY = "@vaultpilot:user-locale";

/**
 * Initialize locale: respect user's explicit choice, else follow system.
 * Should be called once at app startup, before rendering UI.
 */
export async function initLocale(): Promise<void> {
  try {
    const stored = await AsyncStorage.getItem(USER_LANG_KEY);
    if (stored && (stored === "en" || stored === "zh-CN")) {
      i18n.locale = stored;
      return;
    }
  } catch {
    // AsyncStorage unavailable — use system locale
  }

  // Follow system locale
  const locales = Localization.getLocales();
  const systemLang = locales[0]?.languageCode ?? "zh";
  if (systemLang.startsWith("zh")) {
    i18n.locale = "zh-CN";
  } else {
    i18n.locale = "en";
  }
}

/**
 * Switch locale and persist the user's choice.
 * Call when user changes language in settings.
 */
export async function setLocale(locale: "en" | "zh-CN"): Promise<void> {
  i18n.locale = locale;
  try {
    await AsyncStorage.setItem(USER_LANG_KEY, locale);
  } catch {
    // Non-critical — setting just won't persist across restarts
  }
}

/**
 * Get the current locale code.
 */
export function getCurrentLocale(): string {
  return i18n.locale;
}

/**
 * Translate a key. Alias for i18n.t().
 */
export function t(key: string, options?: Record<string, unknown>): string {
  return i18n.t(key, options);
}

export default i18n;