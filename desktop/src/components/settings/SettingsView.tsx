import { useEffect, useState } from "react";
import { useSettingsStore } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import type { AppSettings, ProviderConfig } from "@/types";

type Theme = "system" | "light" | "dark";

export function SettingsView() {
  const { settings, loading, error, load, save } = useSettingsStore();
  const [draft, setDraft] = useState<AppSettings | null>(null);
  const [theme, setTheme] = useState<Theme>("system");

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (settings) {
      setDraft(structuredClone(settings));
    }
  }, [settings]);

  // Apply theme: add/remove .dark on <html>. Reads the persisted pref from
  // the same theme.json the WinUI client uses (best-effort, localStorage fallback).
  useEffect(() => {
    const apply = (mode: Theme) => {
      const root = document.documentElement;
      const isDark =
        mode === "dark" ||
        (mode === "system" && window.matchMedia("(prefers-color-scheme: dark)").matches);
      root.classList.toggle("dark", isDark);
    };
    apply(theme);
    try {
      localStorage.setItem("vaultpilot.theme", theme);
    } catch {
      /* ignore */
    }
  }, [theme]);

  useEffect(() => {
    try {
      const saved = localStorage.getItem("vaultpilot.theme") as Theme | null;
      if (saved) setTheme(saved);
    } catch {
      /* ignore */
    }
  }, []);

  if (!draft) {
    return (
      <div className="flex h-full items-center justify-center text-sm text-muted-foreground">
        {loading ? "加载设置中…" : error ?? "无法加载设置"}
      </div>
    );
  }

  const updateProvider = (patch: Partial<ProviderConfig>) => {
    setDraft((d) => {
      if (!d) return d;
      const idx = d.activeProviderIndex;
      const providers = [...d.providers];
      if (providers[idx]) {
        providers[idx] = { ...providers[idx], ...patch };
      }
      return { ...d, providers, provider: providers[idx] ?? d.provider };
    });
  };

  const handleSave = () => {
    if (draft) save(draft);
  };

  const active = draft.providers[draft.activeProviderIndex] ?? draft.provider;

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto max-w-2xl space-y-6 p-6">
        <section>
          <h2 className="mb-3 text-lg font-semibold">主题</h2>
          <div className="flex gap-2">
            {(["system", "light", "dark"] as Theme[]).map((t) => (
              <Button
                key={t}
                variant={theme === t ? "default" : "outline"}
                size="sm"
                onClick={() => setTheme(t)}
              >
                {t === "system" ? "跟随系统" : t === "light" ? "亮色" : "暗色"}
              </Button>
            ))}
          </div>
        </section>

        <Separator />

        <section className="space-y-3">
          <h2 className="text-lg font-semibold">AI 提供商</h2>
          <Field label="名称">
            <Input
              value={active.name}
              onChange={(e) => updateProvider({ name: e.target.value })}
            />
          </Field>
          <Field label="Base URL">
            <Input
              value={active.baseUrl}
              onChange={(e) => updateProvider({ baseUrl: e.target.value })}
              placeholder="https://api.openai.com/v1"
            />
          </Field>
          <Field label="API Key">
            <Input
              type="password"
              value={active.apiKey}
              onChange={(e) => updateProvider({ apiKey: e.target.value })}
              placeholder="sk-..."
            />
          </Field>
          <Field label="模型">
            <Input
              value={active.model}
              onChange={(e) => updateProvider({ model: e.target.value })}
              placeholder="gpt-4o"
            />
          </Field>
          <div className="grid grid-cols-2 gap-3">
            <Field label="超时 (ms)">
              <Input
                type="number"
                value={active.requestTimeoutMs}
                onChange={(e) =>
                  updateProvider({ requestTimeoutMs: Number(e.target.value) || 60000 })
                }
              />
            </Field>
            <Field label="上下文窗口 (tokens)">
              <Input
                type="number"
                value={active.contextWindowTokens ?? 0}
                onChange={(e) =>
                  updateProvider({ contextWindowTokens: Number(e.target.value) || undefined })
                }
              />
            </Field>
          </div>
        </section>

        <Separator />

        <section className="space-y-3">
          <h2 className="text-lg font-semibold">常规</h2>
          <Field label="Vault 目录">
            <Input
              value={draft.vaultDir}
              onChange={(e) => setDraft({ ...draft, vaultDir: e.target.value })}
            />
          </Field>
          <Field label="代理 URL（可选）">
            <Input
              value={draft.proxyUrl ?? ""}
              onChange={(e) => setDraft({ ...draft, proxyUrl: e.target.value })}
              placeholder="http://127.0.0.1:7890"
            />
          </Field>
          <Field label="系统指令（可选）">
            <Textarea
              value={draft.systemDirective ?? ""}
              onChange={(e) => setDraft({ ...draft, systemDirective: e.target.value })}
              rows={3}
              placeholder="附加到每次对话的系统提示词"
            />
          </Field>
        </section>

        {error && (
          <p className="rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {error}
          </p>
        )}

        <div className="sticky bottom-0 flex justify-end gap-2 border-t border-border bg-background pt-3">
          <Button variant="ghost" onClick={() => settings && setDraft(structuredClone(settings))}>
            重置
          </Button>
          <Button onClick={handleSave} disabled={loading}>
            {loading ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>
    </ScrollArea>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block space-y-1">
      <span className="text-xs font-medium text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

function Separator() {
  return <div className={cn("h-px w-full bg-border")} />;
}
