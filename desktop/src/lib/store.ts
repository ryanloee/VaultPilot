import { create } from "zustand";
import { api, onAgentStatus, type AgentStatusPayload } from "./tauri";
import type {
  AppSettings,
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
  send: (text: string) => Promise<void>;
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

  send: async (text) => {
    if (get().sending || !text.trim()) return;
    set({ sending: true, status: null, error: null });

    // Append the user message immediately (optimistic).
    const userTurn: ChatTurn = {
      id: crypto.randomUUID(),
      role: "user",
      text,
    };
    const history: ConversationTurn[] = get().turns.map((t) => ({ role: t.role, text: t.text }));
    set((s) => ({ turns: [...s.turns, userTurn] }));

    try {
      const result = await api.askWithAi(text, history, null, null);
      const assistantTurn: ChatTurn = {
        id: crypto.randomUUID(),
        role: "assistant",
        text: result.answer,
      };
      set((s) => ({ turns: [...s.turns, assistantTurn] }));

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
