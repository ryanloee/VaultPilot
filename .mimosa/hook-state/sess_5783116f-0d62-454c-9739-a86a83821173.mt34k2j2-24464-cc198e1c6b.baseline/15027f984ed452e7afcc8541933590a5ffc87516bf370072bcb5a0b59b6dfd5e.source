import { describe, it, expect, vi } from "vitest";

// ChatView pulls in the tauri bridge at module scope — mock it so this pure
// helper test runs in the node environment.
vi.mock("@/lib/tauri", () => ({
  api: {},
  onAgentStatus: vi.fn(() => Promise.resolve(() => {})),
  isTauri: () => false,
}));

import { dataUrlToBase64 } from "./ChatView";

describe("dataUrlToBase64 (#4074)", () => {
  it("strips the data:<mime>;base64, prefix", () => {
    expect(dataUrlToBase64("data:image/png;base64,AAAA")).toBe("AAAA");
  });

  it("returns input unchanged when there is no comma", () => {
    expect(dataUrlToBase64("AAAA")).toBe("AAAA");
  });

  it("handles empty string", () => {
    expect(dataUrlToBase64("")).toBe("");
  });
});
