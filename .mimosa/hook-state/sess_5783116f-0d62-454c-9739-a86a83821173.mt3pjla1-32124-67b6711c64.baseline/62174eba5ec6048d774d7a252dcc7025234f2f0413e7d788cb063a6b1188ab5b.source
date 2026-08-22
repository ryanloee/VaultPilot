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

  it("persists the new session to the backend", async () => {
    useChatStore.setState({ chatState: makeChatState() });
    useChatStore.getState().newSession();
    const newId = useChatStore.getState().currentSessionId;

    await vi.waitFor(() => expect(mockApi.saveChatState).toHaveBeenCalled());

    expect(mockApi.saveChatState).toHaveBeenCalledWith(
      expect.objectContaining({
        currentSessionId: newId,
        sessions: expect.arrayContaining([expect.objectContaining({ id: newId })]),
      })
    );
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

  it("persists the selected session to the backend", async () => {
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

    await vi.waitFor(() => expect(mockApi.saveChatState).toHaveBeenCalled());

    expect(mockApi.saveChatState).toHaveBeenCalledWith(
      expect.objectContaining({ currentSessionId: "s2" })
    );
  });
});

describe("useChatStore.deleteSession", () => {
  const twoSessions: ChatState = {
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
  };

  it("removes a non-active session and keeps the current one", () => {
    useChatStore.setState({ chatState: twoSessions, currentSessionId: "s1", turns: [] });
    useChatStore.getState().deleteSession("s2");
    const s = useChatStore.getState();
    expect(s.chatState?.sessions.map((x) => x.id)).toEqual(["s1"]);
    expect(s.currentSessionId).toBe("s1");
  });

  it("falls back to the first remaining session when the active one is deleted", () => {
    useChatStore.setState({ chatState: twoSessions, currentSessionId: "s1", turns: [] });
    useChatStore.getState().deleteSession("s1");
    const s = useChatStore.getState();
    expect(s.chatState?.sessions.map((x) => x.id)).toEqual(["s2"]);
    expect(s.currentSessionId).toBe("s2");
    expect(s.turns).toEqual([{ id: "t9", role: "user", text: "b" }]);
  });

  it("clears everything when the last session is deleted and persists", async () => {
    useChatStore.setState({ chatState: twoSessions, currentSessionId: "s1", turns: [] });
    useChatStore.getState().deleteSession("s1");
    useChatStore.getState().deleteSession("s2");
    const s = useChatStore.getState();
    expect(s.chatState?.sessions).toEqual([]);
    expect(s.currentSessionId).toBeNull();
    expect(s.turns).toEqual([]);

    await vi.waitFor(() => expect(mockApi.saveChatState).toHaveBeenCalled());
    expect(mockApi.saveChatState).toHaveBeenLastCalledWith(
      expect.objectContaining({ sessions: [] })
    );
  });

  it("is a no-op for an unknown id", () => {
    useChatStore.setState({ chatState: twoSessions, currentSessionId: "s1", turns: [] });
    useChatStore.getState().deleteSession("nope");
    const s = useChatStore.getState();
    expect(s.chatState?.sessions).toHaveLength(2);
    expect(s.currentSessionId).toBe("s1");
  });
});

describe("useChatStore.send", () => {
  it("appends user turn optimistically then the assistant reply", async () => {
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
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
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
    mockApi.askWithAi.mockRejectedValue(new Error("ai timeout"));
    await useChatStore.getState().send("问题");
    const s = useChatStore.getState();
    expect(s.error).toContain("ai timeout");
    expect(s.sending).toBe(false);
    // user turn stays (optimistic), no assistant turn
    expect(s.turns).toHaveLength(1);
  });

  it("routes the reply to the initiating session when the user switches away mid-flight (#4060)", async () => {
    let resolveAsk!: (v: { answer: string; usedContextCount: number }) => void;
    mockApi.askWithAi.mockReturnValue(
      new Promise((resolve) => {
        resolveAsk = resolve;
      })
    );
    useChatStore.setState({
      chatState: makeChatState({
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
            turns: [],
            createdAt: "2026-01-01T00:00:00.000Z",
            updatedAt: "2026-01-01T00:00:00.000Z",
          },
        ],
      }),
      currentSessionId: "s1",
      turns: [{ id: "t1", role: "user", text: "a" }],
    });

    const sendPromise = useChatStore.getState().send("问题");
    // While the AI is still responding, switch to session s2.
    useChatStore.getState().selectSession("s2");
    expect(useChatStore.getState().turns).toEqual([]);

    resolveAsk({ answer: "AI 回复", usedContextCount: 0 });
    await sendPromise;

    const s = useChatStore.getState();
    // The view is still s2 and must NOT contain the reply.
    expect(s.currentSessionId).toBe("s2");
    expect(s.turns).toEqual([]);
    // The optimistic user turn + reply landed in the initiating session s1.
    const s1 = s.chatState!.sessions.find((x) => x.id === "s1")!;
    expect(s1.turns).toMatchObject([
      { role: "user", text: "a" },
      { role: "user", text: "问题" },
      { role: "assistant", text: "AI 回复" },
    ]);
    // s2 untouched.
    const s2 = s.chatState!.sessions.find((x) => x.id === "s2")!;
    expect(s2.turns).toEqual([]);
  });

  it("routes the reply to the initiating session when a new session is created mid-flight (#4060)", async () => {
    let resolveAsk!: (v: { answer: string; usedContextCount: number }) => void;
    mockApi.askWithAi.mockReturnValue(
      new Promise((resolve) => {
        resolveAsk = resolve;
      })
    );
    useChatStore.setState({
      chatState: makeChatState({
        sessions: [
          {
            id: "s1",
            title: "会话1",
            turns: [],
            createdAt: "2026-01-01T00:00:00.000Z",
            updatedAt: "2026-01-01T00:00:00.000Z",
          },
        ],
      }),
      currentSessionId: "s1",
      turns: [],
    });

    const sendPromise = useChatStore.getState().send("问题");
    // While the AI is still responding, create a new session.
    useChatStore.getState().newSession();
    const newId = useChatStore.getState().currentSessionId;
    expect(useChatStore.getState().turns).toEqual([]);

    resolveAsk({ answer: "AI 回复", usedContextCount: 0 });
    await sendPromise;

    const s = useChatStore.getState();
    // The new session view must NOT contain the reply.
    expect(s.currentSessionId).toBe(newId);
    expect(s.turns).toEqual([]);
    // The reply + optimistic user turn landed in the initiating session s1.
    const s1 = s.chatState!.sessions.find((x) => x.id === "s1")!;
    expect(s1.turns).toMatchObject([
      { role: "user", text: "问题" },
      { role: "assistant", text: "AI 回复" },
    ]);
  });

  it("passes imagePaths to askWithAi (#4074)", async () => {
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
    mockApi.askWithAi.mockResolvedValue({ answer: "AI 回复", usedContextCount: 0 });
    await useChatStore
      .getState()
      .send("看图", ["/tmp/vp-attachments/photo.png"], [
        { name: "photo.png", dataUrl: "data:image/png;base64,AAAA", path: "/tmp/vp-attachments/photo.png" },
      ]);
    expect(mockApi.askWithAi).toHaveBeenCalledWith(
      "看图",
      expect.any(Array),
      ["/tmp/vp-attachments/photo.png"],
      null
    );
  });

  it("sends image-only turns when text is empty (#4074)", async () => {
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
    mockApi.askWithAi.mockResolvedValue({ answer: "AI 回复", usedContextCount: 0 });
    await useChatStore.getState().send("", ["/tmp/vp-attachments/p.png"], [
      { name: "p.png", dataUrl: "data:image/png;base64,BBBB", path: "/tmp/vp-attachments/p.png" },
    ]);
    expect(mockApi.askWithAi).toHaveBeenCalledWith("", expect.any(Array), ["/tmp/vp-attachments/p.png"], null);
  });

  it("stores attachments on the optimistic user turn (#4074)", async () => {
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
    mockApi.askWithAi.mockResolvedValue({ answer: "AI 回复", usedContextCount: 0 });
    const attachment = { name: "p.png", dataUrl: "data:image/png;base64,CCCC", path: "/tmp/vp-attachments/p.png" };
    await useChatStore.getState().send("看图", [attachment.path!], [attachment]);
    expect(useChatStore.getState().turns[0]).toMatchObject({
      role: "user",
      text: "看图",
      attachments: [attachment],
    });
  });

  it("no-ops on empty text without imagePaths (#4074)", async () => {
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
    await useChatStore.getState().send("", undefined, undefined);
    expect(mockApi.askWithAi).not.toHaveBeenCalled();
    expect(useChatStore.getState().turns).toHaveLength(0);
  });

  it("persists attachment path/name/type but never the base64 dataUrl (#4083)", async () => {
    useChatStore.setState({ chatState: makeChatState(), currentSessionId: "s1", turns: [] });
    mockApi.askWithAi.mockResolvedValue({ answer: "AI 回复", usedContextCount: 0 });
    const attachment = {
      name: "p.png",
      type: "image/png",
      dataUrl: "data:image/png;base64,DDDD",
      path: "/vault/attachments/chat/p.png",
    };
    await useChatStore.getState().send("看图", [attachment.path], [attachment]);
    await vi.waitFor(() => expect(mockApi.saveChatState).toHaveBeenCalled());

    // What reaches the backend (and chat_state.json) has no dataUrl.
    const saveCalls = mockApi.saveChatState.mock.calls as unknown as ChatState[][];
    const saved = saveCalls[saveCalls.length - 1][0];
    const savedTurn = saved.sessions.find((s) => s.id === "s1")!.turns[0];
    expect(savedTurn.attachments).toEqual([
      { name: "p.png", type: "image/png", path: "/vault/attachments/chat/p.png" },
    ]);
    expect(JSON.stringify(saved)).not.toContain("dataUrl");

    // The live view keeps the dataUrl for optimistic rendering.
    expect(useChatStore.getState().turns[0].attachments?.[0].dataUrl).toBe(
      "data:image/png;base64,DDDD"
    );
  });
});
