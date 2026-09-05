import { describe, it, expect } from "vitest";
import { visibleTabs } from "./IntegrationsView";

describe("visibleTabs platform gating", () => {
  it("desktop shows all three tabs including MCP", () => {
    const tabs = visibleTabs(true);
    expect(tabs.map((t) => t.id)).toEqual(["feeds", "mail", "mcp"]);
  });

  it("mobile hides the MCP tab — it is a PC-side client setup guide", () => {
    const tabs = visibleTabs(false);
    expect(tabs.map((t) => t.id)).toEqual(["feeds", "mail"]);
    expect(tabs.some((t) => t.id === "mcp")).toBe(false);
  });
});
