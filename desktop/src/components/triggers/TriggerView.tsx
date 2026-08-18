import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import type { TriggerRule } from "@/types";
import { Button } from "@/components/ui/button";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

const ACTION_LABELS: Record<string, string> = {
  daily_review: "每日回顾",
  summarize_and_tag: "摘要 & 标签",
  suggest_links: "建议链接",
  process_webhook: "处理 Webhook",
  custom: "自定义提示词",
};

const CRON_PRESETS = [
  { label: "每天 8:00", expr: "0 8 * * *" },
  { label: "每天 18:00", expr: "0 18 * * *" },
  { label: "每周一 9:00", expr: "0 9 * * 1" },
  { label: "每小时", expr: "0 * * * *" },
];

export function TriggerView() {
  const [rules, setRules] = useState<TriggerRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newCron, setNewCron] = useState("0 8 * * *");
  const [newAction, setNewAction] = useState("daily_review");
  const [newPrompt, setNewPrompt] = useState("");
  const [creating, setCreating] = useState(false);

  const load = async () => {
    try {
      const list = await api.listTriggerRules();
      setRules(list);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const handleCreate = async () => {
    if (!newLabel.trim()) return;
    setCreating(true);
    try {
      await api.createTriggerRule(
        newLabel.trim(),
        "cron",
        newCron,
        newAction,
        undefined,
        newAction === "custom" ? newPrompt.trim() || undefined : undefined
      );
      setAddOpen(false);
      setNewLabel("");
      setNewCron("0 8 * * *");
      setNewAction("daily_review");
      setNewPrompt("");
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  const handleToggle = async (id: string) => {
    try {
      await api.toggleTriggerRule(id);
      setRules((prev) =>
        prev.map((r) => (r.id === id ? { ...r, enabled: !r.enabled } : r))
      );
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.deleteTriggerRule(id);
      setRules((prev) => prev.filter((r) => r.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        加载中…
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center justify-between border-b border-border px-4 py-3">
        <h2 className="text-sm font-semibold">定时唤醒</h2>
        <Button size="sm" onClick={() => setAddOpen((v) => !v)}>
          {addOpen ? "取消" : "+ 新建"}
        </Button>
      </div>

      {error && (
        <div className="mx-4 mt-2 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
          <button onClick={() => setError(null)} className="ml-2 underline">
            关闭
          </button>
        </div>
      )}

      {/* New rule form */}
      {addOpen && (
        <div className="border-b border-border bg-card/50 p-4 space-y-3">
          <input
            value={newLabel}
            onChange={(e) => setNewLabel(e.target.value)}
            placeholder="规则名称"
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          />
          <div className="flex gap-2 flex-wrap">
            {CRON_PRESETS.map((p) => (
              <button
                key={p.expr}
                onClick={() => setNewCron(p.expr)}
                className={cn(
                  "rounded-md border border-border px-2 py-1 text-xs transition-colors",
                  newCron === p.expr
                    ? "bg-primary text-primary-foreground"
                    : "bg-background hover:bg-accent"
                )}
              >
                {p.label}
              </button>
            ))}
          </div>
          <input
            value={newCron}
            onChange={(e) => setNewCron(e.target.value)}
            placeholder="Cron 表达式 (分 时 日 月 周)"
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm font-mono"
          />
          <select
            value={newAction}
            onChange={(e) => setNewAction(e.target.value)}
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          >
            {Object.entries(ACTION_LABELS).map(([k, v]) => (
              <option key={k} value={k}>
                {v}
              </option>
            ))}
          </select>
          {newAction === "custom" && (
            <Textarea
              value={newPrompt}
              onChange={(e) => setNewPrompt(e.target.value)}
              placeholder="自定义提示词…"
              rows={3}
              className="text-sm"
            />
          )}
          <Button
            size="sm"
            onClick={handleCreate}
            disabled={creating || !newLabel.trim()}
          >
            {creating ? "创建中…" : "创建规则"}
          </Button>
        </div>
      )}

      {/* Rules list */}
      <div className="flex-1 overflow-auto vp-scroll p-4 space-y-2">
        {rules.length === 0 && (
          <p className="text-center text-sm text-muted-foreground py-8">
            暂无定时规则
          </p>
        )}
        {rules.map((rule) => (
          <div
            key={rule.id}
            className={cn(
              "flex items-center gap-3 rounded-lg border border-border p-3 transition-colors",
              rule.enabled ? "bg-card" : "bg-card/50 opacity-60"
            )}
          >
            {/* Toggle */}
            <button
              onClick={() => handleToggle(rule.id)}
              className={cn(
                "h-5 w-9 shrink-0 rounded-full transition-colors relative",
                rule.enabled ? "bg-primary" : "bg-muted"
              )}
            >
              <span
                className={cn(
                  "absolute top-0.5 h-4 w-4 rounded-full bg-white transition-transform",
                  rule.enabled ? "left-[18px]" : "left-0.5"
                )}
              />
            </button>

            {/* Info */}
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium truncate">{rule.label}</div>
              <div className="text-xs text-muted-foreground">
                {rule.triggerType === "cron" ? `⏰ ${rule.triggerConfig}` : `📡 ${rule.triggerConfig}`}
                {" · "}
                {ACTION_LABELS[rule.action] ?? rule.action}
              </div>
            </div>

            {/* Delete */}
            <button
              onClick={() => handleDelete(rule.id)}
              className="shrink-0 rounded p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-destructive/10 hover:text-destructive group-hover:opacity-100"
              title="删除"
            >
              ✕
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
