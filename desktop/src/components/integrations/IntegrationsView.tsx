import { useEffect, useState } from "react";
import { api } from "@/lib/tauri";
import type {
  FeedPollResult,
  FeedSubscription,
  MailAccount,
  MailSyncResult,
  StoredEmail,
} from "@/types";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

/** Format an RFC3339 timestamp as local "YYYY-MM-DD HH:MM". */
function fmtTime(iso: string | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

type TabId = "feeds" | "mail" | "mcp";

const TABS: { id: TabId; label: string }[] = [
  { id: "feeds", label: "订阅源" },
  { id: "mail", label: "邮件" },
  { id: "mcp", label: "MCP" },
];

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span className="text-muted-foreground">{label}</span>
      {children}
    </label>
  );
}

// ── Feeds tab ──────────────────────────────────────────────────────────

function FeedsTab() {
  const [feeds, setFeeds] = useState<FeedSubscription[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [newUrl, setNewUrl] = useState("");
  const [newTitle, setNewTitle] = useState("");
  const [newTags, setNewTags] = useState("");
  const [newInterval, setNewInterval] = useState("60");
  const [saving, setSaving] = useState(false);
  /** Per-feed refresh spinner. */
  const [refreshingId, setRefreshingId] = useState<string | null>(null);
  const [refreshingAll, setRefreshingAll] = useState(false);
  /** Last per-feed poll outcomes, keyed by feed id. */
  const [lastResults, setLastResults] = useState<Record<string, FeedPollResult>>({});

  const reload = async () => {
    try {
      setError(null);
      setFeeds(await api.listFeeds());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
  }, []);

  const handleAdd = async () => {
    if (!newUrl.trim() || saving) return;
    setSaving(true);
    try {
      // Empty kind auto-detects from the URL on the backend; empty title falls
      // back to the URL there too.
      const interval = parseInt(newInterval, 10);
      const feed = await api.addFeed(
        newUrl.trim(),
        newTitle.trim(),
        "",
        "",
        newTags.trim(),
        Number.isFinite(interval) && interval > 0 ? interval : 60
      );
      setFeeds((prev) => [feed, ...prev]);
      setNewUrl("");
      setNewTitle("");
      setNewTags("");
      setNewInterval("60");
      setAddOpen(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleToggle = async (feed: FeedSubscription) => {
    try {
      const ok = await api.setFeedEnabled(feed.id, !feed.enabled);
      if (ok) {
        setFeeds((prev) =>
          prev.map((f) => (f.id === feed.id ? { ...f, enabled: !f.enabled } : f))
        );
      }
    } catch (e) {
      setError(String(e));
    }
  };

  const handleRemove = async (id: string) => {
    if (!window.confirm("确定删除这个订阅源吗？已导入的笔记不会被删除。")) return;
    try {
      const ok = await api.removeFeed(id);
      if (ok) setFeeds((prev) => prev.filter((f) => f.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  const handleRefreshAll = async () => {
    if (refreshingAll) return;
    setRefreshingAll(true);
    try {
      const results = await api.refreshFeeds();
      const byId: Record<string, FeedPollResult> = {};
      for (const r of results) byId[r.feedId] = r;
      setLastResults(byId);
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshingAll(false);
    }
  };

  const handleRefreshOne = async (id: string) => {
    if (refreshingId) return;
    setRefreshingId(id);
    try {
      const r = await api.refreshFeed(id);
      setLastResults((prev) => ({ ...prev, [id]: r }));
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setRefreshingId(null);
    }
  };

  if (loading) {
    return <p className="p-4 text-sm text-muted-foreground">加载中…</p>;
  }

  return (
    <div className="flex flex-col gap-3 p-4">
      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}
      <div className="flex items-center gap-2">
        <Button size="sm" onClick={handleRefreshAll} disabled={refreshingAll}>
          {refreshingAll ? "刷新中…" : "全部刷新"}
        </Button>
        <Button size="sm" variant="secondary" onClick={() => setAddOpen((v) => !v)}>
          {addOpen ? "取消" : "添加订阅"}
        </Button>
        <span className="text-xs text-muted-foreground">
          新条目会自动转为笔记存入 vault
        </span>
      </div>

      {addOpen && (
        <div className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <Field label="订阅地址 (RSS / Atom / JSON)">
            <Input
              value={newUrl}
              onChange={(e) => setNewUrl(e.target.value)}
              placeholder="https://example.com/feed.xml"
            />
          </Field>
          <div className="grid grid-cols-2 gap-2">
            <Field label="标题（留空则自动识别）">
              <Input
                value={newTitle}
                onChange={(e) => setNewTitle(e.target.value)}
                placeholder="示例博客"
              />
            </Field>
            <Field label="标签（逗号分隔）">
              <Input
                value={newTags}
                onChange={(e) => setNewTags(e.target.value)}
                placeholder="tech, news"
              />
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Field label="轮询间隔（分钟）">
              <Input
                inputMode="numeric"
                value={newInterval}
                onChange={(e) => setNewInterval(e.target.value)}
                placeholder="60"
              />
            </Field>
          </div>
          <div>
            <Button size="sm" onClick={handleAdd} disabled={!newUrl.trim() || saving}>
              {saving ? "保存中…" : "保存订阅"}
            </Button>
          </div>
        </div>
      )}

      {feeds.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          还没有订阅源。点击「添加订阅」开始从 RSS / Atom / JSON 源自动导入笔记。
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {feeds.map((f) => {
            const r = lastResults[f.id];
            return (
              <li
                key={f.id}
                className="flex flex-col gap-1 rounded-md border border-border bg-card p-3"
              >
                <div className="flex items-center gap-2">
                  <span
                    className={cn(
                      "h-2 w-2 shrink-0 rounded-full",
                      f.enabled ? "bg-green-500" : "bg-muted-foreground/40"
                    )}
                    title={f.enabled ? "已启用" : "已停用"}
                  />
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {f.title || f.url}
                  </span>
                  <span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[11px] text-muted-foreground">
                    {f.kind || "rss"}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleRefreshOne(f.id)}
                    disabled={refreshingId === f.id || !f.enabled}
                    title="立即刷新这个订阅源"
                  >
                    {refreshingId === f.id ? "…" : "刷新"}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleToggle(f)}
                    title={f.enabled ? "停用" : "启用"}
                  >
                    {f.enabled ? "停用" : "启用"}
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleRemove(f.id)}
                    title="删除订阅源"
                  >
                    删除
                  </Button>
                </div>
                <div className="truncate text-xs text-muted-foreground">{f.url}</div>
                <div className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
                  <span>每 {f.intervalMinutes} 分钟</span>
                  <span>上次抓取 {fmtTime(f.lastFetchedAt)}</span>
                  {f.lastStatus && <span>状态 {f.lastStatus}</span>}
                  {f.lastError && (
                    <span className="text-destructive">{f.lastError}</span>
                  )}
                  {r && r.newEntries > 0 && (
                    <span className="text-green-600">新增 {r.newEntries} 篇</span>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

// ── Mail tab ───────────────────────────────────────────────────────────

function MailTab({ desktop }: { desktop: boolean }) {
  const [accounts, setAccounts] = useState<MailAccount[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [newName, setNewName] = useState("");
  const [newHost, setNewHost] = useState("");
  const [newPort, setNewPort] = useState("993");
  const [newUser, setNewUser] = useState("");
  const [newPass, setNewPass] = useState("");
  const [saving, setSaving] = useState(false);
  const [syncingId, setSyncingId] = useState<string | null>(null);
  /** Last sync outcome per account. */
  const [lastSync, setLastSync] = useState<Record<string, MailSyncResult>>({});
  /** Email search. */
  const [query, setQuery] = useState("");
  const [searching, setSearching] = useState(false);
  const [results, setResults] = useState<StoredEmail[] | null>(null);

  const reload = async () => {
    try {
      setError(null);
      setAccounts(await api.listMailAccounts());
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    if (desktop) void reload();
    else setLoading(false);
  }, [desktop]);

  // Mobile builds drop the `email` feature (OpenSSL can't cross-compile), so
  // the backend commands don't exist there.
  if (!desktop) {
    return (
      <p className="p-4 text-sm text-muted-foreground">
        邮件导入仅支持桌面端（移动端构建不含 IMAP 功能）。请在 Windows / Linux
        桌面应用中使用。
      </p>
    );
  }

  const handleAdd = async () => {
    if (!newName.trim() || !newHost.trim() || !newUser.trim() || saving) return;
    setSaving(true);
    try {
      const port = parseInt(newPort, 10);
      const acc = await api.addMailAccount(
        newName.trim(),
        newHost.trim(),
        Number.isFinite(port) ? port : 993,
        newUser.trim(),
        newPass,
        true,
        30
      );
      setAccounts((prev) => [acc, ...prev]);
      setNewName("");
      setNewHost("");
      setNewPort("993");
      setNewUser("");
      setNewPass("");
      setAddOpen(false);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    if (!window.confirm("确定删除这个邮箱账户吗？已导入的邮件笔记不会被删除。"))
      return;
    try {
      const ok = await api.deleteMailAccount(id);
      if (ok) setAccounts((prev) => prev.filter((a) => a.id !== id));
    } catch (e) {
      setError(String(e));
    }
  };

  const handleSync = async (id: string) => {
    if (syncingId) return;
    setSyncingId(id);
    try {
      const r = await api.syncMailAccount(id);
      setLastSync((prev) => ({ ...prev, [id]: r }));
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setSyncingId(null);
    }
  };

  const handleSearch = async () => {
    if (!query.trim() || searching) return;
    setSearching(true);
    try {
      setResults(await api.searchEmails(query.trim(), 50, 0));
    } catch (e) {
      setError(String(e));
    } finally {
      setSearching(false);
    }
  };

  if (loading) {
    return <p className="p-4 text-sm text-muted-foreground">加载中…</p>;
  }

  return (
    <div className="flex flex-col gap-3 p-4">
      {error && (
        <div className="rounded-md border border-destructive/50 bg-destructive/10 px-3 py-2 text-sm text-destructive">
          {error}
        </div>
      )}
      <div className="flex items-center gap-2">
        <Button size="sm" variant="secondary" onClick={() => setAddOpen((v) => !v)}>
          {addOpen ? "取消" : "添加邮箱"}
        </Button>
        <span className="text-xs text-muted-foreground">
          通过 IMAP 同步收件箱，新邮件会自动转为笔记
        </span>
      </div>

      {addOpen && (
        <div className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <div className="grid grid-cols-2 gap-2">
            <Field label="名称">
              <Input
                value={newName}
                onChange={(e) => setNewName(e.target.value)}
                placeholder="工作邮箱"
              />
            </Field>
            <Field label="IMAP 服务器">
              <Input
                value={newHost}
                onChange={(e) => setNewHost(e.target.value)}
                placeholder="imap.gmail.com"
              />
            </Field>
          </div>
          <div className="grid grid-cols-2 gap-2">
            <Field label="端口">
              <Input
                inputMode="numeric"
                value={newPort}
                onChange={(e) => setNewPort(e.target.value)}
                placeholder="993"
              />
            </Field>
            <Field label="用户名（邮箱地址）">
              <Input
                value={newUser}
                onChange={(e) => setNewUser(e.target.value)}
                placeholder="you@example.com"
              />
            </Field>
          </div>
          <Field label="密码 / 应用专用密码（加密存储）">
            <Input
              type="password"
              value={newPass}
              onChange={(e) => setNewPass(e.target.value)}
              placeholder="••••••••"
            />
          </Field>
          <div>
            <Button
              size="sm"
              onClick={handleAdd}
              disabled={!newName.trim() || !newHost.trim() || !newUser.trim() || saving}
            >
              {saving ? "保存中…" : "保存账户"}
            </Button>
          </div>
        </div>
      )}

      {accounts.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          还没有邮箱账户。点击「添加邮箱」配置 IMAP 同步。
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {accounts.map((a) => {
            const r = lastSync[a.id];
            return (
              <li
                key={a.id}
                className="flex flex-col gap-1 rounded-md border border-border bg-card p-3"
              >
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {a.name}
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {a.username}
                  </span>
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => handleSync(a.id)}
                    disabled={syncingId === a.id}
                    title="立即同步这个账户"
                  >
                    {syncingId === a.id ? "同步中…" : "同步"}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => handleDelete(a.id)}>
                    删除
                  </Button>
                </div>
                <div className="flex flex-wrap gap-x-3 text-xs text-muted-foreground">
                  <span>
                    {a.host}:{a.port}
                  </span>
                  <span>上次同步 {fmtTime(a.lastSyncAt)}</span>
                  {r && (
                    <span>
                      抓取 {r.fetched} / 入库 {r.imported} / 跳过{" "}
                      {r.skippedDuplicates}
                    </span>
                  )}
                  {r && r.errors.length > 0 && (
                    <span className="text-destructive">{r.errors.join("; ")}</span>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}

      <div className="mt-2 flex flex-col gap-2 border-t border-border pt-3">
        <div className="text-sm font-medium">搜索已导入邮件</div>
        <div className="flex gap-2">
          <Input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void handleSearch();
            }}
            placeholder="按主题 / 发件人 / 正文搜索…"
          />
          <Button size="sm" onClick={handleSearch} disabled={!query.trim() || searching}>
            {searching ? "…" : "搜索"}
          </Button>
        </div>
        {results !== null && (
          <div className="text-xs text-muted-foreground">找到 {results.length} 封</div>
        )}
        {(results ?? []).map((m) => (
          <div
            key={m.id}
            className="rounded-md border border-border bg-card p-2 text-sm"
          >
            <div className="truncate font-medium">{m.subject || "(无主题)"}</div>
            <div className="truncate text-xs text-muted-foreground">
              {m.fromAddr} · {fmtTime(m.date)}
            </div>
            <div className="mt-1 line-clamp-2 text-xs text-muted-foreground">
              {m.bodyText.slice(0, 200)}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

// ── MCP tab ────────────────────────────────────────────────────────────

/** Build the client config snippet with the given token (never persisted). */
function mcpConfigSnippet(token: string): string {
  return `{
  "mcpServers": {
    "vaultpilot": {
      "command": "<vaultpilot-mcp 路径>",
      "args": ["--vault-dir", "<你的 vault 目录>", "--token", "${token}"],
      "env": { "VAULTPILOT_MCP_TOKEN": "${token}" }
    }
  }
}`;
}

/** Generate a fresh 32-byte hex token (client-side only, never stored). */
function generateToken(): string {
  const bytes = new Uint8Array(32);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function McpTab({ onOpenUrl }: { onOpenUrl: (url: string) => void }) {
  const [token, setToken] = useState<string | null>(null);
  const [copied, setCopied] = useState<"config" | "token" | null>(null);

  const snippet = mcpConfigSnippet(token ?? "<你的 token>");

  const copy = async (what: "config" | "token") => {
    try {
      await navigator.clipboard.writeText(
        what === "config" ? snippet : (token ?? "")
      );
      setCopied(what);
      setTimeout(() => setCopied(null), 2000);
    } catch {
      // Clipboard may be unavailable (permissions); the text is still
      // selectable as plain text below.
    }
  };

  return (
    <div className="flex flex-col gap-3 p-4 text-sm">
      <p className="text-muted-foreground">
        MCP（Model Context Protocol）让 Claude Desktop、Cursor、Codex 等外部 AI
        客户端直接读写你的 VaultPilot 笔记库。MCP 服务是独立进程（stdio），桌面
        应用内无需常驻开关——按下面配置接入即可。
      </p>

      <div className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
        <span className="font-medium">访问令牌（推荐）</span>
        <p className="text-xs text-muted-foreground">
          配置后只有持有令牌的客户端才能访问 vault：令牌通过
          <code className="mx-1 rounded bg-secondary px-1">--token</code>
          参数（服务端期望值）和客户端
          <code className="mx-1 rounded bg-secondary px-1">env</code>
          注入（持有证明）成对出现，不匹配的进程会被直接拒绝。不配置则任何本地进程都可访问（启动时会打警告）。
        </p>
        <div className="flex items-center gap-2">
          <Button size="sm" onClick={() => setToken(generateToken())}>
            生成新令牌
          </Button>
          {token && (
            <>
              <code className="min-w-0 flex-1 truncate rounded bg-secondary px-2 py-1 font-mono text-xs">
                {token}
              </code>
              <Button size="sm" variant="secondary" onClick={() => copy("token")}>
                {copied === "token" ? "已复制" : "复制令牌"}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => setToken(null)}
                title="清除（仅清除界面显示，不涉及任何存储）"
              >
                清除
              </Button>
            </>
          )}
        </div>
        <p className="text-xs text-muted-foreground">
          令牌只在本页面内存中生成、不落盘；请把它填进客户端配置后自行妥善保管。
          同一个值也可以写进 vault 的 mcp-config.json（
          <code className="rounded bg-secondary px-1">{"{\"token\": \"…\"}"}</code>
          ）替代 --token 参数。
        </p>
      </div>

      <div className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
        <div className="flex items-center justify-between">
          <span className="font-medium">客户端配置片段</span>
          <Button size="sm" variant="secondary" onClick={() => copy("config")}>
            {copied === "config" ? "已复制" : "复制"}
          </Button>
        </div>
        <pre className="overflow-x-auto rounded bg-secondary p-2 font-mono text-xs">
          {snippet}
        </pre>
        <ul className="list-disc pl-5 text-xs text-muted-foreground">
          <li>构建：cargo build --release -p vaultpilot-mcp</li>
          <li>Claude Desktop：编辑 claude_desktop_config.json，加入上面片段</li>
          <li>Codex / Cursor：加入 .cursor/mcp.json 或等效配置</li>
          <li>
            令牌不匹配的客户端会在启动时被拒绝（stderr 报 unauthorized）
          </li>
        </ul>
      </div>
      <div className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
        <span className="font-medium">可用工具</span>
        <ul className="grid grid-cols-2 gap-1 font-mono text-xs text-muted-foreground">
          <li>vault_search</li>
          <li>vault_read</li>
          <li>vault_write</li>
          <li>vault_list</li>
          <li>vault_related</li>
          <li>github_list_issues</li>
        </ul>
      </div>
      <div>
        <Button
          size="sm"
          variant="secondary"
          onClick={() =>
            onOpenUrl("https://modelcontextprotocol.io/docs/getting-started/intro")
          }
        >
          打开 MCP 官方文档
        </Button>
      </div>
    </div>
  );
}

// ── View ───────────────────────────────────────────────────────────────

export function IntegrationsView() {
  const [tab, setTab] = useState<TabId>("feeds");
  const [desktop, setDesktop] = useState(false);

  useEffect(() => {
    api
      .isDesktop()
      .then((v) => setDesktop(v))
      .catch(() => {});
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 items-center gap-1 border-b border-border px-4 pt-3">
        {TABS.map(({ id, label }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={cn(
              "rounded-t-md px-3 py-2 text-sm text-muted-foreground transition-colors hover:text-foreground",
              tab === id &&
                "border-b-2 border-primary font-medium text-foreground"
            )}
          >
            {label}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {tab === "feeds" && <FeedsTab />}
        {tab === "mail" && <MailTab desktop={desktop} />}
        {tab === "mcp" && (
          <McpTab
            onOpenUrl={(url) => {
              api.openExternalUrl(url).catch(() => {});
            }}
          />
        )}
      </div>
    </div>
  );
}
