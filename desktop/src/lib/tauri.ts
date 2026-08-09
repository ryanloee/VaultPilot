import { invoke } from "@tauri-apps/api/core";

/**
 * Type-safe wrapper around Tauri `invoke`. All backend calls go through here
 * so argument names stay consistent with the #[tauri::command] signatures
 * (Tauri dispatches by camelCase argument names).
 */
export const api = {
  ping: () => invoke<boolean>("ping"),

  // Reads (and initializes if absent) the persisted AppSettings. The returned
  // object mirrors vaultpilot_lib::models::AppSettings.
  getSettings: () => invoke<unknown>("get_settings"),
} as const;
