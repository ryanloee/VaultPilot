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

// ── Agent status event (streamed during askWithAi / runAgent) ─────────────

export type AgentStatusEvent = {
  event: "agentStatus";
  payload: {
    stage: string;
    detail: string;
    timestamp: string;
  };
};
