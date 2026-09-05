import { useState, useEffect } from "react";
import { ActivityBar, type ViewId } from "./ActivityBar";
import { MobileTabBar } from "./MobileTabBar";
import { Sidebar } from "./Sidebar";
import { StatusBar } from "./StatusBar";
import { ChatView } from "@/components/chat/ChatView";
import { IntegrationsView } from "@/components/integrations/IntegrationsView";
import { NotesView } from "@/components/notes/NotesView";
import { TriggerView } from "@/components/triggers/TriggerView";
import { SettingsView } from "@/components/settings/SettingsView";
import { useAutoUpdater } from "@/hooks/useAutoUpdater";
import { api } from "@/lib/tauri";

export function AppShell() {
  const [view, setView] = useState<ViewId>("chat");
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false);
  const [desktop, setDesktop] = useState(false);
  useAutoUpdater();

  // Resolve platform once on mount — gates desktop-only views.
  useEffect(() => {
    let cancelled = false;
    api.isDesktop().then((v) => {
      if (!cancelled) setDesktop(v);
    }).catch(() => {});
    return () => { cancelled = true; };
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-background text-foreground pt-[env(safe-area-inset-top)] pb-[env(safe-area-inset-bottom)]">
      <div className="flex min-h-0 flex-1">
        <div className="hidden md:flex">
          <ActivityBar
            active={view}
            onSelect={setView}
            sidebarCollapsed={sidebarCollapsed}
            onToggleSidebar={() => setSidebarCollapsed((v) => !v)}
          />
        </div>
        {view === "chat" && (
          <div className="hidden md:block">
            <Sidebar collapsed={sidebarCollapsed} />
          </div>
        )}
        <main className="flex min-w-0 flex-1 flex-col">
          {view === "chat" && <ChatView />}
          {view === "notes" && <NotesView />}
          {desktop && view === "triggers" && <TriggerView />}
          {view === "integrations" && <IntegrationsView />}
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
