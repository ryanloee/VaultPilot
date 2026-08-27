import { useEffect, useState } from "react";
import { api, onSyncPairing, type SyncPairingEventPayload } from "@/lib/tauri";
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

type ManifestEntryDto = { path: string; sha256: string; mtimeMs: number };

const platformIcon = (p: string) =>
  p === "windows" ? "💻" : p === "linux" ? "🐧" : "📱";

/** One checkbox column of the selective-sync dialog (top-level so its
 * filter input keeps focus across re-renders). */
function ManifestColumn({
  title,
  hint,
  entries,
  filter,
  setFilter,
  sel,
  setSel,
}: {
  title: string;
  hint: string;
  entries: ManifestEntryDto[] | null;
  filter: string;
  setFilter: (v: string) => void;
  sel: Set<string>;
  setSel: React.Dispatch<React.SetStateAction<Set<string>>>;
}) {
  const shown = (entries ?? []).filter((e) =>
    e.path.toLowerCase().includes(filter.toLowerCase())
  );
  const toggle = (path: string) =>
    setSel((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  return (
    <div className="flex flex-col min-w-0">
      <div className="flex items-center justify-between gap-2 mb-1">
        <span className="text-xs font-semibold">{title}</span>
        {entries && (
          <button
            className="text-[11px] text-muted-foreground hover:text-foreground"
            onClick={() =>
              setSel(
                sel.size >= shown.length && shown.length > 0
                  ? new Set()
                  : new Set(shown.map((e) => e.path))
              )
            }
          >
            {sel.size >= shown.length && shown.length > 0 ? "清空" : "全选"}
          </button>
        )}
      </div>
      <div className="text-[11px] text-muted-foreground mb-1">{hint}</div>
      <input
        value={filter}
        onChange={(e) => setFilter(e.target.value)}
        placeholder="过滤路径…"
        className="mb-1 rounded-md border border-border bg-background px-2 py-1 text-xs font-mono"
      />
      {!entries ? (
        <div className="text-xs text-muted-foreground py-4 text-center">
          加载中…
        </div>
      ) : shown.length === 0 ? (
        <div className="text-xs text-muted-foreground py-4 text-center">
          没有文件
        </div>
      ) : (
        <ul className="max-h-72 overflow-auto rounded-md border border-border divide-y divide-border/60">
          {shown.map((e) => (
            <li key={e.path}>
              <label className="flex items-center gap-2 px-2 py-1.5 cursor-pointer hover:bg-muted/50">
                <input
                  type="checkbox"
                  checked={sel.has(e.path)}
                  onChange={() => toggle(e.path)}
                />
                <span
                  className="text-xs truncate"
                  title={`${e.path}\n${new Date(e.mtimeMs).toLocaleString()}`}
                >
                  {e.path}
                </span>
                <span className="ml-auto text-[10px] text-muted-foreground shrink-0 pl-2">
                  {new Date(e.mtimeMs).toLocaleDateString()}
                </span>
              </label>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** Two-column picker: which of the peer's files to download, which local
 * files to send. Opened from「同步」when the mode is 选择性同步. */
function SelectiveSyncDialog({
  peer,
  onClose,
  onDone,
}: {
  peer: PeerDevice;
  onClose: () => void;
  onDone: (r: SyncResult) => void;
}) {
  const [remote, setRemote] = useState<ManifestEntryDto[] | null>(null);
  const [local, setLocal] = useState<ManifestEntryDto[] | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [pullSel, setPullSel] = useState<Set<string>>(new Set());
  const [pushSel, setPushSel] = useState<Set<string>>(new Set());
  const [filterRemote, setFilterRemote] = useState("");
  const [filterLocal, setFilterLocal] = useState("");
  const [running, setRunning] = useState(false);

  useEffect(() => {
    if (!peer.ip) {
      setErr("该设备无 IP 记录，请重新配对或手动指定 IP");
      return;
    }
    void api
      .getPeerManifest(peer.ip)
      .then(setRemote)
      .catch((e) => setErr(`获取对方清单失败：${e}`));
    void api
      .listLocalManifest()
      .then(setLocal)
      .catch((e) => setErr(`获取本机清单失败：${e}`));
  }, [peer]);

  const run = async () => {
    setRunning(true);
    setErr(null);
    try {
      const r = await api.syncSelected(
        peer.ip ?? "",
        peer.deviceId,
        [...pullSel],
        [...pushSel]
      );
      onDone(r);
      onClose();
    } catch (e) {
      setErr(`同步失败：${e}`);
    } finally {
      setRunning(false);
    }
  };

  return (
    <div
      className="fixed inset-0 z-50 bg-black/50 flex items-center justify-center p-4"
      onClick={onClose}
    >
      <div
        className="bg-background rounded-lg border border-border shadow-lg w-full max-w-3xl max-h-[85vh] overflow-auto p-4 space-y-3"
        onClick={(e) => e.stopPropagation()}
      >
        <div>
          <h3 className="text-sm font-semibold">选择性同步 · {peer.hostname}</h3>
          <p className="text-xs text-muted-foreground mt-0.5">
            勾选要从对方下载的笔记和要发送给对方的笔记，两边互不影响；内容冲突的下载会保留本机并把对方版本存为 conflict 副本。
          </p>
        </div>

        {err && (
          <div className="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
            {err}
          </div>
        )}

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <ManifestColumn
            title={`对方的笔记（下载 ${pullSel.size}）`}
            hint={`来自 ${peer.hostname} 的 Vault`}
            entries={remote}
            filter={filterRemote}
            setFilter={setFilterRemote}
            sel={pullSel}
            setSel={setPullSel}
          />
          <ManifestColumn
            title={`本机的笔记（发送 ${pushSel.size}）`}
            hint="当前设备的 Vault"
            entries={local}
            filter={filterLocal}
            setFilter={setFilterLocal}
            sel={pushSel}
            setSel={setPushSel}
          />
        </div>

        <div className="flex items-center justify-end gap-2 pt-1">
          <Button size="sm" variant="ghost" onClick={onClose} disabled={running}>
            取消
          </Button>
          <Button
            size="sm"
            onClick={() => void run()}
            disabled={running || (pullSel.size === 0 && pushSel.size === 0)}
          >
            {running ? "同步中…" : `开始同步（↓${pullSel.size} ↑${pushSel.size}）`}
          </Button>
        </div>
      </div>
    </div>
  );
}

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
          placeholder={`输入 ${device.hostname} 的配对码`}
          className="w-44 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
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
      <div className="text-[11px] text-muted-foreground">
        填 <span className="font-semibold text-foreground">{device.hostname}</span> 界面上显示的配对码（不是你自己的）
      </div>
    </div>
  );
}

function StatusBanner({ msg }: { msg: string | null }) {
  if (!msg) return null;
  const ok = msg.includes("成功");
  return (
    <div
      className={cn(
        "rounded-md border px-3 py-2 text-xs font-medium",
        ok
          ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-600"
          : "border-destructive/40 bg-destructive/10 text-destructive"
      )}
    >
      {msg}
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

  // Desktop-side pairing prompt: the sync server emits `sync-pairing` when another
  // device tries to pair with *us* (the acceptor). Surface it so the user sees a
  // result instead of silence.
  const [pairEvent, setPairEvent] = useState<
    { kind: "accepted" | "rejected"; text: string } | null
  >(null);
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void onSyncPairing((p: SyncPairingEventPayload) => {
      if (p.accepted) {
        setPairEvent({
          kind: "accepted",
          text: `「${p.accepted.hostname}」已与你配对成功`,
        });
        void refreshPeers();
      } else if (p.rejected) {
        setPairEvent({
          kind: "rejected",
          text: `收到配对请求但被拒绝：${p.rejected.reason}`,
        });
      }
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, []);

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

  // Direct pair-by-IP fallback (when LAN scan can't find the other device, e.g.
  // AP isolation / different subnet on mobile).
  const [directIp, setDirectIp] = useState("");
  const [directCode, setDirectCode] = useState("");

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
  const [selectingPeer, setSelectingPeer] = useState<PeerDevice | null>(null);

  const refreshPeers = async () => {
    try {
      setPeers(await api.listSyncPeers());
    } catch {
      /* ignore */
    }
  };

  const doPair = async (ip: string, rawCode: string) => {
    const targetIp = ip.trim();
    const code = rawCode.trim();
    if (!targetIp || !code) {
      setMsg("请填写对方 IP 与配对码");
      return;
    }
    setBusy(true);
    setMsg(null);
    try {
      await api.completePairing(targetIp, code);
      setDevices((prev) => prev.filter((d) => d.ip !== targetIp));
      setDirectIp("");
      setDirectCode("");
      await refreshPeers();
      setMsg(`已与 ${targetIp} 配对成功`);
    } catch (e) {
      setMsg(`配对失败：${e}`);
    } finally {
      setBusy(false);
    }
  };

  const doSync = async (peer: PeerDevice) => {
    const m = mode[peer.deviceId] ?? "full";
    if (m === "selected") {
      setSelectingPeer(peer);
      return;
    }
    setSyncingId(peer.deviceId);
    setMsg(null);
    try {
      const r = await api.syncWithPeer(peer.ip ?? "", peer.deviceId, "full", []);
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

      <StatusBanner msg={msg} />
      {pairEvent && (
        <div
          className={cn(
            "rounded-md border px-3 py-2 text-xs font-medium",
            pairEvent.kind === "accepted"
              ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-600"
              : "border-destructive/40 bg-destructive/10 text-destructive"
          )}
        >
          {pairEvent.text}
        </div>
      )}

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
          本机配对码（这是<strong className="text-foreground">你</strong>的码，发给对方；对方要在它自己的设备里输入<strong className="text-foreground">它的</strong>码才能与你配对）
        </div>
        <div className="flex items-center gap-2">
          <code className="text-sm font-mono font-semibold px-2 py-1 rounded bg-background border border-border select-all">
            {myCode ?? "生成中…"}
          </code>
          <Button size="sm" variant="ghost" onClick={() => navigator.clipboard?.writeText(myCode ?? "")} title="复制">
            ⧉
          </Button>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              void api
                .regeneratePairCode()
                .then(setMyCode)
                .catch((e) => setMsg(`生成配对码失败：${e}`))
            }
          >
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

        {/* Direct pair-by-IP fallback (mobile / AP isolation) */}
        <div className="pt-2 border-t border-border/60 space-y-2">
          <div className="text-xs text-muted-foreground">
            扫描不到对方？直接填对方 IP + 对方显示的配对码：
          </div>
          <div className="flex items-center gap-2 flex-wrap">
            <input
              value={directIp}
              onChange={(e) => setDirectIp(e.target.value)}
              placeholder="对方 IP（如 192.168.1.100）"
              className="w-44 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
            />
            <input
              value={directCode}
              onChange={(e) => setDirectCode(e.target.value)}
              placeholder="对方配对码"
              className="w-32 rounded-md border border-border bg-background px-3 py-1.5 text-sm font-mono"
            />
            <Button
              size="sm"
              variant="outline"
              disabled={busy || !directIp.trim() || !directCode.trim()}
              onClick={() => void doPair(directIp, directCode)}
            >
              {busy ? "配对中…" : "配对"}
            </Button>
          </div>
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
                {syncingId === p.deviceId
                  ? "同步中…"
                  : mode[p.deviceId] === "selected"
                    ? "选择笔记并同步…"
                    : "同步"}
              </Button>
              <Button size="sm" variant="ghost" onClick={() => void removePeer(p.deviceId)}>
                移除
              </Button>
            </div>
            {mode[p.deviceId] === "selected" && (
              <p className="text-[11px] text-muted-foreground">
                点击「同步」后弹窗勾选：从对方下载哪些笔记、把本机哪些笔记发送给对方。
              </p>
            )}
          </div>
        ))}
      </div>

      {selectingPeer && (
        <SelectiveSyncDialog
          peer={selectingPeer}
          onClose={() => setSelectingPeer(null)}
          onDone={(r) => {
            setSyncResult((prev) => ({
              ...prev,
              [selectingPeer.deviceId]: r,
            }));
            setMsg(
              `选择性同步完成：下载 ${r.pulled} · 发送 ${r.pushed} · 冲突 ${r.conflicts}` +
                (r.errors.length > 0 ? ` · 失败 ${r.errors.length}` : "")
            );
            void refreshPeers();
          }}
        />
      )}

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

      <StatusBanner msg={msg} />
    </div>
  );
}
