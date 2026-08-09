import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  ChatState,
  ConversationSummary,
  ConversationTurn,
  NoteDocument,
  NoteMeta,
  RelatedNote,
} from "@/types";

/** AI answer result (mirrors vaultpilot_lib::models::GroundedAnswer). */
export type GroundedAnswer = {
  answer: string;
  citations?: unknown[];
  savedNote?: NoteMeta;
  usedContextCount: number;
  [key: string]: unknown;
};

/** Progress payload emitted by the backend during ask_with_ai. */
export type AgentStatusPayload = {
  stage: string;
  detail: string;
  timestamp: string;
};

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
  ) => invoke<ConversationSummary>("compress_chat_history", { summary, history }),
  askWithAi: (
    question: string,
    history: ConversationTurn[] | null,
    imagePaths: string[] | null,
    modelOverride: string | null
  ) =>
    invoke<GroundedAnswer>("ask_with_ai", {
      question,
      history,
      imagePaths,
      modelOverride,
    }),

  // ── notes ──
  listNotes: () => invoke<NoteMeta[]>("list_notes"),
  loadNote: (id: string) => invoke<NoteDocument>("load_note", { id }),
  saveNote: (note: NoteDocument) => invoke<NoteDocument>("save_note", { note }),
  deleteNote: (id: string) => invoke<boolean>("delete_note", { id }),
  findRelatedNotes: (id: string, limit?: number) =>
    invoke<RelatedNote[]>("find_related_notes", { id, limit }),
  importMarkdown: (paths: string[]) => invoke<unknown>("import_markdown", { paths }),
  rebuildIndex: () => invoke<unknown>("rebuild_index"),
  readImagePreview: (path: string) => invoke<string>("read_image_preview", { path }),
  openVaultDirectory: (path: string) => invoke<void>("open_vault_directory", { path }),

  // ── snapshots ──
  listSnapshots: (noteId: string) => invoke<unknown[]>("list_snapshots", { noteId }),
  getSnapshot: (snapshotId: string) => invoke<unknown>("get_snapshot", { snapshotId }),
  restoreSnapshot: (noteId: string, snapshotId: string) =>
    invoke<NoteDocument>("restore_snapshot", { noteId, snapshotId }),
} as const;

/**
 * Subscribe to backend `agent-status` progress events (emitted during
 * ask_with_ai). Returns an unsubscribe function.
 */
export async function onAgentStatus(
  handler: (payload: AgentStatusPayload) => void
): Promise<UnlistenFn> {
  return listen<AgentStatusPayload>("agent-status", (event) => {
    handler(event.payload);
  });
}
