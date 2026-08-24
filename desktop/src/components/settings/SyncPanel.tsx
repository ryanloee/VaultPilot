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

type DiscoveredDevice = {
  hostname: string;
  platform: string;
  vaultPilotVersion: string;
  noteCount: number;
  vaultName: string;
};

type PeerDevice = {
  deviceId: string;
  hostname: string;
  platform: string;
  token: string;
  ip: string | null;
  addedAt: string;
  lastSyncAt: string | null;
};

type SyncResult = {
  pulled: number;
  pushed: number;
  conflicts: number;
  errors: string[];
};

export function SyncPanel({ vaultDir }: { vaultDir: string }) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [rebuilding, setRebuilding] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  // Discovery
  const [ip, setIp] = useState("");
  const [searching, setSearching] = useState(false);
  const [device, setDevice] = useState<DiscoveredDevice | null>(null);
  const [searchMsg, setSearchMsg] = useState<string | null>(null);

  // Pairing & sync
  const [pairCode, setPairCode] = useState<string | null>(null);
  const [remoteIp, setRemoteIp] = useState("");
  const [remoteCode, setRemoteCode] = useState("");
  const [peers, setPeers] = useState<PeerDevice[]>([]);
  const [busy, setBusy] = useState(false);
  const [syncResult, setSyncResult] = useState<Record<string, SyncResult>>({});
  const [syncingId, setSyncingId] = useState<string | null>(null);

  const refreshPeers = async () => {
    try {
      setPeers(await api.listSyncPeers());
    } catch {
      /* ignore */
    }
  };

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

  const searchDevice = async () => {
    if (!ip.trim()) return;
    setSearching(true);
    setDevice(null);
    setSearchMsg(null);
    try {
      const found = await api.discoverDevice(ip.trim());
      if (found) {
        setDevice(found as DiscoveredDevice);
        setSearchMsg(null);
        void refreshPeers();
      } else {
        setSearchMsg(`在 ${ip} 上未找到 VaultPilot 客户端（或对方未开启同步）`);
      }
    } catch (e) {
      setSearchMsg(`搜索失败：${e}`);
    } finally {
      setSearching(false);
    }
  };

  const genPairCode = async () => {
    try {
      setPairCode(await api.generatePairCode());
      await refreshPeers();
    } catch (e) {
      setMsg(`生成配对码失败：${e}`);
    }
  };

  const doPair = async () => {
    if (!remoteIp.trim() || !remoteCode.trim()) return;
    setBusy(true);
    setMsg(null);
    try {
      await api.completePairing(remoteIp.trim(), remoteCode.trim());
      setRemoteIp("");
      setRemoteCode("");
      await refreshPeers();
      setMsg("配对成功");
    } catch (e) {
      setMsg(`配对失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const doSync = async (peer: PeerDevice) => {
    setSyncingId(peer.deviceId);
    setMsg(null);
    try {
      const r = await api.syncWithPeer(peer.ip ?? "", peer.deviceId);
      setSyncResult((prev) => ({ ...prev, [peer.deviceId]: r as SyncResult }));
      await refreshPeers();
    } catch (e) {
      setMsg(`同步失败：${e}`);
    } finally {
      setSyncingId(null);
    }
  };

  const removePeer = async (id: string) => {
    try {
      await api.removeSyncPeer(id);
      await refreshPeers();
    } catch (e) {
      setMsg(`移除失败：${e}`);
    }
  };

  const platformIcon = (p: string) =>
    p === "windows" ? "💻" : p === "linux" ? "🐧" : "📱";

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-semibold mb-1">笔记同步</h3>
        <p className="text-xs text-muted-foreground">
          在同一局域网里，把当前设备与另一台 VaultPilot 配对后，即可双向同步整个
          Vault（Markdown 文件夹）。也支持用 Syncthing / Dropbox 等工具同步下方路径。
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

      {/* Pairing code (acceptor side) */}
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2 space-y-2">
        <div className="text-xs text-muted-foreground">本机配对码</div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={genPairCode}>
            生成配对码
          </Button>
          {pairCode && (
            <code className="text-sm font-mono font-semibold px-2 py-1 rounded bg-background border border-border select-all">
              {pairCode}
            </code>
          )}
          {pairCode && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => navigator.clipboard?.writeText(pairCode)}
              title="复制配对码"
            >
              ⧉
            </Button>
          )}
        </div>
        {pairCode && (
          <p className="text-xs text-muted-foreground">
            在另一台设备的「配对其他设备」里输入<strong>本机 IP</strong>与这个配对码即可配对。
          </p>
        )}
      </div>

      {/* Initiate pairing (initiator side) */}
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2 space-y-2">
        <div className="text-xs text-muted-foreground">配对其他设备</div>
        <div className="flex items-center gap-2">
          <input
            value={remoteIp}
            onChange={(e) => setRemoteIp(e.target.value)}
            placeholder="对方 IP（如 192.168.1.100）"
            className="flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
          />
          <input
            value={remoteCode}
            onChange={(e) => setRemoteCode(e.target.value)}
            placeholder="对方配对码"
            className="w-28 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
          />
          <Button size="sm" variant="outline" onClick={doPair} disabled={busy || !remoteIp.trim() || !remoteCode.trim()}>
            {busy ? "配对中…" : "配对"}
          </Button>
        </div>
      </div>

      {/* Paired devices */}
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2 space-y-2">
        <div className="text-xs text-muted-foreground">已配对设备</div>
        {peers.length === 0 && (
          <p className="text-xs text-muted-foreground">还没有配对设备。</p>
        )}
        {peers.map((p) => (
          <div key={p.deviceId} className="rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs space-y-1">
            <div className="flex items-center gap-2">
              <span className="text-primary font-semibold">
                {platformIcon(p.platform)} {p.hostname}
              </span>
              <span className="text-muted-foreground">{p.ip ?? "（无 IP）"}</span>
            </div>
            {p.lastSyncAt && (
              <div className="text-muted-foreground">
                上次同步：{new Date(p.lastSyncAt).toLocaleString()}
              </div>
            )}
            {syncResult[p.deviceId] && (
              <div className="text-muted-foreground">
                上次结果：拉取 {syncResult[p.deviceId].pulled} · 推送{" "}
                {syncResult[p.deviceId].pushed} · 冲突{" "}
                {syncResult[p.deviceId].conflicts}
                {syncResult[p.deviceId].errors.length > 0 &&
                  ` · 失败 ${syncResult[p.deviceId].errors.length}`}
              </div>
            )}
            <div className="flex items-center gap-2 pt-1">
              <Button
                size="sm"
                variant="outline"
                onClick={() => void doSync(p)}
                disabled={syncingId === p.deviceId}
              >
                {syncingId === p.deviceId ? "同步中…" : "同步"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void removePeer(p.deviceId)}
              >
                移除
              </Button>
            </div>
          </div>
        ))}
      </div>

      {/* LAN device discovery — direct IP probe */}
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2 space-y-2">
        <div className="text-xs text-muted-foreground">搜索局域网设备</div>
        <div className="flex items-center gap-2">
          <input
            value={ip}
            onChange={(e) => setIp(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void searchDevice()}
            placeholder="输入对方 IP（如 192.168.1.100）"
            className="flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
          />
          <Button size="sm" variant="outline" onClick={() => void searchDevice()} disabled={searching || !ip.trim()}>
            {searching ? "搜索中…" : "搜索"}
          </Button>
        </div>
        {searchMsg && <p className="text-xs text-muted-foreground">{searchMsg}</p>}
        {device && (
          <div className="rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs space-y-1">
            <div className="flex items-center gap-2">
              <span className="text-primary font-semibold">
                {device.platform === "windows" ? "💻" : device.platform === "linux" ? "🐧" : "📱"}{" "}
                {device.hostname}
              </span>
              <span className="text-muted-foreground">({ip})</span>
            </div>
            <div className="text-muted-foreground">
              VaultPilot v{device.vaultPilotVersion} · {device.noteCount} 篇笔记 · {device.vaultName}
            </div>
            <Button
              size="sm"
              variant="outline"
              className="mt-1"
              onClick={() => {
                setRemoteIp(ip.trim());
                void searchDevice();
              }}
            >
              用此 IP 配对
            </Button>
          </div>
        )}
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
