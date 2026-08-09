import { cn } from "@/lib/utils";
import { ChatIcon, GraphIcon, NotesIcon, SettingsIcon } from "./icons";

export type ViewId = "chat" | "notes" | "graph" | "settings";

type ActivityBarProps = {
  active: ViewId;
  onSelect: (view: ViewId) => void;
};

const items: { id: ViewId; label: string; Icon: typeof ChatIcon }[] = [
  { id: "chat", label: "聊天", Icon: ChatIcon },
  { id: "notes", label: "笔记", Icon: NotesIcon },
  { id: "graph", label: "图谱", Icon: GraphIcon },
  { id: "settings", label: "设置", Icon: SettingsIcon },
];

export function ActivityBar({ active, onSelect }: ActivityBarProps) {
  return (
    <nav className="flex w-12 flex-col items-center gap-1 border-r border-border bg-secondary py-2">
      {items.map(({ id, label, Icon }) => (
        <button
          key={id}
          title={label}
          onClick={() => onSelect(id)}
          className={cn(
            "relative flex h-10 w-10 items-center justify-center rounded-md text-muted-foreground transition-colors hover:text-foreground",
            active === id && "text-foreground"
          )}
        >
          {/* active indicator: left accent bar */}
          {active === id && (
            <span className="absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-r bg-primary" />
          )}
          <Icon className="h-5 w-5" />
        </button>
      ))}
    </nav>
  );
}
