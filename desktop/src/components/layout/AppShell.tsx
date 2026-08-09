import { useState } from "react";
import { ActivityBar, type ViewId } from "./ActivityBar";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { ChatView } from "@/components/chat/ChatView";
import { NotesView } from "@/components/notes/NotesView";
import { SettingsView } from "@/components/settings/SettingsView";

export function AppShell() {
  const [view, setView] = useState<ViewId>("chat");

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground">
      <div className="flex min-h-0 flex-1">
        <ActivityBar active={view} onSelect={setView} />
        {view !== "settings" && <Sidebar view={view} />}
        <main className="flex min-w-0 flex-1 flex-col">
          {view === "chat" && <ChatView />}
          {view === "notes" && <NotesView />}
          {view === "graph" && (
            <div className="flex flex-1 items-center justify-center">
              <div className="text-center">
                <h2 className="text-2xl font-semibold tracking-tight">知识图谱</h2>
                <p className="mt-2 text-sm text-muted-foreground">后续阶段实现</p>
              </div>
            </div>
          )}
          {view === "settings" && <SettingsView />}
        </main>
      </div>
      <StatusBar />
    </div>
  );
}
