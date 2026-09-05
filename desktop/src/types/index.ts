/**
 * TypeScript types mirroring `vaultpilot_lib::models` (Rust).
 * Field names are camelCase — Rust side uses `#[serde(rename_all = "camelCase")]`.
 *
 * Only the fields the desktop UI actually reads/writes are listed. Optional
 * Rust fields are marked `Optional` here; the wire format omits nulls.
 */

// ── Settings ──────────────────────────────────────────────────────────────

export type ProviderType = "openai" | "anthropic" | "ollama" | string;

export type ProviderConfig = {
  name: string;
  apiKey: string;
  baseUrl: string;
  model: string;
  requestTimeoutMs: number;
  contextWindowTokens?: number;
  maxOutputTokens?: number;
  providerType?: ProviderType;
};

export type ModelRoutingConfig = {
  enabled: boolean;
  simpleTaskModel?: string;
  complexTaskModel?: string;
  codeTaskModel?: string;
};

export type AppSettings = {
  vaultDir: string;
  provider: ProviderConfig;
  providers: ProviderConfig[];
  activeProviderIndex: number;
  autoCheckUpdates: boolean;
  autoWakeEnabled: boolean;
  autoWakeIntervalMinutes: number;
  autoWakeStartTime?: string;
  autoWakeEndTime?: string;
  autoWakeModel?: string;
  autoWakePrompt?: string;
  responseStyle?: string;
  contextCompression?: boolean;
  compressionThreshold?: number;
  modelRouting?: ModelRoutingConfig;
  proxyUrl?: string;
  systemDirective?: string;
  privacyMode?: boolean;
  /** Auto-number note headings in the renderer (1 / 1.1 / 1.1.2…) (#4062). */
  headingNumbering?: boolean;
  embeddingProvider?: string;
  /** Settings schema revision for one-time migrations — round-trip only. */
  settingsRevision?: number;
};

/** Mirrors vaultpilot_lib::ai::connectivity::ProviderConnectionResult (#3480). */
export type ProviderConnectionResult = {
  ok: boolean;
  status?: number;
  error?: string;
  probeUrl?: string;
  models?: string[];
  /** True/False when a real chat message was sent and accepted/rejected. */
  pingOk?: boolean;
  /** Error detail from the chat ping (quota/balance errors /models cannot catch). */
  pingError?: string;
  pingStatus?: number;
};

// ── Chat ──────────────────────────────────────────────────────────────────

export type ChatRole = "user" | "assistant" | "system";

/** Mirrors vaultpilot_lib::models::ChatTurn (fields: id, role, text). */
export type ChatTurn = {
  id?: string;
  role: ChatRole;
  text: string;
  citations?: unknown[];
  savedNote?: unknown;
  /** Image/attachment payloads attached to this turn (#4074). */
  attachments?: ChatAttachment[];
  /** Agent reasoning trace (summary + steps) for collapsible display. */
  thinking?: {
    summary?: string;
    steps?: { title: string; detail: string }[];
  };
  [key: string]: unknown;
};

/** Mirrors vaultpilot_lib::models::ConversationTurn (used in askWithAi params). */
export type ConversationTurn = {
  role: ChatRole;
  text: string;
};

export type ChatAttachment = {
  path?: string;
  type?: string;
  dataUrl?: string;
  name?: string;
};

export type ConversationSummary = {
  summary: string;
  createdAt?: string;
  [key: string]: unknown;
};

/** Mirrors vaultpilot_lib::models::ChatSession (turns, not messages). */
export type ChatSession = {
  id: string;
  title: string;
  turns: ChatTurn[];
  summary?: ConversationSummary;
  createdAt?: string;
  updatedAt?: string;
};

export type ChatState = {
  currentSessionId: string;
  sessions: ChatSession[];
};

// ── Notes ─────────────────────────────────────────────────────────────────

export type NoteMeta = {
  id: string;
  title: string;
  tags?: string[];
  createdAt?: string;
  updatedAt?: string;
  summary?: string;
  [key: string]: unknown;
};

/** Mirrors vaultpilot_lib::models::Collection (#2042). */
export type Collection = {
  id: string;
  name: string;
  description?: string;
  createdAt?: string;
  updatedAt?: string;
  /** Empty string = root collection. */
  parentId?: string;
  noteCount?: number;
};

export type NoteDocument = {
  meta: NoteMeta;
  body: string;
  searchSnippet?: string;
  searchScore?: number;
};

export type RelatedNote = {
  meta: NoteMeta;
  score: number;
  snippet?: string;
};

/** A note that links **to** a given target note (`[[Title]]` wikilink) (#4061). */
export type BacklinkEntry = {
  meta: NoteMeta;
  linkTarget: string;
};

// ── Trigger Rules (定时唤醒) ──────────────────────────────────────────────

export type TriggerRule = {
  id: string;
  label: string;
  triggerType: "cron" | "event";
  triggerConfig: string;
  filter?: string;
  action: string;
  enabled: boolean;
  customPrompt?: string;
  /** Provider name from settings.providers — null = use active provider. */
  providerName?: string;
  /** Scheduler status — answers "did it fire, and did it work?" */
  lastFiredAt?: string;
  nextFireAt?: string;
  runCount?: number;
  lastStatus?: "success" | "failed" | string;
  lastError?: string;
};

/** One row of the trigger execution log (newest first). */
export type TriggerExecution = {
  id: string;
  ruleId: string;
  label: string;
  action: string;
  /** RFC3339 timestamp. */
  firedAt: string;
  status: "success" | "failed" | string;
  error: string;
  detail: string;
  /** Full AI answer text — stored inline in the DB, NOT as a vault note. */
  resultContent: string;
};

// ── Agent status event (streamed during askWithAi / runAgent) ─────────────

export type AgentStatusEvent = {
  event: "agentStatus";
  payload: {
    stage: string;
    detail: string;
    timestamp: string;
  };
};

// ── Feed Subscriptions (订阅源) ──────────────────────────────────────────

/** Mirrors vaultpilot_lib::models::FeedSubscription (camelCase on the wire). */
export type FeedSubscription = {
  id: string;
  title: string;
  url: string;
  kind: string;
  collection: string;
  tags: string;
  intervalMinutes: number;
  enabled: boolean;
  lastFetchedAt: string;
  etag: string;
  lastModified: string;
  lastEntryId: string;
  lastEntryDate: string;
  /** "success" | "failed" | "skipped" | "" (never polled). */
  lastStatus: string;
  lastError: string;
  createdAt: string;
  updatedAt: string;
};

/** Mirrors vaultpilot_lib::feed_ingest::FeedPollResult. */
export type FeedPollResult = {
  feedId: string;
  status: string;
  newEntries: number;
  error: string;
};

// ── Mail Accounts (邮件导入, desktop only) ───────────────────────────────

/** Mirrors the Tauri MailAccountDto — no password field, ever. */
export type MailAccount = {
  id: string;
  name: string;
  host: string;
  port: number;
  username: string;
  useTls: boolean;
  syncEnabled: boolean;
  syncFrequencyMinutes: number;
  lastSyncAt: string;
  createdAt: string;
  updatedAt: string;
};

/** Mirrors vaultpilot_lib::mail::SyncResult. */
export type MailSyncResult = {
  accountId: string;
  fetched: number;
  imported: number;
  skippedDuplicates: number;
  errors: string[];
};

/** Mirrors vaultpilot_lib::mail::StoredEmail. */
export type StoredEmail = {
  id: string;
  accountId: string;
  messageId: string;
  subject: string;
  fromAddr: string;
  toAddrs: string;
  ccAddrs: string;
  date: string;
  bodyText: string;
  noteId: string;
  importedAt: string;
};

// ── Connectors & MCP (集成) ─────────────────────────────────────────────

/** One row of the `connector_catalog()` (webhook / github / slack / email). */
export type ConnectorInfo = {
  connectorType: string;
  label: string;
  phase: number;
  auth: string;
  capabilities: [string, string][];
  usage: string;
};
