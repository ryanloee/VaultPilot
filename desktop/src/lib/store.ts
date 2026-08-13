import { create } from "zustand";
import { api, onAgentStatus, type AgentStatusPayload } from "./tauri";
import type {
  AppSettings,
  ChatAttachment,
  ChatSession,
  ChatState,
  ChatTurn,
  ConversationTurn,
  NoteDocument,
  NoteMeta,
} from "@/types";

// ── Settings store ────────────────────────────────────────────────────────

type SettingsStore = {
  settings: AppSettings | null;
  loading: boolean;
  error: string | null;
  load: () => Promise<void>;
  save: (s: AppSettings) => Promise<void>;
};

export const useSettingsStore = create<SettingsStore>((set) => ({
  settings: null,
  loading: false,
  error: null,
  load: async () => {
    set({ loading: true, error: null });
    try {
      const s = await api.getSettings();
      set({ settings: s, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
  save: async (s) => {
    set({ loading: true, error: null });
    try {
      const saved = await api.saveSettings(s);
      set({ settings: saved, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
}));

// ── Notes store ───────────────────────────────────────────────────────────

type NotesStore = {
  notes: NoteMeta[];
  loading: boolean;
  current: NoteDocument | null;
  error: string | null;
  loadList: () => Promise<void>;
  open: (id: string) => Promise<void>;
  saveCurrent: (body: string, title?: string) => Promise<void>;
};

export const useNotesStore = create<NotesStore>((set, get) => ({
  notes: [],
  loading: false,
  current: null,
  error: null,
  loadList: async () => {
    set({ loading: true, error: null });
    try {
      const list = await api.listNotes();
      set({ notes: list, loading: false });
    } catch (e) {
      set({ error: String(e), loading: false });
    }
  },
  open: async (id) => {
    try {
      const doc = await api.loadNote(id);
      set({ current: doc });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  saveCurrent: async (body, title) => {
    const cur = get().current;
    if (!cur) return;
    const updated: NoteDocument = {
      ...cur,
      body,
      meta: title ? { ...cur.meta, title } : cur.meta,
    };
    try {
      const saved = await api.saveNote(updated);
      set({ current: saved });
    } catch (e) {
      set({ error: String(e) });
    }
  },
}));

// ── Chat store ────────────────────────────────────────────────────────────

type ChatStore = {
  chatState: ChatState | null;
  currentSessionId: string | null;
  turns: ChatTurn[];
  sending: boolean;
  status: AgentStatusPayload | null;
  error: string | null;
  load: () => Promise<void>;
  send: (text: string, imagePaths?: string[], attachments?: ChatAttachment[]) => Promise<void>;
  newSession: () => void;
  selectSession: (id: string) => void;
};

function emptySession(): ChatSession {
  return {
    id: crypto.randomUUID(),
    title: "新会话",
    turns: [],
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
  };
}

export const useChatStore = create<ChatStore>((set, get) => ({
  chatState: null,
  currentSessionId: null,
  turns: [],
  sending: false,
  status: null,
  error: null,

  load: async () => {
    try {
      const state = await api.loadChatState();
      const sessionId = state.currentSessionId;
      const session = state.sessions.find((s) => s.id === sessionId) ?? state.sessions[0];
      set({
        chatState: state,
        currentSessionId: session?.id ?? null,
        turns: session?.turns ?? [],
      });
    } catch (e) {
      // No chat state yet is fine for a fresh install.
      set({ error: String(e) });
    }
  },

  send: async (text, imagePaths, attachments) => {
    if (get().sending || (!text.trim() && !imagePaths?.length)) return;
    // Snapshot the session that initiates the request: the reply must be
    // written back to THIS session even if the user switches sessions or
    // creates a new one while the AI is still responding (#4060).
    const sessionId = get().currentSessionId;
    if (!sessionId) return;
    set({ sending: true, status: null, error: null });

    // Append the user message immediately (optimistic).
    const userTurn: ChatTurn = {
      id: crypto.randomUUID(),
      role: "user",
      text,
      ...(attachments && attachments.length > 0 ? { attachments } : {}),
    };
    const history: ConversationTurn[] = get().turns.map((t) => ({ role: t.role, text: t.text }));
    set((s) => ({ turns: [...s.turns, userTurn] }));

    // Snapshot the initiating session's turn list right after the optimistic
    // append — the reply is appended to this snapshot, never to whatever
    // session happens to be selected when the response arrives.
    const turnsAtSend = get().turns;

    try {
      const result = await api.askWithAi(text, history, imagePaths ?? null, null);
      const assistantTurn: ChatTurn = {
        id: crypto.randomUUID(),
        role: "assistant",
        text: result.answer,
      };
      const stillOnInitiatingSession = get().currentSessionId === sessionId;
      set((s) => {
        const chatState = s.chatState;
        if (!chatState) return {};
        const finalTurns = [...turnsAtSend, assistantTurn];
        const sessions = chatState.sessions.map((sess) =>
          sess.id === sessionId
            ? { ...sess, turns: finalTurns, updatedAt: new Date().toISOString() }
            : sess
        );
        return {
          chatState: { ...chatState, sessions },
          // Only touch the active view if the initiating session is still selected.
          ...(stillOnInitiatingSession ? { turns: finalTurns } : {}),
        };
      });

      // Persist chat state in the background.
      void persistChatState(get);
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set({ sending: false, status: null });
    }
  },

  newSession: () => {
    const session = emptySession();
    set((s) => ({
      chatState: s.chatState
        ? {
            ...s.chatState,
            currentSessionId: session.id,
            sessions: [session, ...s.chatState.sessions],
          }
        : { currentSessionId: session.id, sessions: [session] },
      currentSessionId: session.id,
      turns: [],
    }));
    void persistChatState(get);
  },

  selectSession: (id) => {
    const state = get().chatState;
    if (!state) return;
    const session = state.sessions.find((s) => s.id === id);
    if (session) {
      set({
        currentSessionId: id,
        turns: session.turns ?? [],
        chatState: { ...state, currentSessionId: id },
      });
      void persistChatState(get);
    }
  },
}));

// Wire up status events from the backend (only once).
let statusUnlisten: Promise<unknown> | null = null;
statusUnlisten ??= onAgentStatus((payload) => {
  useChatStore.setState({ status: payload });
});

async function persistChatState(get: () => ChatStore) {
  const { chatState, currentSessionId, turns } = get();
  if (!chatState || !currentSessionId) return;
  const sessions = chatState.sessions.map((s) =>
    s.id === currentSessionId ? { ...s, turns, updatedAt: new Date().toISOString() } : s
  );
  const updated: ChatState = { ...chatState, sessions };
  useChatStore.setState({ chatState: updated });
  try {
    await api.saveChatState(updated);
  } catch {
    // best-effort
  }
}
