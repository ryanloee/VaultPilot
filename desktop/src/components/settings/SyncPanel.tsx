import { useEffect, useState } from "react";
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
  ip: string;
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

type SyncMode = "full" | "selected";

const platformIcon = (p: string) =>
  p === "windows" ? "💻" : p === "linux" ? "🐧" : "📱";

function DiscoveredCard({
  device,
  busy,
  onPair,
}: {
  device: DiscoveredDevice;
  busy: boolean;
  onPair: (code: string) => void;
}) {
  const [code, setCode] = useState("");
  return (
    <div className="rounded-md border border-primary/30 bg-primary/5 px-3 py-2 text-xs space-y-1">
      <div className="flex items-center gap-2">
        <span className="text-primary font-semibold">
          {platformIcon(device.platform)} {device.hostname}
        </span>
        <span className="text-muted-foreground">{device.ip}</span>
      </div>
      <div className="text-muted-foreground">
        VaultPilot v{device.vaultPilotVersion} · {device.noteCount} 篇笔记 ·{" "}
        {device.vaultName}
      </div>
      <div className="flex items-center gap-2 pt-1">
        <input
          value={code}
          onChange={(e) => setCode(e.target.value)}
          placeholder="输入对方配对码"
          className="w-36 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
        />
        <Button
          size="sm"
          variant="outline"
          disabled={busy || !code.trim()}
          onClick={() => onPair(code)}
        >
          {busy ? "配对中…" : "配对"}
        </Button>
      </div>
    </div>
  );
}

export function SyncPanel({ vaultDir }: { vaultDir: string }) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [rebuilding, setRebuilding] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  // Own pairing code — auto-generated on mount so the other side can type it.
  const [myCode, setMyCode] = useState<string | null>(null);
  const genCode = async () => {
    try {
      setMyCode(await api.generatePairCode());
    } catch (e) {
      setMsg(`生成配对码失败：${e}`);
    }
  };
  useEffect(() => {
    void genCode();
    void refreshPeers();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Single search bar: empty → scan the LAN, IP → targeted probe.
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [devices, setDevices] = useState<DiscoveredDevice[]>([]);
  const [searchMsg, setSearchMsg] = useState<string | null>(null);

  const doSearch = async () => {
    setSearching(true);
    setDevices([]);
    setSearchMsg(null);
    try {
      const q = query.trim();
      if (!q) {
        const found = await api.scanLanDevices();
        if (found.length === 0) {
          setSearchMsg(
            "局域网内未找到其他 VaultPilot 设备（确认对方已开启同步且与你在同一网段）"
          );
        } else {
          setDevices(found);
        }
      } else {
        const found = await api.discoverDevice(q);
        if (found) {
          setDevices([{ ip: q, ...found }]);
        } else {
          setSearchMsg(`在 ${q} 上未找到 VaultPilot 客户端（或对方未开启同步）`);
        }
      }
    } catch (e) {
      setSearchMsg(`搜索失败：${e}`);
    } finally {
      setSearching(false);
    }
  };

  // Pairing & sync
  const [peers, setPeers] = useState<PeerDevice[]>([]);
  const [busy, setBusy] = useState(false);
  const [syncResult, setSyncResult] = useState<Record<string, SyncResult>>({});
  const [syncingId, setSyncingId] = useState<string | null>(null);
  const [mode, setMode] = useState<Record<string, SyncMode>>({});
  const [includes, setIncludes] = useState<Record<string, string>>({});

  const refreshPeers = async () => {
    try {
      setPeers(await api.listSyncPeers());
    } catch {
      /* ignore */
    }
  };

  const doPair = async (ip: string, rawCode: string) => {
    const code = rawCode.trim();
    if (!ip || !code) return;
    setBusy(true);
    setMsg(null);
    try {
      await api.completePairing(ip, code);
      await refreshPeers();
      setMsg(`已与 ${ip} 配对成功`);
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
      const m = mode[peer.deviceId] ?? "full";
      const inc =
        m === "selected"
          ? (includes[peer.deviceId] ?? "")
              .split(",")
              .map((s) => s.trim())
              .filter(Boolean)
          : [];
      const r = await api.syncWithPeer(peer.ip ?? "", peer.deviceId, m, inc);
      setSyncResult((prev) => ({ ...prev, [peer.deviceId]: r }));
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
          在同一局域网里把当前设备与另一台 VaultPilot 配对，即可双向同步整个 Vault。
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

      {/* Own pairing code (auto-generated) */}
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2 space-y-2">
        <div className="text-xs text-muted-foreground">
          本机配对码（把此码告诉对方，对方输入它来与你配对）
        </div>
        <div className="flex items-center gap-2">
          <code className="text-sm font-mono font-semibold px-2 py-1 rounded bg-background border border-border select-all">
            {myCode ?? "生成中…"}
          </code>
          <Button size="sm" variant="ghost" onClick={() => navigator.clipboard?.writeText(myCode ?? "")} title="复制">
            ⧉
          </Button>
          <Button size="sm" variant="outline" onClick={() => void genCode()}>
            重新生成
          </Button>
        </div>
      </div>

      {/* Single search bar */}
      <div className="rounded-md border border-border bg-muted/30 px-3 py-2 space-y-2">
        <div className="text-xs text-muted-foreground">
          搜索设备（留空 = 扫描整个局域网；填 IP = 定向搜索）
        </div>
        <div className="flex items-center gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && void doSearch()}
            placeholder="输入对方 IP（如 192.168.1.100），留空则扫描局域网"
            className="flex-1 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
          />
          <Button size="sm" variant="outline" onClick={() => void doSearch()} disabled={searching}>
            {searching ? "搜索中…" : "搜索"}
          </Button>
        </div>
        {searchMsg && <p className="text-xs text-muted-foreground">{searchMsg}</p>}
        {devices.map((d) => (
          <DiscoveredCard key={d.ip} device={d} busy={busy} onPair={(code) => void doPair(d.ip, code)} />
        ))}
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
            <div className="flex items-center gap-2 pt-1 flex-wrap">
              <select
                value={mode[p.deviceId] ?? "full"}
                onChange={(e) =>
                  setMode((prev) => ({ ...prev, [p.deviceId]: e.target.value as SyncMode }))
                }
                className="rounded-md border border-border bg-background px-2 py-1.5 text-sm"
              >
                <option value="full">全部同步</option>
                <option value="selected">选择性同步</option>
              </select>
              <Button
                size="sm"
                variant="outline"
                onClick={() => void doSync(p)}
                disabled={syncingId === p.deviceId}
              >
                {syncingId === p.deviceId ? "同步中…" : "同步"}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => void removePeer(p.deviceId)}>
                移除
              </Button>
            </div>
            {mode[p.deviceId] === "selected" && (
              <input
                value={includes[p.deviceId] ?? ""}
                onChange={(e) =>
                  setIncludes((prev) => ({ ...prev, [p.deviceId]: e.target.value }))
                }
                placeholder="只同步这些目录（逗号分隔，如 notes,journal），留空=全部"
                className="w-full rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
              />
            )}
          </div>
        ))}
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
    </div>
  );
}
