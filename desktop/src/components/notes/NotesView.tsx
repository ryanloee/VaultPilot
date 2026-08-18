import { useEffect, useState } from "react";
import { useNotesStore, useSettingsStore } from "@/lib/store";
import { api } from "@/lib/tauri";
import type { BacklinkEntry, Collection, NoteMeta } from "@/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { Input } from "@/components/ui/input";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Markdown } from "@/components/chat/Markdown";
import { TrashIcon } from "@/components/layout/icons";
import { cn, formatDate } from "@/lib/utils";
import { CollectionTree } from "./CollectionTree";

export function NotesView() {
  const { notes, current, loading, error, loadList, open, saveCurrent, clearCurrent } =
    useNotesStore();
  const settings = useSettingsStore((s) => s.settings);
  const loadSettings = useSettingsStore((s) => s.load);
  const [editing, setEditing] = useState(false);
  const [mobileDetail, setMobileDetail] = useState(false);
  const [draftBody, setDraftBody] = useState("");
  const [draftTitle, setDraftTitle] = useState("");
  const [backlinks, setBacklinks] = useState<BacklinkEntry[]>([]);
  const [backlinksLoading, setBacklinksLoading] = useState(false);
  const [collections, setCollections] = useState<Collection[]>([]);
  const [selectedCollectionId, setSelectedCollectionId] = useState<string | null>(null);
  const [collectionNotes, setCollectionNotes] = useState<NoteMeta[] | null>(null);
  const [noteCollectionIds, setNoteCollectionIds] = useState<string[]>([]);

  useEffect(() => {
    loadList();
  }, [loadList]);

  // Ensure the heading-numbering toggle is known even if the user never
  // opened the settings view (#4062).
  useEffect(() => {
    loadSettings();
  }, [loadSettings]);

  // Load backlinks for the opened note (#4061).
  useEffect(() => {
    if (!current) {
      setBacklinks([]);
      return;
    }
    let cancelled = false;
    setBacklinksLoading(true);
    api
      .findBacklinks(current.meta.id)
      .then((b) => {
        if (!cancelled) setBacklinks(b);
      })
      .catch(() => {
        if (!cancelled) setBacklinks([]);
      })
      .finally(() => {
        if (!cancelled) setBacklinksLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [current?.meta.id]);

  // Load collections (hierarchical grouping, #2042).
  const reloadCollections = () => {
    api
      .listCollections()
      .then(setCollections)
      .catch(() => setCollections([]));
  };
  useEffect(() => {
    reloadCollections();
  }, []);

  // Load notes for the selected collection.
  useEffect(() => {
    if (!selectedCollectionId) {
      setCollectionNotes(null);
      return;
    }
    let cancelled = false;
    api
      .listNotesInCollection(selectedCollectionId, 500)
      .then((r) => {
        if (!cancelled) setCollectionNotes(r.notes);
      })
      .catch(() => {
        if (!cancelled) setCollectionNotes([]);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedCollectionId]);

  // Load which collections the opened note belongs to.
  useEffect(() => {
    if (!current) {
      setNoteCollectionIds([]);
      return;
    }
    let cancelled = false;
    api
      .getCollectionsForNote(current.meta.id)
      .then((cols) => {
        if (!cancelled) setNoteCollectionIds(cols.map((c) => c.id));
      })
      .catch(() => {
        if (!cancelled) setNoteCollectionIds([]);
      });
    return () => {
      cancelled = true;
    };
  }, [current?.meta.id]);

  const handleOpen = async (id: string) => {
    await open(id);
    setEditing(false);
    setMobileDetail(true);
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

  const handleDelete = async () => {
    if (!current) return;
    if (!window.confirm(`确定删除笔记「${current.meta.title || "无标题"}」吗？`)) return;
    try {
      await api.deleteNote(current.meta.id);
      clearCurrent();
      await loadList();
    } catch (e) {
      alert(`删除失败：${String(e)}`);
    }
  };

  const handleToggleCollection = async (collectionId: string) => {
    if (!current) return;
    const isMember = noteCollectionIds.includes(collectionId);
    try {
      if (isMember) {
        await api.removeNoteFromCollection(current.meta.id, collectionId);
        setNoteCollectionIds((ids) => ids.filter((id) => id !== collectionId));
      } else {
        await api.addNoteToCollection(current.meta.id, collectionId);
        setNoteCollectionIds((ids) => [...ids, collectionId]);
      }
      reloadCollections();
    } catch (e) {
      alert(`更新分类失败：${String(e)}`);
    }
  };

  const handleSelectCollection = (id: string | null) => {
    setSelectedCollectionId(id);
    if (id === null) void loadList();
  };

  const displayedNotes = selectedCollectionId ? collectionNotes ?? [] : notes;

  return (
    <div className="flex h-full flex-col md:flex-row">
      {/* Notes list */}
      <ScrollArea
        className={cn(
          "w-full shrink-0 border-r border-border md:w-72",
          mobileDetail && "hidden md:block"
        )}
      >
        {/* Hierarchical collections tree (#2042) */}
        <CollectionTree
          collections={collections}
          selectedId={selectedCollectionId}
          onSelect={handleSelectCollection}
          onChange={reloadCollections}
        />
        <button
          onClick={() => handleSelectCollection(null)}
          className={cn(
            "block w-full border-b border-border px-3 py-2 text-left text-xs font-medium uppercase tracking-wide text-muted-foreground transition-colors hover:bg-accent",
            selectedCollectionId === null && "bg-accent text-foreground"
          )}
        >
          {selectedCollectionId
            ? `← 全部笔记 (${notes.length})`
            : `全部笔记 (${notes.length})`}
        </button>
        {loading && <p className="px-3 py-2 text-xs text-muted-foreground">加载中…</p>}
        {displayedNotes.map((n) => (
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
        {!loading && displayedNotes.length === 0 && (
          <p className="px-3 py-4 text-center text-xs text-muted-foreground">
            {selectedCollectionId ? "该分类下暂无笔记" : "暂无笔记"}
          </p>
        )}
      </ScrollArea>

      {/* Note detail / editor */}
      <div
        className={cn(
          "flex min-w-0 flex-1 flex-col",
          !mobileDetail && "hidden md:flex"
        )}
      >
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
              <div className="flex min-w-0 items-center gap-2">
                <Button
                  variant="ghost"
                  size="icon"
                  className="md:hidden"
                  onClick={() => setMobileDetail(false)}
                  title="返回列表"
                >
                  ‹
                </Button>
                <h1 className="truncate text-lg font-semibold">
                  {current.meta.title || "无标题"}
                </h1>
              </div>
              <div className="flex items-center gap-1">
                <Button variant="ghost" size="sm" onClick={handleEdit}>
                  编辑
                </Button>
                <Button
                  variant="ghost"
                  size="icon"
                  onClick={() => void handleDelete()}
                  title="删除笔记"
                  className="text-muted-foreground hover:text-destructive"
                >
                  <TrashIcon className="h-4 w-4" />
                </Button>
              </div>
            </div>
            <ScrollArea className="flex-1">
              <article className="mx-auto max-w-3xl p-4 md:p-6">
                {current.body ? (
                  <Markdown content={current.body} numberHeadings={settings?.headingNumbering} />
                ) : (
                  <p className="text-sm text-muted-foreground">（空笔记）</p>
                )}

                {/* Collections membership (#2042) */}
                <section className="mt-6 border-t border-border pt-4">
                  <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    所属分类
                  </h2>
                  {collections.length === 0 ? (
                    <p className="mt-2 text-xs text-muted-foreground">
                      暂无分类 — 在左侧面板「+ 新建」创建
                    </p>
                  ) : (
                    <ul className="mt-2 flex flex-wrap gap-2">
                      {collections.map((c) => {
                        const checked = noteCollectionIds.includes(c.id);
                        return (
                          <li key={c.id}>
                            <button
                              onClick={() => void handleToggleCollection(c.id)}
                              className={cn(
                                "rounded-full border px-3 py-1 text-xs transition-colors",
                                checked
                                  ? "border-primary bg-primary/10 text-primary"
                                  : "border-border text-muted-foreground hover:border-primary/50 hover:text-foreground"
                              )}
                              title={checked ? "移出分类" : "加入分类"}
                            >
                              {checked ? "✓ " : "+ "}
                              {c.name}
                            </button>
                          </li>
                        );
                      })}
                    </ul>
                  )}
                </section>

                {/* Backlinks panel (#4061) */}
                <section className="mt-6 border-t border-border pt-4">
                  <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                    引用本笔记（{backlinks.length}）
                  </h2>
                  {backlinksLoading && (
                    <p className="mt-2 text-xs text-muted-foreground">加载中…</p>
                  )}
                  {!backlinksLoading && backlinks.length === 0 && (
                    <p className="mt-2 text-xs text-muted-foreground">暂无笔记引用本文</p>
                  )}
                  <ul className="mt-2 space-y-1">
                    {backlinks.map((b) => (
                      <li key={b.meta.id} className="flex items-baseline gap-2">
                        <button
                          onClick={() => handleOpen(b.meta.id)}
                          className="text-sm text-primary hover:underline"
                        >
                          {b.meta.title || "无标题"}
                        </button>
                        <span className="text-[10px] text-muted-foreground">
                          {formatDate(b.meta.updatedAt ?? b.meta.createdAt)}
                        </span>
                      </li>
                    ))}
                  </ul>
                </section>
              </article>
            </ScrollArea>
          </div>
        )}
      </div>
    </div>
  );
}
