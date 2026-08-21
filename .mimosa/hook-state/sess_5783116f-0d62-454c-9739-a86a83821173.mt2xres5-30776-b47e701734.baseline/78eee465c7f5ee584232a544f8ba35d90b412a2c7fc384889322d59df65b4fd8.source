import { cn } from "@/lib/utils";
import { ChatIcon, NotesIcon, SettingsIcon, TriggerIcon } from "./icons";
import type { ViewId } from "./ActivityBar";

const items: { id: ViewId; label: string; Icon: typeof ChatIcon }[] = [
  { id: "chat", label: "聊天", Icon: ChatIcon },
  { id: "notes", label: "笔记", Icon: NotesIcon },
  { id: "triggers", label: "定时唤醒", Icon: TriggerIcon },
  { id: "settings", label: "设置", Icon: SettingsIcon },
];

/** Mobile bottom navigation — shown below the `md` breakpoint. */
export function MobileTabBar({
  active,
  onSelect,
}: {
  active: ViewId;
  onSelect: (view: ViewId) => void;
}) {
  return (
    <nav className="grid shrink-0 grid-cols-4 border-t border-border bg-card pb-[env(safe-area-inset-bottom)] md:hidden">
      {items.map(({ id, label, Icon }) => (
        <button
          key={id}
          onClick={() => onSelect(id)}
          className={cn(
            "flex flex-col items-center gap-0.5 py-2 text-[10px] text-muted-foreground transition-colors",
            active === id && "text-foreground"
          )}
        >
          <Icon className="h-5 w-5" />
          {label}
          {active === id && (
            <span className="h-0.5 w-6 rounded-full bg-primary" />
          )}
        </button>
      ))}
    </nav>
  );
}
