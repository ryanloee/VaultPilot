import { useState } from "react";
import { ActivityBar, type ViewId } from "./ActivityBar";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";

const mainLabels: Record<ViewId, string> = {
  chat: "聊天",
  notes: "笔记",
  graph: "图谱",
  settings: "设置",
};

export function AppShell() {
  const [view, setView] = useState<ViewId>("chat");

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
      <div className="flex min-h-0 flex-1">
        <ActivityBar active={view} onSelect={setView} />
        <Sidebar view={view} />
        <main className="flex min-w-0 flex-1 flex-col">
          {/* Stage-1 placeholder — real views come in later stages. */}
          <div className="flex flex-1 items-center justify-center">
            <div className="text-center">
              <h2 className="text-2xl font-semibold tracking-tight">
                {mainLabels[view]}
              </h2>
              <p className="mt-2 text-sm text-muted-foreground">
                主区域占位 · 后续阶段填充
              </p>
            </div>
          </div>
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
