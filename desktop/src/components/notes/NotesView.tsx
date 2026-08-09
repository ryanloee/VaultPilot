import { useEffect, useState } from "react";
import { useNotesStore } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Markdown } from "@/components/chat/Markdown";
import { cn } from "@/lib/utils";

export function NotesView() {
  const { notes, current, loading, error, loadList, open, saveCurrent } = useNotesStore();
  const [editing, setEditing] = useState(false);
  const [draftBody, setDraftBody] = useState("");
  const [draftTitle, setDraftTitle] = useState("");

  useEffect(() => {
    loadList();
  }, [loadList]);

  const handleOpen = async (id: string) => {
    await open(id);
    setEditing(false);
  };

  const handleEdit = () => {
    if (!current) return;
    setDraftBody(current.body);
    setDraftTitle(current.meta.title);
    setEditing(true);
  };

  const handleSave = async () => {
    await saveCurrent(draftBody, draftTitle);
    setEditing(false);
    await loadList();
  };

  const formatDate = (iso?: string) => {
    if (!iso) return "";
    try {
      return new Date(iso).toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" });
    } catch {
      return "";
    }
  };

  return (
    <div className="flex h-full">
      {/* Notes list */}
      <ScrollArea className="w-64 shrink-0 border-r border-border">
        <div className="border-b border-border px-3 py-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">
          笔记 ({notes.length})
        </div>
        {loading && <p className="px-3 py-2 text-xs text-muted-foreground">加载中…</p>}
        {notes.map((n) => (
          <button
            key={n.id}
            onClick={() => handleOpen(n.id)}
            className={cn(
              "block w-full border-b border-border px-3 py-2 text-left transition-colors hover:bg-accent",
              current?.meta.id === n.id && "bg-accent"
            )}
          >
            <div className="truncate text-sm font-medium">{n.title || "无标题"}</div>
            <div className="mt-0.5 flex items-center gap-2 text-[10px] text-muted-foreground">
              <span>{formatDate(n.updatedAt ?? n.createdAt)}</span>
              {n.tags && n.tags.length > 0 && (
                <span className="truncate">{n.tags.slice(0, 3).join(", ")}</span>
              )}
            </div>
          </button>
        ))}
        {!loading && notes.length === 0 && (
          <p className="px-3 py-4 text-center text-xs text-muted-foreground">暂无笔记</p>
        )}
      </ScrollArea>

      {/* Note detail / editor */}
      <div className="flex min-w-0 flex-1 flex-col">
        {error && (
          <p className="bg-destructive/10 px-4 py-2 text-xs text-destructive">{error}</p>
        )}
        {!current ? (
          <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
            从左侧选择一篇笔记
          </div>
        ) : editing ? (
          <div className="flex h-full flex-col p-4 gap-3">
            <Input
              value={draftTitle}
              onChange={(e) => setDraftTitle(e.target.value)}
              placeholder="标题"
              className="text-base font-semibold"
            />
            <Textarea
              value={draftBody}
              onChange={(e) => setDraftBody(e.target.value)}
              className="flex-1 font-mono text-sm"
              placeholder="笔记内容（Markdown）"
            />
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setEditing(false)}>
                取消
              </Button>
              <Button onClick={handleSave}>保存</Button>
            </div>
          </div>
        ) : (
          <div className="flex h-full flex-col">
            <div className="flex items-center justify-between border-b border-border px-4 py-2">
              <h1 className="truncate text-lg font-semibold">
                {current.meta.title || "无标题"}
              </h1>
              <Button variant="ghost" size="sm" onClick={handleEdit}>
                编辑
              </Button>
            </div>
            <ScrollArea className="flex-1">
              <article className="mx-auto max-w-3xl p-6">
                {current.body ? (
                  <Markdown content={current.body} />
                ) : (
                  <p className="text-sm text-muted-foreground">（空笔记）</p>
                )}
              </article>
            </ScrollArea>
          </div>
        )}
      </div>
    </div>
  );
}
