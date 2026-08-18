import { useEffect, useState } from "react";
import { useSettingsStore } from "@/lib/store";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ScrollArea } from "@/components/ui/scroll-area";
import { cn } from "@/lib/utils";
import { api } from "@/lib/tauri";
import { applyAndPersistTheme, savedTheme, type Theme } from "@/lib/theme";
import type { AppSettings, ProviderConfig, ProviderConnectionResult } from "@/types";

export function SettingsView() {
  const { settings, loading, error, load, save } = useSettingsStore();
  const [draft, setDraft] = useState<AppSettings | null>(null);
  const [theme, setTheme] = useState<Theme>("system");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<ProviderConnectionResult | null>(null);

  // Apply + persist the theme when the user clicks a theme button.
  const selectTheme = (mode: Theme) => {
    applyAndPersistTheme(mode);
    setTheme(mode);
  };

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (settings) {
      setDraft(structuredClone(settings));
    }
  }, [settings]);

  // Read persisted theme once on mount for the button highlight state.
  useEffect(() => {
    setTheme(savedTheme() ?? "system");
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

  const handleTestConnection = async () => {
    if (testing) return;
    setTesting(true);
    setTestResult(null);
    try {
      const result = await api.testProviderConnection(
        active.baseUrl,
        active.apiKey,
        active.providerType ?? "openai",
        active.model || undefined,
        active.requestTimeoutMs
      );
      setTestResult(result);
    } catch (e) {
      setTestResult({ ok: false, error: String(e) });
    } finally {
      setTesting(false);
    }
  };

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
                onClick={() => selectTheme(t)}
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
            <div className="flex gap-2">
              <Input
                type="password"
                value={active.apiKey}
                onChange={(e) => updateProvider({ apiKey: e.target.value })}
                placeholder="sk-..."
              />
              <Button
                variant="outline"
                size="sm"
                onClick={handleTestConnection}
                disabled={
                  testing || !active.apiKey || active.apiKey.startsWith("ENC:v1:")
                }
                title={
                  !active.apiKey
                    ? "请先填写 API Key"
                    : active.apiKey.startsWith("ENC:v1:")
                      ? "存储的 Key 无法解密，请重新输入完整 Key"
                      : "测试能否连接到该提供商"
                }
              >
                {testing ? "测试中…" : "测试连接"}
              </Button>
            </div>
          </Field>
          {testResult && (
            <p
              className={cn(
                "flex items-center gap-2 text-xs",
                testResult.ok ? "text-green-600" : "text-destructive"
              )}
            >
              {testResult.ok ? (
                <>
                  <span>✓ 连接成功</span>
                  {testResult.pingOk === true ? (
                    <span>· 消息发送测试通过</span>
                  ) : (
                    testResult.pingOk === false && (
                      <span>· ⚠ 消息发送失败（模型未配置时跳过）</span>
                    )
                  )}
                  {testResult.status && <span>(HTTP {testResult.status})</span>}
                  {testResult.models && testResult.models.length > 0 && (
                    <span>
                      · 检测到模型: {testResult.models.slice(0, 5).join(", ")}
                      {testResult.models.length > 5 ? ` 等 ${testResult.models.length} 个` : ""}
                    </span>
                  )}
                </>
              ) : (
                <span>
                  ✗ 连接失败: {testResult.error ?? "未知错误"}
                  {testResult.pingStatus ? ` (ping HTTP ${testResult.pingStatus})` : ""}
                </span>
              )}
            </p>
          )}
          <Field label="模型">
            <Input
              value={active.model}
              onChange={(e) => updateProvider({ model: e.target.value })}
              placeholder="gpt-4o"
            />
          </Field>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
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
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={!!draft.headingNumbering}
              onChange={(e) => setDraft({ ...draft, headingNumbering: e.target.checked })}
            />
            笔记标题自动编号（1 / 1.1 / 1.1.2…，仅渲染层，不修改源文件）
          </label>
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
