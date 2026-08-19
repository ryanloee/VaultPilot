import { useEffect, useMemo, useState } from "react";
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

const DAY_LABELS = ["日", "一", "二", "三", "四", "五", "六"];

type WeekdayPreset = "every" | "weekdays" | "weekends" | "custom";
const WEEKDAY_PRESETS: { id: WeekdayPreset; label: string; days: number[] | null }[] = [
  { id: "every", label: "每天", days: null },
  { id: "weekdays", label: "工作日", days: [1, 2, 3, 4, 5] },
  { id: "weekends", label: "周末", days: [0, 6] },
  { id: "custom", label: "自选", days: null },
];

/** Generate a 5-field cron expression from UI state. */
function toCron(minute: number, hour: number, days: number[]): string {
  if (days.length === 7) return `${minute} ${hour} * * *`;
  return `${minute} ${hour} * * ${days.join(",")}`;
}

/** Parse a cron expression back to UI state (best-effort). */
function fromCron(expr: string): { minute: number; hour: number; days: number[] } {
  const parts = expr.trim().split(/\s+/);
  const minute = parseInt(parts[0] ?? "0", 10) || 0;
  const hour = parseInt(parts[1] ?? "8", 10) || 8;
  let days: number[] = [];
  if (parts[4] && parts[4] !== "*") {
    days = parts[4].split(",").map((d) => parseInt(d, 10)).filter((n) => !isNaN(n));
  } else {
    days = [0, 1, 2, 3, 4, 5, 6];
  }
  return { minute, hour, days };
}

/** Format cron to human-readable Chinese. */
function cronToLabel(expr: string): string {
  const { minute, hour, days } = fromCron(expr);
  const time = `${String(hour).padStart(2, "0")}:${String(minute).padStart(2, "0")}`;
  if (days.length === 7) return `每天 ${time}`;
  if (days.length === 5 && days.every((d) => d >= 1 && d <= 5)) return `工作日 ${time}`;
  if (days.length === 2 && days.includes(0) && days.includes(6)) return `周末 ${time}`;
  const dayStr = days.sort().map((d) => `周${DAY_LABELS[d]}`).join("、");
  return `${dayStr} ${time}`;
}

function ScrollPicker({
  value,
  onChange,
  items,
  className,
}: {
  value: number;
  onChange: (v: number) => void;
  items: { value: number; label: string }[];
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col gap-0.5", className)}>
      <button
        onClick={() => {
          const idx = items.findIndex((i) => i.value === value);
          onChange(items[(idx - 1 + items.length) % items.length].value);
        }}
        className="text-xs text-muted-foreground hover:text-foreground"
      >
        ▲
      </button>
      <div className="rounded-md border border-border bg-background px-3 py-1.5 text-center text-sm font-mono min-w-[3rem]">
        {String(value).padStart(2, "0")}
      </div>
      <button
        onClick={() => {
          const idx = items.findIndex((i) => i.value === value);
          onChange(items[(idx + 1) % items.length].value);
        }}
        className="text-xs text-muted-foreground hover:text-foreground"
      >
        ▼
      </button>
    </div>
  );
}

export function TriggerView() {
  const [rules, setRules] = useState<TriggerRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [newLabel, setNewLabel] = useState("");
  const [newHour, setNewHour] = useState(8);
  const [newMinute, setNewMinute] = useState(0);
  const [weekdayPreset, setWeekdayPreset] = useState<WeekdayPreset>("every");
  const [customDays, setCustomDays] = useState<number[]>([1, 2, 3, 4, 5]);
  const [newAction, setNewAction] = useState("daily_review");
  const [newPrompt, setNewPrompt] = useState("");
  const [creating, setCreating] = useState(false);

  const selectedDays = useMemo(() => {
    const preset = WEEKDAY_PRESETS.find((p) => p.id === weekdayPreset);
    if (preset?.days) return preset.days;
    return customDays;
  }, [weekdayPreset, customDays]);

  const cronExpr = useMemo(() => toCron(newMinute, newHour, selectedDays), [newMinute, newHour, selectedDays]);
  const cronLabel = useMemo(() => cronToLabel(cronExpr), [cronExpr]);

  const hours = Array.from({ length: 24 }, (_, i) => ({ value: i, label: String(i).padStart(2, "0") }));
  const minutes = Array.from({ length: 12 }, (_, i) => ({ value: i * 5, label: String(i * 5).padStart(2, "0") }));

  const toggleCustomDay = (day: number) => {
    setCustomDays((prev) =>
      prev.includes(day) ? prev.filter((d) => d !== day) : [...prev, day].sort()
    );
  };

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

  useEffect(() => { load(); }, []);

  const handleCreate = async () => {
    if (!newLabel.trim() || selectedDays.length === 0) return;
    setCreating(true);
    try {
      await api.createTriggerRule(
        newLabel.trim(),
        "cron",
        cronExpr,
        newAction,
        undefined,
        newAction === "custom" ? newPrompt.trim() || undefined : undefined
      );
      setAddOpen(false);
      setNewLabel("");
      setNewHour(8);
      setNewMinute(0);
      setWeekdayPreset("every");
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
      setRules((prev) => prev.map((r) => (r.id === id ? { ...r, enabled: !r.enabled } : r)));
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
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">加载中…</div>
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
          <button onClick={() => setError(null)} className="ml-2 underline">关闭</button>
        </div>
      )}

      {/* New rule form */}
      {addOpen && (
        <div className="border-b border-border bg-card/50 p-4 space-y-4">
          <input
            value={newLabel}
            onChange={(e) => setNewLabel(e.target.value)}
            placeholder="规则名称（如：早间回顾）"
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          />

          {/* Time picker */}
          <div>
            <label className="text-xs text-muted-foreground mb-1 block">执行时间</label>
            <div className="flex items-center gap-1">
              <ScrollPicker value={newHour} onChange={setNewHour} items={hours} />
              <span className="text-lg font-bold text-muted-foreground">:</span>
              <ScrollPicker value={newMinute} onChange={setNewMinute} items={minutes} />
            </div>
          </div>

          {/* Weekday selector */}
          <div>
            <label className="text-xs text-muted-foreground mb-1 block">执行日</label>
            <div className="flex gap-1.5 flex-wrap">
              {WEEKDAY_PRESETS.map((p) => (
                <button
                  key={p.id}
                  onClick={() => setWeekdayPreset(p.id)}
                  className={cn(
                    "rounded-md border border-border px-2.5 py-1 text-xs transition-colors",
                    weekdayPreset === p.id
                      ? "bg-primary text-primary-foreground"
                      : "bg-background hover:bg-accent"
                  )}
                >
                  {p.label}
                </button>
              ))}
            </div>
            {weekdayPreset === "custom" && (
              <div className="flex gap-1.5 mt-2">
                {DAY_LABELS.map((label, i) => (
                  <button
                    key={i}
                    onClick={() => toggleCustomDay(i)}
                    className={cn(
                      "h-8 w-8 rounded-full border border-border text-xs transition-colors",
                      customDays.includes(i)
                        ? "bg-primary text-primary-foreground"
                        : "bg-background hover:bg-accent"
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
            )}
          </div>

          {/* Preview */}
          <div className="rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
            📅 {cronLabel}
          </div>

          {/* Action */}
          <select
            value={newAction}
            onChange={(e) => setNewAction(e.target.value)}
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          >
            {Object.entries(ACTION_LABELS).map(([k, v]) => (
              <option key={k} value={k}>{v}</option>
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
            disabled={creating || !newLabel.trim() || selectedDays.length === 0}
          >
            {creating ? "创建中…" : "创建规则"}
          </Button>
        </div>
      )}

      {/* Rules list */}
      <div className="flex-1 overflow-auto vp-scroll p-4 space-y-2">
        {rules.length === 0 && (
          <p className="text-center text-sm text-muted-foreground py-8">暂无定时规则</p>
        )}
        {rules.map((rule) => (
          <div
            key={rule.id}
            className={cn(
              "flex items-center gap-3 rounded-lg border border-border p-3 transition-colors",
              rule.enabled ? "bg-card" : "bg-card/50 opacity-60"
            )}
          >
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
            <div className="min-w-0 flex-1">
              <div className="text-sm font-medium truncate">{rule.label}</div>
              <div className="text-xs text-muted-foreground">
                {rule.triggerType === "cron" ? cronToLabel(rule.triggerConfig) : `📡 ${rule.triggerConfig}`}
                {" · "}
                {ACTION_LABELS[rule.action] ?? rule.action}
              </div>
            </div>
            <button
              onClick={() => handleDelete(rule.id)}
              className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
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
