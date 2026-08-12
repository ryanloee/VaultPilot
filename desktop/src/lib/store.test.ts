import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the tauri bridge BEFORE importing the store.
// NOTE: vi.mock factories are hoisted above imports, so the mock object must
// be created via vi.hoisted to be referenceable from the factory.
const { mockApi } = vi.hoisted(() => ({
  mockApi: {
    ping: vi.fn(async () => true),
    getSettings: vi.fn(async () => null),
    saveSettings: vi.fn(),
    loadChatState: vi.fn(),
    saveChatState: vi.fn(async () => null),
    askWithAi: vi.fn(),
    listNotes: vi.fn(async () => []),
    loadNote: vi.fn(),
    saveNote: vi.fn(),
    deleteNote: vi.fn(async () => true),
    listActions: vi.fn(async () => []),
    compressChatHistory: vi.fn(),
  },
}));

vi.mock("./tauri", () => ({
  api: mockApi,
  onAgentStatus: vi.fn(() => Promise.resolve(() => {})),
  isTauri: () => false,
}));

// crypto.randomUUID is available in Node ≥ 19; ensure it exists.
import { useChatStore } from "./store";
import type { ChatState } from "@/types";

function makeChatState(overrides: Partial<ChatState> = {}): ChatState {
  return {
    currentSessionId: "s1",
    sessions: [
      {
        id: "s1",
        title: "会话1",
        turns: [
          { id: "t1", role: "user", text: "你好" },
          { id: "t2", role: "assistant", text: "你好！" },
        ],
        createdAt: "2026-01-01T00:00:00.000Z",
        updatedAt: "2026-01-01T00:00:00.000Z",
      },
    ],
    ...overrides,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  useChatStore.setState({
    chatState: null,
    currentSessionId: null,
    turns: [],
    sending: false,
    status: null,
    error: null,
  });
});

describe("useChatStore.load", () => {
  it("loads chat state and selects the current session", async () => {
    mockApi.loadChatState.mockResolvedValue(makeChatState());
    await useChatStore.getState().load();
    const s = useChatStore.getState();
    expect(s.chatState).not.toBeNull();
    expect(s.currentSessionId).toBe("s1");
    expect(s.turns).toHaveLength(2);
    expect(s.turns[0].text).toBe("你好");
  });

  it("falls back to the first session when currentSessionId is empty", async () => {
    mockApi.loadChatState.mockResolvedValue(makeChatState({ currentSessionId: "" }));
    await useChatStore.getState().load();
    expect(useChatStore.getState().currentSessionId).toBe("s1");
  });

  it("sets an error when the backend is unreachable", async () => {
    mockApi.loadChatState.mockRejectedValue(new Error("backend down"));
    await useChatStore.getState().load();
    expect(useChatStore.getState().error).toContain("backend down");
  });
});

describe("useChatStore.newSession", () => {
  it("creates a session with a fresh id and empty turns", () => {
    useChatStore.setState({ chatState: makeChatState() });
    useChatStore.getState().newSession();
    const s = useChatStore.getState();
    expect(s.currentSessionId).not.toBe("s1");
    expect(s.turns).toEqual([]);
    expect(s.chatState?.sessions[0].id).toBe(s.currentSessionId);
  });
});

describe("useChatStore.selectSession", () => {
  it("switches to the requested session", () => {
    useChatStore.setState({
      chatState: makeChatState({
        currentSessionId: "s1",
        sessions: [
          {
            id: "s1",
            title: "会话1",
            turns: [{ id: "t1", role: "user", text: "a" }],
            createdAt: "2026-01-01T00:00:00.000Z",
            updatedAt: "2026-01-01T00:00:00.000Z",
          },
          {
            id: "s2",
            title: "会话2",
            turns: [{ id: "t9", role: "user", text: "b" }],
            createdAt: "2026-01-01T00:00:00.000Z",
            updatedAt: "2026-01-01T00:00:00.000Z",
          },
        ],
      }),
    });
    useChatStore.getState().selectSession("s2");
    const s = useChatStore.getState();
    expect(s.currentSessionId).toBe("s2");
    expect(s.turns).toEqual([{ id: "t9", role: "user", text: "b" }]);
    expect(s.chatState?.currentSessionId).toBe("s2");
  });
});

describe("useChatStore.send", () => {
  it("appends user turn optimistically then the assistant reply", async () => {
    useChatStore.setState({ chatState: makeChatState(), turns: [] });
    mockApi.askWithAi.mockResolvedValue({ answer: "AI 回复", usedContextCount: 0 });
    await useChatStore.getState().send("问题");
    const s = useChatStore.getState();
    expect(s.turns[0]).toMatchObject({ role: "user", text: "问题" });
    expect(s.turns[1]).toMatchObject({ role: "assistant", text: "AI 回复" });
    expect(s.sending).toBe(false);
  });

  it("ignores empty or whitespace-only input", async () => {
    useChatStore.setState({ chatState: makeChatState(), turns: [] });
    await useChatStore.getState().send("   ");
    expect(mockApi.askWithAi).not.toHaveBeenCalled();
    expect(useChatStore.getState().turns).toHaveLength(0);
  });

  it("records an error and clears sending when the AI call fails", async () => {
    useChatStore.setState({ chatState: makeChatState(), turns: [] });
    mockApi.askWithAi.mockRejectedValue(new Error("ai timeout"));
    await useChatStore.getState().send("问题");
    const s = useChatStore.getState();
    expect(s.error).toContain("ai timeout");
    expect(s.sending).toBe(false);
    // user turn stays (optimistic), no assistant turn
    expect(s.turns).toHaveLength(1);
  });
});
