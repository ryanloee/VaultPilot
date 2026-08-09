import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  ChatState,
  ConversationSummary,
  ConversationTurn,
  NoteDocument,
  NoteMeta,
  RelatedNote,
} from "@/types";

/**
 * Type-safe wrapper around Tauri `invoke`. All backend calls go through here.
 * Command names are snake_case (Rust fn names); argument names are camelCase
 * (Tauri dispatches by camelCase argument names).
 */
export const api = {
  // ── system ──
  ping: () => invoke<boolean>("ping"),

  // ── settings ──
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) =>
    invoke<AppSettings>("save_settings", { settings }),

  // ── chat ──
  loadChatState: () => invoke<ChatState>("load_chat_state"),
  saveChatState: (chatState: ChatState) =>
    invoke<ChatState>("save_chat_state", { chatState }),
  listActions: () => invoke<unknown[]>("list_actions"),
  compressChatHistory: (
    summary: ConversationSummary | null,
    history: ConversationTurn[]
  ) =>
    invoke<ConversationSummary>("compress_chat_history", {
      summary,
      history,
    }),

  // ── notes ──
  listNotes: () => invoke<NoteMeta[]>("list_notes"),
  loadNote: (id: string) => invoke<NoteDocument>("load_note", { id }),
  saveNote: (note: NoteDocument) =>
    invoke<NoteDocument>("save_note", { note }),
  deleteNote: (id: string) => invoke<boolean>("delete_note", { id }),
  findRelatedNotes: (id: string, limit?: number) =>
    invoke<RelatedNote[]>("find_related_notes", { id, limit }),
  importMarkdown: (paths: string[]) =>
    invoke<unknown>("import_markdown", { paths }),
  rebuildIndex: () => invoke<unknown>("rebuild_index"),
  readImagePreview: (path: string) =>
    invoke<string>("read_image_preview", { path }),
  openVaultDirectory: (path: string) =>
    invoke<void>("open_vault_directory", { path }),

  // ── snapshots ──
  listSnapshots: (noteId: string) =>
    invoke<unknown[]>("list_snapshots", { noteId }),
  getSnapshot: (snapshotId: string) =>
    invoke<unknown>("get_snapshot", { snapshotId }),
  restoreSnapshot: (noteId: string, snapshotId: string) =>
    invoke<NoteDocument>("restore_snapshot", { noteId, snapshotId }),
} as const;
