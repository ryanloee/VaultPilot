import { useMemo, useState } from "react";
import { api } from "@/lib/tauri";
import type { Collection } from "@/types";
import { cn } from "@/lib/utils";

type CollectionTreeProps = {
  collections: Collection[];
  selectedId: string | null;
  onSelect: (id: string | null) => void;
  onChange: () => void;
};

/** Recursive tree of collections built from the flat list. */
type TreeNode = {
  collection: Collection;
  children: TreeNode[];
};

function buildTree(collections: Collection[]): TreeNode[] {
  const byParent = new Map<string, TreeNode[]>();
  for (const c of collections) {
    const parent = c.parentId ?? "";
    if (!byParent.has(parent)) byParent.set(parent, []);
    byParent.get(parent)!.push({ collection: c, children: [] });
  }
  const attach = (nodes: TreeNode[]) => {
    for (const n of nodes) {
      const kids = byParent.get(n.collection.id);
      if (kids) {
        n.children = kids;
        attach(kids);
      }
    }
  };
  const roots = byParent.get("") ?? [];
  attach(roots);
  return roots;
}

export function CollectionTree({
  collections,
  selectedId,
  onSelect,
  onChange,
}: CollectionTreeProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [creating, setCreating] = useState<{ parentId: string; name: string } | null>(null);
  const [renaming, setRenaming] = useState<{ id: string; name: string } | null>(null);

  const tree = useMemo(() => buildTree(collections), [collections]);

  const toggle = (id: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const startCreate = (parentId: string) => {
    setCreating({ parentId, name: "" });
    setRenaming(null);
  };

  const submitCreate = async () => {
    if (!creating || !creating.name.trim()) {
      setCreating(null);
      return;
    }
    try {
      await api.createCollection(creating.name.trim(), "", creating.parentId || undefined);
      setCreating(null);
      onChange();
    } catch (e) {
      alert(`新建分类失败：${String(e)}`);
    }
  };

  const startRename = (id: string, name: string) => {
    setRenaming({ id, name });
    setCreating(null);
  };

  const submitRename = async () => {
    if (!renaming) return;
    if (!renaming.name.trim()) {
      setRenaming(null);
      return;
    }
    try {
      await api.renameCollection(renaming.id, renaming.name.trim());
      setRenaming(null);
      onChange();
    } catch (e) {
      alert(`重命名失败：${String(e)}`);
    }
  };

  const handleDelete = async (c: Collection) => {
    const childCount = collections.filter((x) => x.parentId === c.id).length;
    const msg = childCount > 0
      ? `删除分类「${c.name}」将同时删除其 ${childCount} 个子分类（笔记不受影响）。确定？`
      : `确定删除分类「${c.name}」？（笔记不受影响）`;
    if (!window.confirm(msg)) return;
    try {
      await api.deleteCollection(c.id);
      if (selectedId === c.id) onSelect(null);
      onChange();
    } catch (e) {
      alert(`删除失败：${String(e)}`);
    }
  };

  const handleDrop = async (targetId: string, e: React.DragEvent) => {
    e.preventDefault();
    const sourceId = e.dataTransfer.getData("text/plain");
    if (!sourceId || sourceId === targetId) return;
    try {
      await api.moveCollection(sourceId, targetId);
      onChange();
    } catch (err) {
      alert(`移动失败：${String(err)}`);
    }
  };

  const renderNode = (node: TreeNode, depth: number) => {
    const c = node.collection;
    const hasChildren = node.children.length > 0;
    const isCollapsed = collapsed.has(c.id);
    const isSelected = selectedId === c.id;
    const isRenaming = renaming?.id === c.id;

    return (
      <div key={c.id}>
        <div
          draggable
          onDragStart={(e) => e.dataTransfer.setData("text/plain", c.id)}
          onDragOver={(e) => e.preventDefault()}
          onDrop={(e) => handleDrop(c.id, e)}
          className={cn(
            "group flex cursor-pointer items-center gap-1 rounded px-2 py-1 text-sm transition-colors hover:bg-accent",
            isSelected && "bg-accent"
          )}
          style={{ paddingLeft: `${depth * 12 + 8}px` }}
          onClick={() => onSelect(c.id)}
        >
          {hasChildren ? (
            <button
              onClick={(e) => {
                e.stopPropagation();
                toggle(c.id);
              }}
              className="h-4 w-4 shrink-0 text-muted-foreground"
              title={isCollapsed ? "展开" : "折叠"}
            >
              {isCollapsed ? "▸" : "▾"}
            </button>
          ) : (
            <span className="h-4 w-4 shrink-0" />
          )}
          {isRenaming ? (
            <input
              autoFocus
              value={renaming!.name}
              onChange={(e) => setRenaming({ id: c.id, name: e.target.value })}
              onBlur={() => void submitRename()}
              onKeyDown={(e) => {
                if (e.key === "Enter") void submitRename();
                if (e.key === "Escape") setRenaming(null);
              }}
              className="w-full min-w-0 rounded border border-input bg-background px-1 py-0.5 text-sm focus:outline-none"
              onClick={(e) => e.stopPropagation()}
            />
          ) : (
            <span className="min-w-0 flex-1 truncate">{c.name}</span>
          )}
          {typeof c.noteCount === "number" && c.noteCount > 0 && !isRenaming && (
            <span className="shrink-0 text-[10px] text-muted-foreground">{c.noteCount}</span>
          )}
          <span className="hidden shrink-0 gap-0.5 group-hover:flex">
            <button
              onClick={(e) => {
                e.stopPropagation();
                startCreate(c.id);
              }}
              className="h-4 w-4 rounded text-muted-foreground hover:text-foreground"
              title="新建子分类"
            >
              +
            </button>
            <button
              onClick={(e) => {
                e.stopPropagation();
                startRename(c.id, c.name);
              }}
              className="h-4 w-4 rounded text-muted-foreground hover:text-foreground"
              title="重命名"
            >
              ✎
            </button>
            <button
              onClick={(e) => {
                e.stopPropagation();
                void handleDelete(c);
              }}
              className="h-4 w-4 rounded text-muted-foreground hover:text-destructive"
              title="删除"
            >
              ✕
            </button>
          </span>
        </div>
        {creating?.parentId === c.id && (
          <div
            className="flex items-center gap-1 px-2 py-1"
            style={{ paddingLeft: `${(depth + 1) * 12 + 8}px` }}
          >
            <input
              autoFocus
              value={creating.name}
              onChange={(e) => setCreating({ parentId: c.id, name: e.target.value })}
              onBlur={() => void submitCreate()}
              onKeyDown={(e) => {
                if (e.key === "Enter") void submitCreate();
                if (e.key === "Escape") setCreating(null);
              }}
              placeholder="分类名称"
              className="w-full min-w-0 rounded border border-input bg-background px-1 py-0.5 text-sm focus:outline-none"
            />
          </div>
        )}
        {hasChildren && !isCollapsed && (
          <div>{node.children.map((child) => renderNode(child, depth + 1))}</div>
        )}
      </div>
    );
  };

  return (
    <div className="flex flex-col">
      <div className="flex items-center justify-between border-b border-border px-3 py-2">
        <span className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          分类
        </span>
        <button
          onClick={() => startCreate("")}
          className="text-xs text-muted-foreground hover:text-foreground"
          title="新建根分类"
        >
          + 新建
        </button>
      </div>
      <div
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          const sourceId = e.dataTransfer.getData("text/plain");
          if (!sourceId) return;
          void api.moveCollection(sourceId, "").then(() => onChange());
        }}
      >
        {tree.map((node) => renderNode(node, 0))}
        {creating?.parentId === "" && (
          <div className="flex items-center gap-1 px-3 py-1">
            <input
              autoFocus
              value={creating.name}
              onChange={(e) => setCreating({ parentId: "", name: e.target.value })}
              onBlur={() => void submitCreate()}
              onKeyDown={(e) => {
                if (e.key === "Enter") void submitCreate();
                if (e.key === "Escape") setCreating(null);
              }}
              placeholder="分类名称"
              className="w-full min-w-0 rounded border border-input bg-background px-1 py-0.5 text-sm focus:outline-none"
            />
          </div>
        )}
        {tree.length === 0 && creating?.parentId !== "" && (
          <p className="px-3 py-2 text-xs text-muted-foreground">
            暂无分类 — 点击右上角「+ 新建」创建
          </p>
        )}
      </div>
    </div>
  );
}