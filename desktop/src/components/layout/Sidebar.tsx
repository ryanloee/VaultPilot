import type { ViewId } from "./ActivityBar";

type SidebarProps = {
  view: ViewId;
};

const labels: Record<ViewId, string> = {
  chat: "会话列表",
  notes: "笔记列表",
  graph: "图谱",
  settings: "设置",
};

/** Stage-1 placeholder sidebar — real session/note lists come in later stages. */
export function Sidebar({ view }: SidebarProps) {
  return (
    <aside className="flex w-60 shrink-0 flex-col border-r border-border bg-card">
      <div className="border-b border-border px-4 py-3 text-xs font-medium uppercase tracking-wide text-muted-foreground">
        {labels[view]}
      </div>
      <div className="flex flex-1 items-center justify-center px-4 text-center text-xs text-muted-foreground">
        <p>
          {labels[view]}占位
          <br />
          <span className="opacity-60">(后续阶段填充)</span>
        </p>
      </div>
    </aside>
  );
}
