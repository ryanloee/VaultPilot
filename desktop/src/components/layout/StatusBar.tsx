import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { api } from "@/lib/tauri";
import { isTauri } from "@/lib/mock";
import { cn } from "@/lib/utils";
import { useUpdaterStore } from "@/lib/store";

type ConnState = "checking" | "ok" | "fail";

export function StatusBar() {
  const [conn, setConn] = useState<ConnState>("checking");
  const [appVersion, setAppVersion] = useState<string>("");
  const { phase, downloaded, total, version, error } = useUpdaterStore();

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const ok = await api.ping();
        if (!cancelled) setConn(ok ? "ok" : "fail");
      } catch {
        if (!cancelled) setConn("fail");
      }
    })();
    if (isTauri()) {
      getVersion()
        .then((v) => {
          if (!cancelled) setAppVersion(v);
        })
        .catch(() => {});
    }
    return () => {
      cancelled = true;
    };
  }, []);

  const progressPct =
    phase === "downloading" && total > 0 ? Math.round((downloaded / total) * 100) : 0;

  return (
    <footer className="flex h-6 items-center justify-between border-t border-border bg-secondary px-3 text-[11px] text-muted-foreground">
      <div className="flex items-center gap-3">
        <span className="flex items-center gap-1.5">
          <span
            className={cn(
              "h-1.5 w-1.5 rounded-full",
              conn === "ok" && "bg-emerald-500",
              conn === "fail" && "bg-destructive",
              conn === "checking" && "bg-muted-foreground animate-pulse"
            )}
          />
          {conn === "ok" && "后端已连接"}
          {conn === "fail" && "后端未连接"}
          {conn === "checking" && "正在检测…"}
        </span>

        {/* Update download progress — visible on every page */}
        {phase === "downloading" && (
          <span className="flex items-center gap-1.5 text-primary">
            <span className="h-1.5 w-1.5 animate-pulse rounded-full bg-primary" />
            下载更新 v{version}…
            {total > 0 && (
              <span className="ml-1 inline-block h-1 w-16 overflow-hidden rounded-full bg-muted">
                <span
                  className="block h-full bg-primary transition-all"
                  style={{ width: `${progressPct}%` }}
                />
              </span>
            )}
            {total > 0 ? ` ${progressPct}%` : ` ${(downloaded / 1048576).toFixed(1)}MB`}
          </span>
        )}
        {phase === "ready" && (
          <span className="text-emerald-600">✓ 更新 v{version} 已就绪 — 重启后生效</span>
        )}
        {phase === "error" && (
          <span className="text-destructive" title={error ?? undefined}>
            ⚠ 更新失败
          </span>
        )}
      </div>
      <span>VaultPilot{appVersion ? ` v${appVersion}` : ""} · Tauri v2</span>
    </footer>
  );
}
