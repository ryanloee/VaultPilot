import { useState } from "react";
import { api } from "@/lib/tauri";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type SyncStatus = {
  disk_files: number;
  indexed_notes: number;
  needs_rebuild: boolean;
  latest_disk_mtime: string;
};

export function SyncPanel({ vaultDir }: { vaultDir: string }) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [rebuilding, setRebuilding] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const check = async () => {
    setChecking(true);
    setMsg(null);
    try {
      const s = await api.vaultSyncStatus();
      setStatus(s as SyncStatus);
    } catch (e) {
      setMsg(`检测失败：${e}`);
    } finally {
      setChecking(false);
    }
  };

  const rebuild = async () => {
    setRebuilding(true);
    setMsg(null);
    try {
      await api.rebuildIndex();
      setMsg("索引重建完成");
      await check();
    } catch (e) {
      setMsg(`重建失败：${e}`);
    } finally {
      setRebuilding(false);
    }
  };

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-semibold mb-1">笔记同步</h3>
        <p className="text-xs text-muted-foreground">
          Vault 本身是一个 Markdown 文件夹，天然支持文件夹级同步。
          用 Syncthing / Dropbox / OneDrive 等工具把下面的路径同步到手机即可。
        </p>
      </div>

      <div className="rounded-md border border-border bg-muted/30 px-3 py-2">
        <div className="text-xs text-muted-foreground mb-0.5">Vault 路径</div>
        <div className="flex items-center gap-2">
          <code className="text-xs break-all flex-1">{vaultDir}</code>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => navigator.clipboard?.writeText(vaultDir)}
            title="复制路径"
          >
            ⧉
          </Button>
        </div>
      </div>

      <div className="flex items-center gap-2">
        <Button size="sm" variant="outline" onClick={check} disabled={checking}>
          {checking ? "检测中…" : "检测变更"}
        </Button>
        <Button
          size="sm"
          onClick={rebuild}
          disabled={rebuilding || (status !== null && !status.needs_rebuild)}
        >
          {rebuilding ? "重建中…" : "重建索引"}
        </Button>
      </div>

      {status && (
        <div
          className={cn(
            "rounded-md border px-3 py-2 text-xs space-y-1",
            status.needs_rebuild
              ? "border-destructive/30 bg-destructive/5"
              : "border-border bg-muted/30"
          )}
        >
          <div className="flex justify-between">
            <span>磁盘文件</span>
            <span className="font-mono">{status.disk_files} 篇</span>
          </div>
          <div className="flex justify-between">
            <span>索引笔记</span>
            <span className="font-mono">{status.indexed_notes} 篇</span>
          </div>
          {status.latest_disk_mtime && (
            <div className="flex justify-between">
              <span>最新文件时间</span>
              <span className="font-mono">
                {new Date(status.latest_disk_mtime).toLocaleString()}
              </span>
            </div>
          )}
          {status.needs_rebuild ? (
            <p className="text-destructive pt-1">
              ⚠ 磁盘与索引不一致（可能是同步工具更新了文件），建议重建索引。
            </p>
          ) : (
            <p className="text-muted-foreground pt-1">✓ 磁盘与索引一致，无需重建。</p>
          )}
        </div>
      )}

      {msg && <p className="text-xs text-muted-foreground">{msg}</p>}

      <details className="text-xs">
        <summary className="cursor-pointer text-muted-foreground hover:text-foreground">
          同步设置指引（Syncthing / Dropbox）
        </summary>
        <div className="mt-2 space-y-2 text-muted-foreground pl-3 border-l-2 border-border">
          <p>
            <strong>Syncthing（推荐，免费、P2P、无云端）</strong>
          </p>
          <ol className="list-decimal list-inside space-y-1">
            <li>电脑和手机都安装 Syncthing（Android 版叫 Syncthing-Fork）</li>
            <li>电脑端添加共享文件夹，选择上面的 Vault 路径</li>
            <li>手机端接受共享，选择一个本地目录存放</li>
            <li>在 VaultPilot 手机端的设置里，把 Vault 路径指向同步目录</li>
          </ol>
          <p>
            <strong>Dropbox / OneDrive</strong>
          </p>
          <ol className="list-decimal list-inside space-y-1">
            <li>把 Vault 路径改为 Dropbox/OneDrive 内的子文件夹</li>
            <li>手机端装对应网盘 App，等文件同步完成后同样设置路径</li>
          </ol>
          <p className="pt-1">
            ⚠ 同步完成后，点上面的「检测变更」→「重建索引」让搜索引擎识别新文件。
          </p>
        </div>
      </details>
    </div>
  );
}
