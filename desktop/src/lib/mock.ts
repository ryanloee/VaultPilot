import type {
  AppSettings,
  BacklinkEntry,
  ChatState,
  NoteDocument,
  NoteMeta,
  ConversationSummary,
  ConversationTurn,
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
          text: "这是一个本地优先的知识库笔记应用，支持 AI 对话、笔记管理和知识图谱。",
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
    body: "# 欢迎使用 VaultPilot\n\n这是一个 **本地优先** 的知识库笔记应用。\n\n- 支持 AI 对话\n- 支持笔记管理\n- 支持知识图谱\n\n```ts\nconsole.log(\"hello\")\n```",
  },
  "note-2": {
    meta: notes[1],
    body: "# Markdown 语法\n\n## 列表\n\n- 项目一\n- 项目二\n\n## 代码\n\n```rust\nfn main() {}\n```",
  },
};

// ── mock API surface (mirrors the real api object) ────────────────────────

export const mockApi = {
  ping: async (): Promise<boolean> => true,

  getSettings: async (): Promise<AppSettings> => JSON.parse(JSON.stringify(settings)),
  saveSettings: async (s: AppSettings): Promise<AppSettings> => {
    settings = JSON.parse(JSON.stringify(s));
    return settings;
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

  listNotes: async (): Promise<NoteMeta[]> => [...notes],
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

  listActions: async (): Promise<unknown[]> => [],
  compressChatHistory: async (
    _summary: ConversationSummary | null,
    _history: ConversationTurn[]
  ): Promise<ConversationSummary> => ({ summary: "压缩摘要", createdAt: now() }),
} as const;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
