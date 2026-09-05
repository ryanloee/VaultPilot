import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  BacklinkEntry,
  ChatState,
  Collection,
  ConversationSummary,
  ConversationTurn,
  FeedPollResult,
  FeedSubscription,
  MailAccount,
  MailSyncResult,
  NoteDocument,
  NoteMeta,
  ProviderConnectionResult,
  RelatedNote,
  StoredEmail,
  TriggerExecution,
  TriggerRule,
} from "@/types";
import { isTauri, mockApi } from "./mock";

/** AI answer result (mirrors vaultpilot_lib::models::GroundedAnswer). */
export type GroundedAnswer = {
  answer: string;
  citations?: unknown[];
  savedNote?: NoteMeta;
  usedContextCount: number;
  /** Provider-reported token usage for the whole answer pipeline. */
  usageInputTokens?: number;
  usageOutputTokens?: number;
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
  isDesktop: () => invoke<boolean>("is_desktop"),
  openExternalUrl: (url: string) => invoke<void>("open_external_url", { url }),

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
  vaultSyncStatus: () =>
    invoke<{
      disk_files: number;
      indexed_notes: number;
      needs_rebuild: boolean;
      latest_disk_mtime: string;
    }>("vault_sync_status"),
  discoverDevice: (ip: string) =>
    invoke<{
      hostname: string;
      platform: string;
      vaultPilotVersion: string;
      noteCount: number;
      vaultName: string;
    } | null>("discover_device", { ip }),
  scanLanDevices: () =>
    invoke<
      Array<{
        ip: string;
        hostname: string;
        platform: string;
        vaultPilotVersion: string;
        noteCount: number;
        vaultName: string;
      }>
    >("scan_lan_devices"),
  // ── sync pairing & transfer ──
  generatePairCode: () => invoke<string>("generate_pair_code"),
  regeneratePairCode: () => invoke<string>("regenerate_pair_code"),
  listSyncPeers: () =>
    invoke<
      Array<{
        deviceId: string;
        hostname: string;
        platform: string;
        token: string;
        ip: string | null;
        addedAt: string;
        lastSyncAt: string | null;
      }>
    >("list_sync_peers"),
  removeSyncPeer: (deviceId: string) =>
    invoke<void>("remove_sync_peer", { deviceId }),
  completePairing: (ip: string, pairCode: string) =>
    invoke<{
      deviceId: string;
      hostname: string;
      platform: string;
      token: string;
      ip: string | null;
      addedAt: string;
      lastSyncAt: string | null;
    }>("complete_pairing", { ip, pairCode }),
  syncWithPeer: (
    ip: string,
    deviceId: string,
    mode?: "full" | "selected",
    includes?: string[]
  ) =>
    invoke<{
      pulled: number;
      pushed: number;
      conflicts: number;
      errors: string[];
    }>("sync_with_peer", { ip, deviceId, mode, includes }),
  getPeerManifest: (ip: string) =>
    invoke<
      Array<{ path: string; sha256: string; mtimeMs: number }>
    >("get_peer_manifest", { ip }),
  listLocalManifest: () =>
    invoke<
      Array<{ path: string; sha256: string; mtimeMs: number }>
    >("list_local_manifest"),
  syncSelected: (
    ip: string,
    deviceId: string,
    pull?: string[],
    push?: string[]
  ) =>
    invoke<{
      pulled: number;
      pushed: number;
      conflicts: number;
      errors: string[];
    }>("sync_selected", { ip, deviceId, pull, push }),
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
  listTriggerExecutions: (limit?: number) =>
    invoke<TriggerExecution[]>("list_trigger_executions", { limit }),
  deleteTriggerExecution: (executionId: string) =>
    invoke<boolean>("delete_trigger_execution", { executionId }),
  clearTriggerExecutions: () =>
    invoke<number>("clear_trigger_executions"),
  createTriggerRule: (
    label: string,
    triggerType: string,
    triggerConfig: string,
    action: string,
    filter?: string,
    customPrompt?: string,
    providerName?: string
  ) =>
    invoke<TriggerRule>("create_trigger_rule", {
      label,
      triggerType,
      triggerConfig,
      action,
      filter,
      customPrompt,
      providerName,
    }),
  toggleTriggerRule: (ruleId: string) =>
    invoke<boolean>("toggle_trigger_rule", { ruleId }),
  deleteTriggerRule: (ruleId: string) =>
    invoke<boolean>("delete_trigger_rule", { ruleId }),
  fireTriggerRuleNow: (ruleId: string) =>
    invoke<{ success: boolean; error: string | null; detail: string | null }>(
      "fire_trigger_rule_now",
      { ruleId }
    ),
  updateTriggerRule: (
    ruleId: string,
    label: string,
    triggerType: string,
    triggerConfig: string,
    action: string,
    filter?: string,
    customPrompt?: string,
    providerName?: string
  ) =>
    invoke<TriggerRule>("update_trigger_rule", {
      ruleId,
      label,
      triggerType,
      triggerConfig,
      action,
      filter,
      customPrompt,
      providerName,
    }),

  // ── feeds (RSS/Atom/JSON subscriptions) ──
  listFeeds: () => invoke<FeedSubscription[]>("list_feeds"),
  addFeed: (
    url: string,
    title: string,
    kind: string,
    collection: string,
    tags: string,
    intervalMinutes: number
  ) =>
    invoke<FeedSubscription>("add_feed", {
      url,
      title,
      kind,
      collection,
      tags,
      intervalMinutes,
    }),
  updateFeed: (
    id: string,
    title: string,
    kind: string,
    collection: string,
    tags: string,
    intervalMinutes: number,
    enabled: boolean
  ) =>
    invoke<boolean>("update_feed", {
      id,
      title,
      kind,
      collection,
      tags,
      intervalMinutes,
      enabled,
    }),
  removeFeed: (id: string) => invoke<boolean>("remove_feed", { id }),
  setFeedEnabled: (id: string, enabled: boolean) =>
    invoke<boolean>("set_feed_enabled", { id, enabled }),
  /** Fetch all enabled feeds now; each feed reports its own status. */
  refreshFeeds: () => invoke<FeedPollResult[]>("refresh_feeds"),
  /** Fetch a single feed now. */
  refreshFeed: (id: string) => invoke<FeedPollResult>("refresh_feed", { id }),

  // ── mail (IMAP-to-vault; desktop only) ──
  listMailAccounts: () => invoke<MailAccount[]>("list_mail_accounts"),
  addMailAccount: (
    name: string,
    host: string,
    port: number,
    username: string,
    password: string,
    useTls: boolean,
    syncFrequencyMinutes: number
  ) =>
    invoke<MailAccount>("add_mail_account", {
      name,
      host,
      port,
      username,
      password,
      useTls,
      syncFrequencyMinutes,
    }),
  deleteMailAccount: (id: string) =>
    invoke<boolean>("delete_mail_account", { id }),
  syncMailAccount: (id: string) =>
    invoke<MailSyncResult>("sync_mail_account", { id }),
  searchEmails: (query: string, limit?: number, offset?: number) =>
    invoke<StoredEmail[]>("search_emails", { query, limit, offset }),
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

/** Payload pushed by the sync server on the *acceptor* side. */
export type SyncPairingEventPayload = {
  accepted?: { hostname: string; platform: string };
  rejected?: { reason: string };
};

/**
 * Subscribe to `sync-pairing` events — emitted on the device that *receives* a
 * pairing handshake (the acceptor). Lets the desktop show a prompt when another
 * device pairs with it, instead of staying silent. No-op in mock mode.
 */
export async function onSyncPairing(
  handler: (payload: SyncPairingEventPayload) => void
): Promise<UnlistenFn> {
  if (!isTauri()) {
    return () => {
      /* mock mode: no events */
    };
  }
  return listen<SyncPairingEventPayload>("sync-pairing", (event) => {
    handler(event.payload);
  });
}
