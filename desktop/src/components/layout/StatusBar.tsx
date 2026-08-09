import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import { cn } from "@/lib/utils";

type ConnState = "checking" | "ok" | "fail";

export function StatusBar() {
  const [conn, setConn] = useState<ConnState>("checking");

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
    return () => {
      cancelled = true;
    };
  }, []);

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
      </div>
      <span>VaultPilot Desktop · Tauri v2</span>
    </footer>
  );
}
