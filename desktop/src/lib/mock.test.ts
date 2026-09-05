import { describe, it, expect } from "vitest";
import { mockApi, isTauri } from "./mock";
import type { NoteDocument } from "@/types";

describe("mockApi.getSettings / saveSettings round-trip", () => {
  it("returns a deep copy (mutating result does not affect internal state)", async () => {
    const s1 = await mockApi.getSettings();
    s1.systemDirective = "MUTATED";
    const s2 = await mockApi.getSettings();
    expect(s2.systemDirective).not.toBe("MUTATED");
  });

  it("saveSettings persists and returns the saved value", async () => {
    const saved = await mockApi.saveSettings({
      ...(await mockApi.getSettings()),
      systemDirective: "be concise",
      proxyUrl: "http://proxy:7897",
    });
    expect(saved.systemDirective).toBe("be concise");
    const reread = await mockApi.getSettings();
    expect(reread.systemDirective).toBe("be concise");
    expect(reread.proxyUrl).toBe("http://proxy:7897");
  });
});

describe("mockApi notes CRUD", () => {
  it("listNotes returns an array of note metadata", async () => {
    const notes = await mockApi.listNotes();
    expect(Array.isArray(notes)).toBe(true);
    for (const n of notes) {
      expect(n).toHaveProperty("id");
      expect(n).toHaveProperty("title");
    }
  });

  it("loadNote returns a full document for an existing note", async () => {
    const notes = await mockApi.listNotes();
    if (notes.length === 0) return; // no fixture notes — skip
    const doc = await mockApi.loadNote(notes[0].id);
    expect(doc.meta.id).toBe(notes[0].id);
  });

  it("loadNote throws for a missing id", async () => {
    await expect(mockApi.loadNote("definitely-missing-note")).rejects.toThrow();
  });

  it("saveNote then loadNote round-trips content", async () => {
    const notes = await mockApi.listNotes();
    const base = notes.length
      ? await mockApi.loadNote(notes[0].id)
      : null;
    const doc: NoteDocument = base ?? {
      meta: {
        id: "test-note-1",
        title: "测试笔记",
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        tags: [],
      },
      body: "# 标题\n\n正文内容",
    };
    doc.body = "# 更新后的内容";
    await mockApi.saveNote(doc);
    const reread = await mockApi.loadNote(doc.meta.id);
    expect(reread.body).toBe("# 更新后的内容");
  });

  it("deleteNote removes a saved note", async () => {
    // Use a dedicated note so we don't disturb fixture data (module state
    // is shared across tests in the same file).
    const doc: NoteDocument = {
      meta: {
        id: "test-note-to-delete",
        title: "待删除",
        createdAt: new Date().toISOString(),
        updatedAt: new Date().toISOString(),
        tags: [],
      },
      body: "content",
    };
    await mockApi.saveNote(doc);
    const ok = await mockApi.deleteNote(doc.meta.id);
    expect(ok).toBe(true);
    await expect(mockApi.loadNote(doc.meta.id)).rejects.toThrow();
  });
});

describe("mockApi trigger rules", () => {  it("updateTriggerRule returns the rule with the new values", async () => {
    const updated = await mockApi.updateTriggerRule(
      "mock-trigger-1",
      "改后的回顾",
      "cron",
      "30 18 * * 1-5",
      "custom",
      undefined,
      "总结今天"
    );
    expect(updated.id).toBe("mock-trigger-1");
    expect(updated.label).toBe("改后的回顾");
    expect(updated.triggerConfig).toBe("30 18 * * 1-5");
    expect(updated.action).toBe("custom");
    expect(updated.customPrompt).toBe("总结今天");
  });
});

describe("mockApi feeds", () => {
  it("addFeed then listFeeds contains the new feed", async () => {
    const feed = await mockApi.addFeed(
      "https://example.com/new.xml",
      "New Feed",
      "rss",
      "",
      "tech",
      30
    );
    expect(feed.id).toBeTruthy();
    expect(feed.url).toBe("https://example.com/new.xml");
    const feeds = await mockApi.listFeeds();
    expect(feeds.some((f) => f.id === feed.id)).toBe(true);
  });

  it("setFeedEnabled toggles and removeFeed deletes", async () => {
    const feed = await mockApi.addFeed(
      "https://example.com/toggle.xml",
      "Toggle",
      "rss",
      "",
      "",
      60
    );
    expect(await mockApi.setFeedEnabled(feed.id, false)).toBe(true);
    expect((await mockApi.listFeeds()).find((f) => f.id === feed.id)?.enabled).toBe(
      false
    );
    expect(await mockApi.removeFeed(feed.id)).toBe(true);
    expect((await mockApi.listFeeds()).some((f) => f.id === feed.id)).toBe(false);
  });

  it("refreshFeeds returns per-feed results", async () => {
    const results = await mockApi.refreshFeeds();
    expect(Array.isArray(results)).toBe(true);
    for (const r of results) {
      expect(r).toHaveProperty("feedId");
      expect(r).toHaveProperty("status");
    }
  });
});

describe("mockApi mail accounts", () => {
  it("addMailAccount then listMailAccounts contains it (no password field)", async () => {
    const acc = await mockApi.addMailAccount(
      "Test",
      "imap.example.com",
      993,
      "t@example.com",
      "secret",
      true,
      30
    );
    expect(acc.id).toBeTruthy();
    // The DTO must never carry a password.
    expect(acc).not.toHaveProperty("password");
    const accounts = await mockApi.listMailAccounts();
    expect(accounts.some((a) => a.id === acc.id)).toBe(true);
  });

  it("syncMailAccount reports counts and updates lastSyncAt", async () => {
    const acc = await mockApi.addMailAccount(
      "SyncMe",
      "imap.example.com",
      993,
      "s@example.com",
      "secret",
      true,
      30
    );
    const r = await mockApi.syncMailAccount(acc.id);
    expect(r.accountId).toBe(acc.id);
    expect(r.fetched + r.imported + r.skippedDuplicates).toBeGreaterThanOrEqual(0);
    const reread = (await mockApi.listMailAccounts()).find((a) => a.id === acc.id);
    expect(reread?.lastSyncAt).toBeTruthy();
    expect(await mockApi.deleteMailAccount(acc.id)).toBe(true);
  });

  it("searchEmails matches the fixture mail", async () => {
    const hits = await mockApi.searchEmails("Mock");
    expect(hits.length).toBeGreaterThanOrEqual(1);
    expect(hits[0]).toHaveProperty("subject");
  });
});

describe("isTauri", () => {
  it("returns false in a plain Node test environment (no window.__TAURI_INTERNALS__)", () => {
    expect(isTauri()).toBe(false);
  });
});
