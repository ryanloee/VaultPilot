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

describe("mockApi trigger rules", () => {
  it("updateTriggerRule returns the rule with the new values", async () => {
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

describe("isTauri", () => {
  it("returns false in a plain Node test environment (no window.__TAURI_INTERNALS__)", () => {
    expect(isTauri()).toBe(false);
  });
});
