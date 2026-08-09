import { useState } from "react";
import { ActivityBar, type ViewId } from "./ActivityBar";
import { MobileTabBar } from "./MobileTabBar";
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
        <div className="hidden md:flex">
          <ActivityBar active={view} onSelect={setView} />
        </div>
        {view !== "settings" && (
          <div className="hidden md:block">
            <Sidebar view={view} />
          </div>
        )}
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
      <div className="hidden md:block">
        <StatusBar />
      </div>
      <MobileTabBar active={view} onSelect={setView} />
    </div>
  );
}
