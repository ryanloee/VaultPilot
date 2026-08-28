import { useEffect, useMemo, useState } from "react";
import { api } from "@/lib/tauri";
import type { TriggerExecution, TriggerRule } from "@/types";
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

/**
 * Timezone handling: the backend executor evaluates cron schedules in UTC,
 * but the picker collects local wall-clock time. These helpers convert
 * between the two so "每天 08:00" means 08:00 where the user actually is.
 * Day-of-week sets can shift across midnight during conversion (e.g. local
 * Mon 00:30 in UTC+8 is Sun 16:30 UTC).
 */

/** Next local date (starting today) whose weekday matches `day`. */
function nextLocalDateWithWeekday(day: number): Date {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  for (let i = 0; i < 7; i++) {
    if (d.getDay() === day) return new Date(d);
    d.setDate(d.getDate() + 1);
  }
  return d;
}

/** Convert local HH:MM on `day` (null = any day) to UTC HH:MM + weekday. */
function localToUtc(hour: number, minute: number, day: number | null): { h: number; m: number; d: number | null } {
  const base = day === null ? new Date() : nextLocalDateWithWeekday(day);
  base.setHours(hour, minute, 0, 0);
  return { h: base.getUTCHours(), m: base.getUTCMinutes(), d: day === null ? null : base.getUTCDay() };
}

/** Convert UTC HH:MM on `day` (null = any day) to local HH:MM + weekday. */
function utcToLocal(hour: number, minute: number, day: number | null): { h: number; m: number; d: number | null } {
  const base = new Date();
  const target = day === null ? base.getUTCDay() : day;
  for (let i = 0; i < 7; i++) {
    if (base.getUTCDay() === target) break;
    base.setUTCDate(base.getUTCDate() + 1);
  }
  base.setUTCHours(hour, minute, 0, 0);
  return { h: base.getHours(), m: base.getMinutes(), d: day === null ? null : base.getDay() };
}

/** Generate a 5-field UTC cron expression from local UI state. */
export function toCron(minute: number, hour: number, days: number[]): string {
  // Stored expressions are STANDARD cron dow (0/7=Sunday, 1=Monday..6=
  // Saturday). The Rust side (normalize_cron_expr) translates numeric dow
  // to day names because the `cron` crate's own numbering is nonstandard
  // (1=Sunday..7=Saturday, 0 rejected) — do NOT "fix" numbers for the
  // crate here (#4086). JS day numbers (0=Sun, 1-6=Mon-Sat) map to
  // standard cron as 0 and 1-6; we emit 7 instead of 0 only because both
  // mean Sunday in standard cron and 7 reads unambiguously.
  // parseDowField reverses this: cron 7 → JS 0 via `% 7`.
  const toCronDay = (jsDay: number): number => (jsDay === 0 ? 7 : jsDay);
  if (days.length === 7) {
    const { h, m } = localToUtc(hour, minute, null);
    return `${m} ${h} * * *`;
  }
  const utcDays = new Set<number>();
  let uh = -1;
  let um = -1;
  for (const d of days) {
    const s = localToUtc(hour, minute, d);
    if (s.d !== null) utcDays.add(toCronDay(s.d));
    // Constant-offset timezones give identical times for every day; DST
    // zones can differ around transitions — take the first day's time.
    if (uh < 0) {
      uh = s.h;
      um = s.m;
    }
  }
  const list = [...utcDays].sort((a, b) => a - b);
  if (list.length === 7) return `${um} ${uh} * * *`;
  return `${um} ${uh} * * ${list.join(",")}`;
}

/** Parse a cron weekday field ("1,3-5", "1-5", "*") into a day list. */
function parseDowField(field: string): number[] {
  const days = new Set<number>();
  for (const part of field.split(",")) {
    const range = part.match(/^(\d+)\s*-\s*(\d+)$/);
    if (range) {
      const from = parseInt(range[1], 10);
      const to = parseInt(range[2], 10);
      for (let d = from; d <= to && d <= 7; d++) days.add(d % 7); // cron allows 7 = Sunday
    } else {
      const n = parseInt(part, 10);
      if (!isNaN(n) && n >= 0 && n <= 7) days.add(n % 7);
    }
  }
  return [...days].sort((a, b) => a - b);
}

/** Parse a UTC cron expression back to local UI state (best-effort). */
export function fromCron(expr: string): { minute: number; hour: number; days: number[] } {
  const parts = expr.trim().split(/\s+/);
  // NB: plain `parseInt(x) || fallback` would turn a legitimate 0 (midnight)
  // into the fallback — 0 is falsy.
  const toInt = (s: string | undefined, fallback: number) => {
    const n = s !== undefined ? parseInt(s, 10) : NaN;
    return Number.isFinite(n) ? n : fallback;
  };
  const uMinute = toInt(parts[0], 0);
  const uHour = toInt(parts[1], 8);
  const uDays = parts[4] && parts[4] !== "*" ? parseDowField(parts[4]) : [0, 1, 2, 3, 4, 5, 6];
  if (uDays.length === 7) {
    const { h, m } = utcToLocal(uHour, uMinute, null);
    return { minute: m, hour: h, days: [0, 1, 2, 3, 4, 5, 6] };
  }
  const localDays = new Set<number>();
  let lh = -1;
  let lm = -1;
  for (const d of uDays) {
    const s = utcToLocal(uHour, uMinute, d);
    localDays.add(s.d ?? 0);
    if (lh < 0) {
      lh = s.h;
      lm = s.m;
    }
  }
  const list = [...localDays].sort((a, b) => a - b);
  return { minute: lm, hour: lh, days: list.length === 7 ? [0, 1, 2, 3, 4, 5, 6] : list };
}

/** Detect which weekday preset a parsed day list corresponds to. */
export function detectPreset(days: number[]): WeekdayPreset {
  const sorted = [...days].sort((a, b) => a - b);
  if (sorted.length === 7) return "every";
  if (sorted.length === 5 && sorted.every((d) => d >= 1 && d <= 5)) return "weekdays";
  if (sorted.length === 2 && sorted.includes(0) && sorted.includes(6)) return "weekends";
  return "custom";
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

/** Format an RFC3339 timestamp as local "YYYY-MM-DD HH:MM". */
function fmtTime(iso: string | undefined): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
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
  const [executions, setExecutions] = useState<TriggerExecution[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  /** Rule being edited, or null when the form is in create mode. */
  const [editingRule, setEditingRule] = useState<TriggerRule | null>(null);
  const [newLabel, setNewLabel] = useState("");
  const [newHour, setNewHour] = useState(8);
  const [newMinute, setNewMinute] = useState(0);
  const [weekdayPreset, setWeekdayPreset] = useState<WeekdayPreset>("every");
  const [customDays, setCustomDays] = useState<number[]>([1, 2, 3, 4, 5]);
  const [newAction, setNewAction] = useState("daily_review");
  const [newPrompt, setNewPrompt] = useState("");
  const [creating, setCreating] = useState(false);
  /** Rule currently being fired via the ⚡ button (shows spinner). */
  const [firingRuleId, setFiringRuleId] = useState<string | null>(null);
  /** Execution whose result note is expanded inline. */
  const [expandedExecId, setExpandedExecId] = useState<string | null>(null);
  /** Loaded note body for the expanded execution. */
  const [expandedNoteBody, setExpandedNoteBody] = useState<string | null>(null);
  /** Selected provider name (empty = use active provider). */
  const [providerName, setProviderName] = useState("");
  /** Available providers from settings (loaded once on mount). */
  const [providers, setProviders] = useState<{ name: string; model: string }[]>([]);

  useEffect(() => {
    (async () => {
      try {
        const settings = await api.getSettings();
        setProviders(
          (settings.providers ?? []).map((p: { name: string; model: string }) => ({
            name: p.name,
            model: p.model,
          }))
        );
      } catch {
        // Settings not loadable — the dropdown just shows "默认".
      }
    })();
  }, []);

  const formOpen = addOpen || editingRule !== null;
  /** Event rules keep their trigger config in this UI — only cron schedules
   *  are editable here (matches the create form, which is cron-only). */
  const editingEvent = editingRule !== null && editingRule.triggerType === "event";

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
      const [list, execs] = await Promise.all([
        api.listTriggerRules(),
        api.listTriggerExecutions(30),
      ]);
      setRules(list);
      setExecutions(execs);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { load(); }, []);

  // The backend executor ticks every 60 s — poll at half that cadence so a
  // fire becomes visible here within ~90 s without user action.
  useEffect(() => {
    const timer = setInterval(() => { load(); }, 30_000);
    return () => clearInterval(timer);
  }, []);

  const resetForm = () => {
    setAddOpen(false);
    setEditingRule(null);
    setNewLabel("");
    setNewHour(8);
    setNewMinute(0);
    setWeekdayPreset("every");
    setCustomDays([1, 2, 3, 4, 5]);
    setNewAction("daily_review");
    setNewPrompt("");
    setProviderName("");
  };

  /** Open the form pre-filled with an existing rule. */
  const startEdit = (rule: TriggerRule) => {
    setAddOpen(false);
    setEditingRule(rule);
    setNewLabel(rule.label);
    if (rule.triggerType === "cron") {
      const { minute, hour, days } = fromCron(rule.triggerConfig);
      setNewHour(hour);
      setNewMinute(minute);
      setWeekdayPreset(detectPreset(days));
      setCustomDays(days);
    }
    setNewAction(rule.action);
    setNewPrompt(rule.customPrompt ?? "");
    setProviderName(rule.providerName ?? "");
  };

  const handleSave = async () => {
    if (!newLabel.trim() || (!editingEvent && selectedDays.length === 0)) return;
    // Custom action REQUIRES a prompt — saving without one would create a
    // rule that always fires with error #2842 on every trigger.
    if (newAction === "custom" && !newPrompt.trim()) {
      setError("自定义提示词动作必须填写提示词内容，或改选其他动作。");
      return;
    }
    setCreating(true);
    try {
      const prompt = newAction === "custom" ? newPrompt.trim() || undefined : undefined;
      const pn = providerName.trim() || undefined;
      if (editingRule) {
        await api.updateTriggerRule(
          editingRule.id,
          newLabel.trim(),
          editingEvent ? "event" : "cron",
          editingEvent ? editingRule.triggerConfig : cronExpr,
          newAction,
          editingEvent ? editingRule.filter : undefined,
          prompt,
          pn
        );
      } else {
        await api.createTriggerRule(newLabel.trim(), "cron", cronExpr, newAction, undefined, prompt, pn);
      }
      resetForm();
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
      // Optimistic flip for snappiness, then a silent reload so the status
      // line catches up (a disabled rule has no next-fire time).
      setRules((prev) => prev.map((r) => (r.id === id ? { ...r, enabled: !r.enabled } : r)));
      void load();
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.deleteTriggerRule(id);
      if (editingRule?.id === id) resetForm();
      setRules((prev) => prev.filter((r) => r.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  /** ⚡ Fire a rule right now, bypassing the schedule. */
  const handleFireNow = async (id: string) => {
    setFiringRuleId(id);
    try {
      const result = await api.fireTriggerRuleNow(id);
      if (!result.success && result.error) {
        setError(`触发失败：${result.error}`);
      }
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setFiringRuleId(null);
    }
  };

  /** Click an execution row → toggle inline display of the AI result.
   *  The content lives in `resultContent` on the record itself — no note
   *  loading needed (trigger results are DB-only, never in the vault). */
  const toggleExecExpand = (exec: TriggerExecution) => {
    if (expandedExecId === exec.id) {
      setExpandedExecId(null);
      setExpandedNoteBody(null);
    } else {
      setExpandedExecId(exec.id);
      setExpandedNoteBody(exec.resultContent || "（无结果内容）");
    }
  };

  /** Delete a single execution record. */
  const handleDeleteExecution = async (id: string) => {
    try {
      await api.deleteTriggerExecution(id);
      setExecutions((prev) => prev.filter((e) => e.id !== id));
      if (expandedExecId === id) {
        setExpandedExecId(null);
        setExpandedNoteBody(null);
      }
    } catch (e) {
      setError(String(e));
    }
  };

  /** Clear ALL execution records. */
  const handleClearExecutions = async () => {
    try {
      await api.clearTriggerExecutions();
      setExecutions([]);
      setExpandedExecId(null);
      setExpandedNoteBody(null);
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
        <div className="flex items-center gap-1.5">
          <Button size="sm" variant="ghost" onClick={() => load()} title="刷新状态与执行记录">
            ⟳
          </Button>
          <Button size="sm" onClick={() => (formOpen ? resetForm() : setAddOpen(true))}>
            {formOpen ? "取消" : "+ 新建"}
          </Button>
        </div>
      </div>

      {error && (
        <div className="mx-4 mt-2 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
          {error}
          <button onClick={() => setError(null)} className="ml-2 underline">关闭</button>
        </div>
      )}

      {/* Create / edit rule form */}
      {formOpen && (
        <div className="border-b border-border bg-card/50 p-4 space-y-4">
          <div className="text-xs font-medium text-muted-foreground">
            {editingRule ? `编辑规则：${editingRule.label}` : "新建规则"}
          </div>
          <input
            value={newLabel}
            onChange={(e) => setNewLabel(e.target.value)}
            placeholder="规则名称（如：早间回顾）"
            className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
          />

          {editingEvent ? (
            /* Event rules have no schedule — keep the original trigger config. */
            <div className="rounded-md bg-muted/50 px-3 py-2 text-xs text-muted-foreground">
              📡 事件触发：{editingRule?.triggerConfig}（事件与筛选保持不变，可修改名称 / 动作 / 提示词）
            </div>
          ) : (
            <>
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
            </>
          )}

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

          {/* Provider selection — each rule can use a different LLM. */}
          <div>
            <label className="text-xs text-muted-foreground mb-1 block">使用供应商</label>
            <select
              value={providerName}
              onChange={(e) => setProviderName(e.target.value)}
              className="w-full rounded-md border border-border bg-background px-3 py-2 text-sm"
            >
              <option value="">默认（当前活跃供应商）</option>
              {providers.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}（{p.model}）
                </option>
              ))}
            </select>
          </div>

          {newAction === "custom" && (
            <div className="space-y-1">
              <Textarea
                value={newPrompt}
                onChange={(e) => setNewPrompt(e.target.value)}
                placeholder="自定义提示词（必填）— 例如：总结我最近一周的笔记并列出关键要点"
                rows={3}
                className={cn("text-sm", !newPrompt.trim() && "border-destructive/50")}
              />
              {!newPrompt.trim() && (
                <p className="text-xs text-destructive">⚠ 选择「自定义提示词」动作时必须填写提示词内容</p>
              )}
            </div>
          )}

          <Button
            size="sm"
            onClick={handleSave}
            disabled={
              creating ||
              !newLabel.trim() ||
              (!editingEvent && selectedDays.length === 0) ||
              (newAction === "custom" && !newPrompt.trim())
            }
          >
            {creating
              ? editingRule
                ? "保存中…"
                : "创建中…"
              : editingRule
                ? "保存修改"
                : "创建规则"}
          </Button>
        </div>
      )}

      {/* Rules list */}
      <div className="flex-1 overflow-auto vp-scroll p-4 space-y-2">
        {rules.length === 0 && (
          <p className="text-center text-sm text-muted-foreground py-8">暂无定时规则</p>
        )}
        {rules.map((rule) => (          <div
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
                {rule.providerName ? ` · 🏷 ${rule.providerName}` : ""}
              </div>
              {rule.triggerType === "cron" && (
                <div
                  className={cn(
                    "text-xs mt-0.5 truncate",
                    rule.lastStatus === "failed" ? "text-destructive" : "text-muted-foreground"
                  )}
                  title={rule.lastStatus === "failed" ? rule.lastError : undefined}
                >
                  {rule.lastStatus === "failed"
                    ? `⚠ 上次触发失败：${rule.lastError || "未知错误"}`
                    : rule.lastFiredAt
                      ? `✓ 上次触发 ${fmtTime(rule.lastFiredAt)} · 已触发 ${rule.runCount ?? 0} 次`
                      : "⏳ 尚未触发过"}
                  {rule.nextFireAt ? ` · 下次 ${fmtTime(rule.nextFireAt)}` : ""}
                </div>
              )}
            </div>
            <button
              onClick={() => handleFireNow(rule.id)}
              disabled={firingRuleId === rule.id}
              className={cn(
                "shrink-0 rounded p-1 transition-colors",
                firingRuleId === rule.id
                  ? "text-muted-foreground animate-pulse"
                  : "text-muted-foreground hover:bg-primary/10 hover:text-primary"
              )}
              title="立即触发"
            >
              {firingRuleId === rule.id ? "…" : "⚡"}
            </button>
            <button
              onClick={() => startEdit(rule)}
              className={cn(
                "shrink-0 rounded p-1 transition-colors",
                editingRule?.id === rule.id
                  ? "bg-accent text-foreground"
                  : "text-muted-foreground hover:bg-accent hover:text-foreground"
              )}
              title="编辑"
            >
              ✎
            </button>
            <button
              onClick={() => handleDelete(rule.id)}
              className="shrink-0 rounded p-1 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
              title="删除"
            >
              ✕
            </button>
          </div>
        ))}

        {/* Recent executions — the "did it actually run?" log */}
        <div className="border-t border-border pt-3 mt-4">
          <div className="text-xs font-semibold text-muted-foreground mb-2">最近执行记录</div>
          {executions.length === 0 ? (
            <p className="text-xs text-muted-foreground py-2">
              暂无执行记录 — 规则到点触发后会显示在这里（失败也会记录原因）
            </p>
          ) : (
            <>
              <div className="mb-1 text-right">
                <button
                  onClick={() => void handleClearExecutions()}
                  className="text-xs text-muted-foreground transition-colors hover:text-destructive"
                  title="清空全部执行记录"
                >
                  清空记录
                </button>
              </div>
              <div className="space-y-1">
                {executions.map((e) => {
                  // detail carries "tokens_in=… tokens_out=…" on success.
                  const tokens = e.detail.match(/tokens_in=(\d+).*tokens_out=(\d+)/);
                  const hasResult = !!e.resultContent;
                  const expanded = expandedExecId === e.id;
                  return (
                    <div key={e.id}>
                      <div
                        onClick={() => hasResult && toggleExecExpand(e)}
                        className={cn(
                          "flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-xs",
                          hasResult && "cursor-pointer hover:bg-accent/50 transition-colors"
                        )}
                        title={hasResult ? "点击查看结果" : undefined}
                      >
                        <span
                          className={cn(
                            "shrink-0 rounded px-1.5 py-0.5",
                            e.status === "success"
                              ? "bg-primary/10 text-primary"
                              : "bg-destructive/10 text-destructive"
                          )}
                        >
                          {e.status === "success" ? "成功" : "失败"}
                        </span>
                        <span className="shrink-0 text-muted-foreground font-mono">{fmtTime(e.firedAt)}</span>
                        <span className="shrink-0 truncate max-w-[25%]" title={ACTION_LABELS[e.action] ?? e.action}>
                          {e.label}
                        </span>
                        {e.status === "success" && tokens && (
                          <span className="shrink-0 text-muted-foreground" title={e.detail}>
                            ⤵ {tokens[1]} / {tokens[2]} tokens
                          </span>
                        )}
                        {e.status === "failed" && e.error && (
                          <span className="min-w-0 flex-1 truncate text-destructive" title={e.error}>
                            {e.error}
                          </span>
                        )}
                        {hasResult && (
                          <span className="ml-auto shrink-0 text-muted-foreground">
                            {expanded ? "▲" : "▼"}
                          </span>
                        )}
                        <button
                          onClick={(ev) => {
                            ev.stopPropagation();
                            void handleDeleteExecution(e.id);
                          }}
                          className="shrink-0 rounded p-0.5 text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
                          title="删除此记录"
                        >
                          ✕
                        </button>
                      </div>
                      {expanded && (
                        <div className="mt-1 rounded-md border border-border bg-muted/30 px-3 py-2 text-xs max-h-48 overflow-auto vp-scroll whitespace-pre-wrap break-words">
                          {expandedNoteBody ?? "（无内容）"}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
