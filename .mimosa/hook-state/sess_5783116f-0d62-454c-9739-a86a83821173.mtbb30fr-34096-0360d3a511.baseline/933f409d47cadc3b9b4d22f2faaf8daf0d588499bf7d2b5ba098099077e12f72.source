import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the Tauri bridge modules BEFORE importing tauri.ts.
const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => {}),
}));

import { tauriApi } from "./tauri";

beforeEach(() => {
  invoke.mockClear();
});

/**
 * Regression test for #4082: Tauri v2 registers commands by their exact Rust
 * fn identifier (no `_cmd` stripping), so every `invoke` string the frontend
 * sends must match the Rust command name exactly — a `_cmd`-suffixed command
 * is unreachable and image send / STT fail with "Command not found".
 */
describe("tauri command-name contract (#4082)", () => {
  it("saveTempAttachment invokes save_temp_attachment (no _cmd suffix)", async () => {
    await tauriApi.saveTempAttachment("AQID", "photo.png", true);
    expect(invoke).toHaveBeenCalledWith("save_temp_attachment", {
      dataBase64: "AQID",
      filename: "photo.png",
      persistent: true,
    });
  });

  it("transcribeAudio invokes transcribe_audio (no _cmd suffix)", async () => {
    await tauriApi.transcribeAudio("/tmp/voice.webm", "en");
    expect(invoke).toHaveBeenCalledWith("transcribe_audio", {
      audioPath: "/tmp/voice.webm",
      language: "en",
    });
  });

  it("no invoke target across the whole API carries a _cmd suffix", async () => {
    // Exercise every thin wrapper; a `_cmd`-suffixed Rust command would be
    // unreachable, so any _cmd name here is a regression (#4082).
    const calls: Array<[string, unknown]> = [
      ["ping", []],
      ["getSettings", []],
      ["saveSettings", [{ vaultDir: "/vault" }]],
      ["loadChatState", []],
      ["saveChatState", [{ currentSessionId: "", sessions: [] }]],
      ["listActions", []],
      ["compressChatHistory", [null, []]],
      ["askWithAi", ["q", [], [], null]],
      ["saveTempAttachment", ["AQID", "a.png"]],
      ["transcribeAudio", ["/tmp/a.webm"]],
      ["listNotes", []],
      ["loadNote", ["n1"]],
      ["saveNote", [{ meta: { id: "n1" } }]],
      ["deleteNote", ["n1"]],
      ["findRelatedNotes", ["n1", 3]],
      ["findBacklinks", ["n1"]],
      ["importMarkdown", [["/vault/a.md"]]],
      ["rebuildIndex", []],
      ["readImagePreview", ["/vault/attachments/chat/a.png"]],
      ["openVaultDirectory", ["/vault"]],
      ["listSnapshots", ["n1"]],
      ["getSnapshot", ["s1"]],
      ["restoreSnapshot", ["n1", "s1"]],
    ] as const;
    for (const [method, args] of calls) {
      const fn = (tauriApi as Record<string, (...a: unknown[]) => unknown>)[method];
      expect(typeof fn, `tauriApi.${method} must exist`).toBe("function");
      try {
        await fn(...(args as unknown[]));
      } catch {
        // invoke is mocked — a throw here would only mean arg-shape drift;
        // the command-name assertion below is what matters.
      }
    }
    const names: string[] = invoke.mock.calls.map((c) => c[0] as string);
    expect(names.length).toBeGreaterThanOrEqual(calls.length);
    for (const name of names) {
      expect(name, `command name must not carry a _cmd suffix: ${name}`).not.toMatch(/_cmd$/);
    }
  });
});
