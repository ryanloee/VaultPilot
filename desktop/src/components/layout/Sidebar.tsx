import { useEffect } from "react";
import type { ViewId } from "./ActivityBar";
import { useChatStore } from "@/lib/store";
import { cn, formatDate } from "@/lib/utils";

type SidebarProps = {
  view: ViewId;
};

const labels: Record<ViewId, string> = {
  chat: "会话列表",
  notes: "笔记列表",
  graph: "图谱",
  settings: "设置",
};

/** Sidebar shows chat sessions in chat view; placeholder elsewhere. */
export function Sidebar({ view }: SidebarProps) {
  const { chatState, currentSessionId, load, selectSession, newSession } = useChatStore();

  useEffect(() => {
    if (view === "chat") load();
  }, [view, load]);

  return (
    <aside className="flex w-60 shrink-0 flex-col border-r border-border bg-card">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          {labels[view]}
        </span>
        {view === "chat" && (
          <button
            onClick={newSession}
            title="新会话"
            className="text-muted-foreground hover:text-foreground"
          >
            +
          </button>
        )}
      </div>

      {view === "chat" ? (
        <div className="flex-1 overflow-auto vp-scroll">
          {chatState?.sessions.map((s) => (
            <button
              key={s.id}
              onClick={() => selectSession(s.id)}
              className={cn(
                "block w-full border-b border-border px-4 py-2 text-left text-sm transition-colors hover:bg-accent",
                currentSessionId === s.id && "bg-accent"
              )}
            >
              <div className="truncate">{s.title || "新会话"}</div>
              {formatDate(s.updatedAt) && (
                <div className="mt-0.5 text-[10px] text-muted-foreground">
                  {formatDate(s.updatedAt)}
                </div>
              )}
            </button>
          ))}
          {(!chatState || chatState.sessions.length === 0) && (
            <p className="px-4 py-6 text-center text-xs text-muted-foreground">暂无会话</p>
          )}
        </div>
      ) : (
        <div className="flex flex-1 items-center justify-center px-4 text-center text-xs text-muted-foreground">
          <p>
            {labels[view]}占位
            <br />
            <span className="opacity-60">(后续阶段填充)</span>
          </p>
        </div>
      )}
    </aside>
  );
}
