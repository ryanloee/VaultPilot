import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  BacklinkEntry,
  ChatState,
  Collection,
  ConversationSummary,
  ConversationTurn,
  NoteDocument,
  NoteMeta,
  ProviderConnectionResult,
  RelatedNote,
  TriggerRule,
} from "@/types";
import { isTauri, mockApi } from "./mock";

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
 * The real Tauri-backed API. All commands go through `invoke`; command names
 * are snake_case (Rust fn names), argument names are camelCase.
 */
export const tauriApi = {
  // ── system ──
  ping: () => invoke<boolean>("ping"),

  // ── settings ──
  getSettings: () => invoke<AppSettings>("get_settings"),
  saveSettings: (settings: AppSettings) =>
    invoke<AppSettings>("save_settings", { settings }),
  testProviderConnection: (
    apiBase: string,
    apiKey: string,
    providerType: string,
    model?: string,
    timeoutMs?: number
  ) =>
    invoke<ProviderConnectionResult>("test_provider_connection", {
      apiBase,
      apiKey,
      providerType,
      model,
      timeoutMs,
    }),

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
  /**
   * Persist a base64 attachment and return its disk path (#4074).
   * `persistent = true` (image sends) writes into the vault's
   * `attachments/chat/` dir so history images survive temp-dir wipes; the
   * chat state then stores only the path, never the base64 blob (#4083).
   * Audio blobs stay in the (TTL-swept) OS temp dir.
   */
  saveTempAttachment: (
    dataBase64: string,
    filename: string,
    persistent = false
  ) =>
    invoke<string>("save_temp_attachment", { dataBase64, filename, persistent }),
  /** Transcribe an audio file via the active provider's Whisper endpoint (#4074). */
  transcribeAudio: (audioPath: string, language?: string) =>
    invoke<string>("transcribe_audio", { audioPath, language }),

  // ── notes ──
  listNotes: () => invoke<NoteMeta[]>("list_notes"),
  loadNote: (id: string) => invoke<NoteDocument>("load_note", { id }),
  saveNote: (note: NoteDocument) => invoke<NoteDocument>("save_note", { note }),
  deleteNote: (id: string) => invoke<boolean>("delete_note", { id }),
  findRelatedNotes: (id: string, limit?: number) =>
    invoke<RelatedNote[]>("find_related_notes", { id, limit }),
  findBacklinks: (id: string) => invoke<BacklinkEntry[]>("find_backlinks", { id }),
  importMarkdown: (paths: string[]) => invoke<unknown>("import_markdown", { paths }),
  rebuildIndex: () => invoke<unknown>("rebuild_index"),
  readImagePreview: (path: string) => invoke<string>("read_image_preview", { path }),
  openVaultDirectory: (path: string) => invoke<void>("open_vault_directory", { path }),

  // ── snapshots ──
  listSnapshots: (noteId: string) => invoke<unknown[]>("list_snapshots", { noteId }),
  getSnapshot: (snapshotId: string) => invoke<unknown>("get_snapshot", { snapshotId }),
  restoreSnapshot: (noteId: string, snapshotId: string) =>
    invoke<NoteDocument>("restore_snapshot", { noteId, snapshotId }),

  // ── collections ──
  listCollections: () => invoke<Collection[]>("list_collections"),
  createCollection: (name: string, description?: string, parentId?: string) =>
    invoke<Collection>("create_collection", { name, description, parentId }),
  renameCollection: (collectionId: string, name: string) =>
    invoke<boolean>("rename_collection", { collectionId, name }),
  moveCollection: (collectionId: string, newParentId?: string) =>
    invoke<boolean>("move_collection", { collectionId, newParentId }),
  deleteCollection: (collectionId: string) =>
    invoke<boolean>("delete_collection", { collectionId }),
  addNoteToCollection: (noteId: string, collectionId: string) =>
    invoke<boolean>("add_note_to_collection", { noteId, collectionId }),
  removeNoteFromCollection: (noteId: string, collectionId: string) =>
    invoke<boolean>("remove_note_from_collection", { noteId, collectionId }),
  listNotesInCollection: (
    collectionId: string,
    limit?: number,
    offset?: number
  ) =>
    invoke<{ notes: NoteMeta[] }>("list_notes_in_collection", {
      collectionId,
      limit,
      offset,
    }),
  getCollectionsForNote: (noteId: string) =>
    invoke<Collection[]>("get_collections_for_note", { noteId }),

  // ── triggers ──
  listTriggerRules: () => invoke<TriggerRule[]>("list_trigger_rules"),
  createTriggerRule: (
    label: string,
    triggerType: string,
    triggerConfig: string,
    action: string,
    filter?: string,
    customPrompt?: string
  ) =>
    invoke<TriggerRule>("create_trigger_rule", {
      label,
      triggerType,
      triggerConfig,
      action,
      filter,
      customPrompt,
    }),
  toggleTriggerRule: (ruleId: string) =>
    invoke<boolean>("toggle_trigger_rule", { ruleId }),
  deleteTriggerRule: (ruleId: string) =>
    invoke<boolean>("delete_trigger_rule", { ruleId }),
};

/**
 * Type-safe wrapper around the backend API. In a real Tauri shell it calls
 * `invoke`; in a plain browser (no Tauri bridge, e.g. browser-based UI
 * testing) it falls back to the in-memory mock so the UI can still be
 * exercised. Command names are snake_case; args camelCase.
 */
export const api = isTauri() ? tauriApi : mockApi;

/**
 * Subscribe to backend `agent-status` progress events (emitted during
 * ask_with_ai). Returns an unsubscribe function. In browser (mock) mode this
 * is a no-op so the real Tauri event listener isn't bound.
 */
export async function onAgentStatus(
  handler: (payload: AgentStatusPayload) => void
): Promise<UnlistenFn> {
  if (!isTauri()) {
    return () => {
      /* mock mode: no events */
    };
  }
  return listen<AgentStatusPayload>("agent-status", (event) => {
    handler(event.payload);
  });
}
