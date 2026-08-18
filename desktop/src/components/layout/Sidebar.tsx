import { useEffect } from "react";
import { useChatStore } from "@/lib/store";
import { cn, formatDate } from "@/lib/utils";
import { TrashIcon } from "./icons";

type SidebarProps = {
  collapsed: boolean;
};

/** Sidebar shows the chat session list (chat view only). */
export function Sidebar({ collapsed }: SidebarProps) {
  const { chatState, currentSessionId, load, selectSession, newSession, deleteSession } =
    useChatStore();

  useEffect(() => {
    load();
  }, [load]);

  return (
    <aside
      className={cn(
        "flex shrink-0 flex-col border-r border-border bg-card transition-[width] duration-200",
        collapsed ? "w-0 overflow-hidden border-r-0" : "w-60"
      )}
    >
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          会话列表
        </span>
        <button
          onClick={newSession}
          title="新会话"
          className="text-muted-foreground hover:text-foreground"
        >
          +
        </button>
      </div>

      <div className="flex-1 overflow-auto vp-scroll">
        {chatState?.sessions.map((s) => (
          <div
            key={s.id}
            className={cn(
              "group flex items-center border-b border-border transition-colors hover:bg-accent",
              currentSessionId === s.id && "bg-accent"
            )}
          >
            <button
              onClick={() => selectSession(s.id)}
              className="min-w-0 flex-1 py-2 pl-4 pr-1 text-left text-sm"
            >
              <div className="truncate">{s.title || "新会话"}</div>
              {formatDate(s.updatedAt) && (
                <div className="mt-0.5 text-[10px] text-muted-foreground">
                  {formatDate(s.updatedAt)}
                </div>
              )}
            </button>
            <button
              onClick={() => deleteSession(s.id)}
              title="删除会话"
              aria-label={`删除会话 ${s.title || "新会话"}`}
              className="mr-2 shrink-0 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100 focus-visible:opacity-100"
            >
              <TrashIcon className="h-3.5 w-3.5" />
            </button>
          </div>
        ))}
        {(!chatState || chatState.sessions.length === 0) && (
          <p className="px-4 py-6 text-center text-xs text-muted-foreground">暂无会话</p>
        )}
      </div>
    </aside>
  );
}
