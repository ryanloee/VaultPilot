import { useEffect, useState } from "react";
import { api } from "./lib/tauri";

/**
 * Stage-0 smoke-test page: verifies the full toolchain works end-to-end.
 *  - "ping" proves the Tauri command dispatcher + vaultpilot_lib link.
 *  - "getSettings" proves we can reach the existing StorageContext
 *    (reads the same settings.json the WinUI client uses).
 *
 * Real layout / chat / notes land in later stages.
 */
export default function App() {
  const [pingOk, setPingOk] = useState<boolean | null>(null);
  const [settings, setSettings] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const ok = await api.ping();
        setPingOk(ok);
        const s = await api.getSettings();
        setSettings(s);
      } catch (e) {
        setError(String(e));
      }
    })();
  }, []);

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center gap-6 bg-background text-foreground">
      <h1 className="text-4xl font-semibold tracking-tight">VaultPilot</h1>
      <p className="text-muted-foreground">Tauri v2 + React + shadcn/ui 重构进行中</p>

      <div className="rounded-lg border border-border bg-card p-6 shadow-sm w-[480px] space-y-3">
        <div className="flex items-center justify-between">
          <span className="text-sm text-muted-foreground">后端连接</span>
          <StatusBadge
            ok={pingOk}
            label={pingOk === null ? "检测中…" : pingOk ? "已连接" : "失败"}
          />
        </div>

        {error && (
          <pre className="rounded bg-destructive/10 p-3 text-xs text-destructive whitespace-pre-wrap">
            {error}
          </pre>
        )}

        {settings !== null && (
          <details className="text-xs">
            <summary className="cursor-pointer text-muted-foreground">
              已读取 settings.json
            </summary>
            <pre className="mt-2 max-h-64 overflow-auto rounded bg-muted p-3">
              {JSON.stringify(settings, null, 2)}
            </pre>
          </details>
        )}
      </div>

      <p className="text-xs text-muted-foreground">阶段 0 · 脚手架验证</p>
    </div>
  );
}

function StatusBadge({ ok, label }: { ok: boolean | null; label: string }) {
  const color =
    ok === null
      ? "bg-muted text-muted-foreground"
      : ok
        ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-400"
        : "bg-destructive/15 text-destructive";
  return (
    <span className={`rounded-full px-2.5 py-0.5 text-xs font-medium ${color}`}>
      {label}
    </span>
  );
}
