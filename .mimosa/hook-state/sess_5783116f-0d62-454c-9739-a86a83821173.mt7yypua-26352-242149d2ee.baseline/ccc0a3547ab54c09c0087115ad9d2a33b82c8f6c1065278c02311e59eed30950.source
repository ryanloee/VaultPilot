import type {
  AppSettings,
  BacklinkEntry,
  ChatState,
  Collection,
  NoteDocument,
  NoteMeta,
  ConversationSummary,
  ConversationTurn,
  ProviderConnectionResult,
  TriggerExecution,
  TriggerRule,
} from "@/types";

/** Local copy of the answer type to avoid importing from tauri.ts (which
 * depends on the Tauri bridge module). */
export type GroundedAnswer = {
  answer: string;
  citations?: unknown[];
  savedNote?: NoteMeta;
  usedContextCount: number;
  [key: string]: unknown;
};

/**
 * Mock implementation of the backend API for browser-based UI testing.
 *
 * When the app runs in a plain browser (no Tauri bridge), `invoke` is
 * unavailable. This module provides in-memory fake data so the UI's layout and
 * interaction logic can still be exercised with the browser automation tool.
 * It is never used in the real Tauri shell — only when `isTauri()` is false.
 */

function uuid() {
  return crypto.randomUUID();
}

const now = () => new Date().toISOString();

// ── in-memory data ─────────────────────────────────────────────────────────

let settings: AppSettings = {
  vaultDir: "C:/Users/test/Documents/VaultPilotVault",
  provider: {
    name: "Mock Provider",
    apiKey: "sk-mock",
    baseUrl: "https://api.mock.com/v1",
    model: "mock-model",
    requestTimeoutMs: 60000,
    contextWindowTokens: 200000,
    providerType: "openai",
  },
  providers: [
    {
      name: "Mock Provider",
      apiKey: "sk-mock",
      baseUrl: "https://api.mock.com/v1",
      model: "mock-model",
      requestTimeoutMs: 60000,
      contextWindowTokens: 200000,
      providerType: "openai",
    },
  ],
  activeProviderIndex: 0,
  autoCheckUpdates: true,
  autoWakeEnabled: false,
  autoWakeIntervalMinutes: 30,
  proxyUrl: "",
  systemDirective: "",
};

let chatState: ChatState = {
  currentSessionId: "",
  sessions: [
    {
      id: "mock-session-1",
      title: "示例会话：AI 入门",
      turns: [
        { id: uuid(), role: "user", text: "你好，介绍一下这个应用" },
        {
          id: uuid(),
          role: "assistant",
          text: "这是一个本地优先的知识库笔记应用，支持 AI 对话和笔记管理。",
        },
      ],
      createdAt: now(),
      updatedAt: now(),
    },
    {
      id: "mock-session-2",
      title: "空会话",
      turns: [],
      createdAt: now(),
      updatedAt: now(),
    },
  ],
};
chatState.currentSessionId = chatState.sessions[0].id;

let notes: NoteMeta[] = [
  {
    id: "note-1",
    title: "欢迎使用 VaultPilot",
    tags: ["入门", "笔记"],
    createdAt: now(),
    updatedAt: now(),
    summary: "这是一个欢迎笔记，介绍基本用法。",
  },
  {
    id: "note-2",
    title: "Markdown 语法速查",
    tags: ["markdown"],
    createdAt: now(),
    updatedAt: now(),
    summary: "标题、列表、代码块、表格的写法。",
  },
];

let noteDocs: Record<string, NoteDocument> = {
  "note-1": {
    meta: notes[0],
    body: "# 欢迎使用 VaultPilot\n\n这是一个 **本地优先** 的知识库笔记应用。\n\n- 支持 AI 对话\n- 支持笔记管理\n\n```ts\nconsole.log(\"hello\")\n```",
  },
  "note-2": {
    meta: notes[1],
    body: "# Markdown 语法\n\n## 列表\n\n- 项目一\n- 项目二\n\n## 代码\n\n```rust\nfn main() {}\n```",
  },
};

let mockCollections: Collection[] = [
  {
    id: "col-1",
    name: "入门",
    description: "",
    createdAt: now(),
    updatedAt: now(),
    parentId: "",
    noteCount: 1,
  },
  {
    id: "col-2",
    name: "参考",
    description: "",
    createdAt: now(),
    updatedAt: now(),
    parentId: "",
    noteCount: 1,
  },
  {
    id: "col-3",
    name: "Markdown",
    description: "",
    createdAt: now(),
    updatedAt: now(),
    parentId: "col-2",
    noteCount: 0,
  },
];

// ── mock API surface (mirrors the real api object) ────────────────────────

export const mockApi = {
  ping: async (): Promise<boolean> => true,
  isDesktop: async (): Promise<boolean> => false,

  getSettings: async (): Promise<AppSettings> => JSON.parse(JSON.stringify(settings)),
  saveSettings: async (s: AppSettings): Promise<AppSettings> => {
    settings = JSON.parse(JSON.stringify(s));
    return settings;
  },
  testProviderConnection: async (
    apiBase: string,
    _apiKey: string,
    _providerType: string,
    _model?: string
  ): Promise<ProviderConnectionResult> => {
    // Mock mode: pretend the endpoint is reachable with a canned model list.
    return {
      ok: true,
      status: 200,
      probeUrl: `${apiBase.replace(/\/$/, "")}/models`,
      models: ["mock-model"],
      pingOk: true,
      pingStatus: 200,
    };
  },

  loadChatState: async (): Promise<ChatState> => JSON.parse(JSON.stringify(chatState)),
  saveChatState: async (c: ChatState): Promise<ChatState> => {
    chatState = JSON.parse(JSON.stringify(c));
    return chatState;
  },
  askWithAi: async (
    question: string,
    _history: ConversationTurn[] | null,
    _imagePaths: string[] | null,
    _modelOverride: string | null
  ): Promise<GroundedAnswer> => {
    // Simulate latency + a canned answer.
    await new Promise((r) => setTimeout(r, 800));
    return {
      answer: `这是对「${question}」的模拟回复（Mock 模式）。\n\n- 第一点说明\n- 第二点说明\n\n> 由浏览器测试模式生成`,
      usedContextCount: 0,
    };
  },
  saveTempAttachment: async (
    _dataBase64: string,
    filename: string,
    _persistent = false
  ): Promise<string> => {
    // Mock mode: no real temp file; return a fake path so the UI flow works.
    return `/tmp/mock-attachments/${filename}`;
  },
  transcribeAudio: async (audioPath: string): Promise<string> => {
    // Mock mode: canned transcript so the voice-input UI is exercisable.
    return `语音转文字（Mock）：${audioPath.split("/").pop() ?? "audio"}`;
  },
  readImagePreview: async (_path: string): Promise<string> => {
    // Mock mode: no filesystem; return a 1×1 transparent PNG so image
    // attachments render without a real file (path-based history view).
    return "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
  },

  listNotes: async (): Promise<NoteMeta[]> => [...notes],
  rebuildIndex: async (): Promise<unknown> => ({ status: "rebuilt" }),
  vaultSyncStatus: async (): Promise<{
    disk_files: number;
    indexed_notes: number;
    needs_rebuild: boolean;
    latest_disk_mtime: string;
  }> => ({
    disk_files: 10,
    indexed_notes: 10,
    needs_rebuild: false,
    latest_disk_mtime: new Date().toISOString(),
  }),
  discoverDevice: async (
    _ip: string
  ): Promise<{
    hostname: string;
    platform: string;
    vaultPilotVersion: string;
    noteCount: number;
    vaultName: string;
  } | null> => null,
  scanLanDevices: async (): Promise<
    Array<{
      ip: string;
      hostname: string;
      platform: string;
      vaultPilotVersion: string;
      noteCount: number;
      vaultName: string;
    }>
  > => [],
  generatePairCode: async (): Promise<string> => "MOCK12",
  regeneratePairCode: async (): Promise<string> => "MOCK13",
  listSyncPeers: async (): Promise<
    Array<{
      deviceId: string;
      hostname: string;
      platform: string;
      token: string;
      ip: string | null;
      addedAt: string;
      lastSyncAt: string | null;
    }>
  > => [],
  removeSyncPeer: async (_deviceId: string): Promise<void> => undefined,
  completePairing: async (
    _ip: string,
    _pairCode: string
  ): Promise<{
    deviceId: string;
    hostname: string;
    platform: string;
    token: string;
    ip: string | null;
    addedAt: string;
    lastSyncAt: string | null;
  }> => ({
    deviceId: "mock-peer",
    hostname: "Mock Peer",
    platform: "linux",
    token: "mock-token",
    ip: "127.0.0.1",
    addedAt: new Date().toISOString(),
    lastSyncAt: null,
  }),
  syncWithPeer: async (
    _ip: string,
    _deviceId: string,
    _mode?: "full" | "selected",
    _includes?: string[]
  ): Promise<{
    pulled: number;
    pushed: number;
    conflicts: number;
    errors: string[];
  }> => ({ pulled: 0, pushed: 0, conflicts: 0, errors: [] }),
  loadNote: async (id: string): Promise<NoteDocument> => {
    const doc = noteDocs[id];
    if (!doc) throw new Error(`note not found: ${id}`);
    return JSON.parse(JSON.stringify(doc));
  },
  saveNote: async (note: NoteDocument): Promise<NoteDocument> => {
    noteDocs[note.meta.id] = JSON.parse(JSON.stringify(note));
    return note;
  },
  deleteNote: async (id: string): Promise<boolean> => {
    notes = notes.filter((n) => n.id !== id);
    delete noteDocs[id];
    return true;
  },

  findBacklinks: async (id: string): Promise<BacklinkEntry[]> => {
    const target = noteDocs[id];
    if (!target) return [];
    // Simulate: any other note whose body contains [[<target title>]].
    const title = target.meta.title.toLowerCase();
    const result: BacklinkEntry[] = [];
    for (const [nid, doc] of Object.entries(noteDocs)) {
      if (nid === id) continue;
      const m = doc.body.match(/\[\[([^\]|#]+)(?:[|\]#][^\]]*)?\]\]/g) ?? [];
      if (m.some((link) => link.toLowerCase().includes(title))) {
        result.push({ meta: doc.meta, linkTarget: doc.meta.title });
      }
    }
    return result;
  },

  // ── collections (in-memory tree) ──
  listCollections: async (): Promise<Collection[]> => [...mockCollections],
  createCollection: async (
    name: string,
    description?: string,
    parentId?: string
  ): Promise<Collection> => {
    const ts = now();
    const col: Collection = {
      id: `col-${mockCollections.length + 1}`,
      name,
      description: description ?? "",
      createdAt: ts,
      updatedAt: ts,
      parentId: parentId ?? "",
      noteCount: 0,
    };
    mockCollections.push(col);
    return col;
  },
  renameCollection: async (collectionId: string, name: string): Promise<boolean> => {
    const col = mockCollections.find((c) => c.id === collectionId);
    if (!col) return false;
    col.name = name;
    return true;
  },
  moveCollection: async (collectionId: string, newParentId?: string): Promise<boolean> => {
    const col = mockCollections.find((c) => c.id === collectionId);
    if (!col) return false;
    col.parentId = newParentId ?? "";
    return true;
  },
  deleteCollection: async (collectionId: string): Promise<boolean> => {
    const before = mockCollections.length;
    mockCollections = mockCollections.filter((c) => c.id !== collectionId);
    return mockCollections.length < before;
  },
  addNoteToCollection: async (): Promise<boolean> => true,
  removeNoteFromCollection: async (): Promise<boolean> => true,
  listNotesInCollection: async (collectionId: string): Promise<{ notes: NoteMeta[] }> => {
    if (collectionId === "col-1") {
      return { notes: [notes[0]] };
    }
    if (collectionId === "col-2") {
      return { notes: [notes[1]] };
    }
    return { notes: [] };
  },
  getCollectionsForNote: async (noteId: string): Promise<Collection[]> => {
    if (noteId === "note-1") return [mockCollections[0]];
    if (noteId === "note-2") return [mockCollections[1], mockCollections[2]];
    return [];
  },

  listActions: async (): Promise<unknown[]> => [],
  compressChatHistory: async (
    _summary: ConversationSummary | null,
    _history: ConversationTurn[]
  ): Promise<ConversationSummary> => ({ summary: "压缩摘要", createdAt: now() }),

  // ── triggers ──
  listTriggerRules: async (): Promise<TriggerRule[]> => [
    {
      id: "mock-trigger-1",
      label: "每日早间回顾",
      triggerType: "cron",
      triggerConfig: "0 8 * * *",
      action: "daily_review",
      enabled: true,
      lastFiredAt: now(),
      nextFireAt: now(),
      runCount: 12,
      lastStatus: "success",
    },
    {
      id: "mock-trigger-2",
      label: "新笔记自动标签",
      triggerType: "event",
      triggerConfig: "note_created",
      filter: "tags CONTAINS meeting",
      action: "summarize_and_tag",
      enabled: false,
      runCount: 0,
    },
  ],
  listTriggerExecutions: async (limit?: number): Promise<TriggerExecution[]> => {
    const rows: TriggerExecution[] = [
      {
        id: "mock-exec-1",
        ruleId: "mock-trigger-1",
        label: "每日早间回顾",
        action: "daily_review",
        firedAt: now(),
        status: "success",
        error: "",
        detail: "tokens_in=1000 tokens_out=200",
        resultContent: "这是AI生成的每日回顾结果内容…",
      },
      {
        id: "mock-exec-2",
        ruleId: "mock-trigger-1",
        label: "每日早间回顾",
        action: "daily_review",
        firedAt: now(),
        status: "failed",
        error: "AI execution failed: no API key configured",
        detail: "",
        resultContent: "",
      },
    ];
    return typeof limit === "number" ? rows.slice(0, limit) : rows;
  },
  deleteTriggerExecution: async (_executionId: string): Promise<boolean> => true,
  clearTriggerExecutions: async (): Promise<number> => 2,
  createTriggerRule: async (
    label: string,
    triggerType: string,
    triggerConfig: string,
    action: string,
    filter?: string,
    customPrompt?: string,
    providerName?: string
  ): Promise<TriggerRule> => ({
    id: `mock-trigger-${Date.now()}`,
    label,
    triggerType: triggerType as "cron" | "event",
    triggerConfig,
    filter,
    action,
    enabled: true,
    customPrompt,
    providerName,
  }),
  toggleTriggerRule: async (_ruleId: string): Promise<boolean> => true,
  deleteTriggerRule: async (_ruleId: string): Promise<boolean> => true,
  fireTriggerRuleNow: async (
    _ruleId: string
  ): Promise<{ success: boolean; error: string | null; detail: string | null }> => ({
    success: true,
    error: null,
    detail: "note_id=mock-note-fire-now",
  }),
  updateTriggerRule: async (
    ruleId: string,
    label: string,
    triggerType: string,
    triggerConfig: string,
    action: string,
    filter?: string,
    customPrompt?: string,
    providerName?: string
  ): Promise<TriggerRule> => ({
    id: ruleId,
    label,
    triggerType: triggerType as "cron" | "event",
    triggerConfig,
    filter,
    action,
    enabled: true,
    customPrompt,
    providerName,
  }),
} as const;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
