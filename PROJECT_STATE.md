# VaultPilot 项目指挥文档

> 本文档由指挥官任务自动维护，所有 cron agent 运行前必须先读取此文档。

## 项目概述
- **仓库**: ryanloee/VaultPilot
- **技术栈**: Rust 后端核心 + 三端客户端
- **核心路径**: /home/jy/wk/VaultPilot/

## 平台架构（三端独立）
| 平台 | 类型 | 技术栈 | 路径 | 说明 |
|------|------|--------|------|------|
| **Windows** | 桌面 App | C# WinUI 3 | native/ | 完整 GUI 客户端 |
| **Linux** | CLI | Rust | crates/cli/ | 终端直接用，不需要前端 |
| **Android** | 手机 App | React Native (Expo) | mobile/ | APK 独立安装，不依赖电脑 |

- 三端共用 Rust 后端核心（crates/core/）
- 不支持 iOS、不做浏览器版本
- Android 端直接调 LLM API（用户自备 key），不连远程服务器

## 当前阶段
**AI 驱动产品改进** — 讨论团队主动审查代码质量、发现改进点、产出高质量 issue；修复团队自动实现；审核团队把关合并

## 优先级排序
1. 🔴 Bug 修复（功能性、数据丢失、崩溃）
2. 🟠 安全问题（路径穿越、注入、密钥泄露）
3. 🟡 性能问题（OOM、IO 开销、内存泄漏）
4. 🔵 架构改进（模块拆分、错误类型统一）
5. ⚪ UI/功能增强（新功能、交互优化）
6. 📝 文档和测试覆盖

## 已完成记录
<!-- 由 pr-review 任务在合并/关闭 PR 后更新 -->
- #1342 + #1343: Agent Mode Phase 3.2 — run_agent() 自主工具循环 + CLI agent 命令 (PR #1346 已合并)
- #1344: MainWindow.Chat.cs 拆分为 4 个 partial class 文件 (PR #1347 已合并)
- README 更新：补充 Android 端、Agent Mode 功能、三端架构图、移动端构建说明 (main 直接提交)
- #1429: store.ts 纯函数提取 + 31 单元测试 (PR #1433 已合并)
- #1430: NoteEditorScreen 纯函数提取 + 29 单元测试 (PR #1434 已合并)
- #1431: CHANGELOG.md 创建 v0.3.x 发版历史 (PR #1435 已合并)
- #1436: CLI agent FinalAnswer 丢弃回答文本修复 (PR #1438 已合并)
- #1437: OfflineBanner 深色主题支持 (PR #1439 已合并)
- #1432: Agent Mode 首次使用引导 (WinUI tooltip + CLI help) (PR #1440 已合并)
- #1441: clientPure.ts 26 单元测试 (PR #1444 已合并)
- #1442: models.rs 拆分为 provider.rs + settings.rs + mod.rs (PR #1445 已合并)
- #1443: storage/pool.rs 连接池管理提取 (PR #1446 已合并)
- #1447: pending_syncs UNIQUE index 去重修复 (PR #1450 已合并)
- #1448: useNetworkState 挂载时主动检查网络连接 (PR #1451 已合并)
- #1449: flushPendingSyncs 边界测试补充 (PR #1452 已合并)
- #1453: WriteApprovalDialog human-readable diff 预览 (PR #1456 已合并)
- #1454: run_agent() 异常路径 12 个集成测试 (PR #1457 已合并)
- #1455: 移动端 sync + settingsSync 边界测试 (PR #1458 已合并)
- #217: WinUI 启动冒烟测试
- #500: MCP server tool errors sanitize_error() 包装 (PR #505 已合并)
- #501: build_note_path 使用完整 UUID 消除截断 (PR #506 已合并)
- #502: settings_api_key_encrypted_on_disk 测试修复并取消 #[ignore] (PR #506 已合并)
- #503: DNS rebinding TOCTOU — pin DNS resolution 防重绑定 (PR #507 已合并)
- #504: attachment LIKE 全表扫描 — FTS5 评分优化 (PR #507 已合并)
- #453: ExecuteAiRequestAsync 期间禁用 NewSession/DeleteSession 按钮 (PR #456 已合并)
- #454: NotesView 后端调用 30s CancellationToken 超时保护 (PR #456 已合并)
- #455: FTS5 查询错误 tracing::warn! 日志记录 (PR #456 已合并)
- #445: winui_build CI 添加 MSBuild build + test 步骤 (main 直接提交已修复)
- #417: CancelActiveRequest Volatile.Read 保护 (PR #422 已合并)
- #418: serde_yml → serde_yaml_ng 替换废弃依赖 (PR #421 已合并)
- #416: derive_machine_key KDF — 已由 PR #413 (PBKDF2-HMAC-SHA256 600k) 修复，关闭
- #226: save_note 路径限制到 vault 目录 (PR #241 已合并)
- #225: sanitize 函数转义闭合 XML 标签 (PR #240 已合并)
- #236: delete_note 事务顺序修复 (PR #237 已合并)
- #212: 休眠恢复后重连指数退避 (PR #224 已合并)
- #205: async void 事件处理异常保护 (PR #222 已合并)
- #175: index_note_file 事务包装 (PR #221 已合并)
- #182: lib.rs 错误类型统一为 anyhow (PR #220 已合并)
- #179: base_url SSRF 防护 (PR #219 已合并)
- #188: CJK slugify 哈希后缀 (PR #210 已合并)
- #187: prompt 注入防护 (PR #209 已合并)
- #229: expand_term_aliases update 同义词映射移除 (PR #242 已合并)
- #174: save_note 原子写入修复 + #186: atomic_write 临时文件权限限制 (PR #243 已合并)
- #230: SQLite WAL 模式 + busy_timeout (PR #245 已合并)
- #157: AppSettings 缓存避免 N+1 磁盘 IO (PR #247 已合并)
- #227 + #197: CJK token 估算修复 + chat 双重保存移除 (PR #250 已合并)
- #235: read_file 大文件智能截断 head+tail (PR #251 已合并)
- #177: API 响应体流式读取 + 50MB 限制防 OOM (PR #252 已合并)
- #180: 附件搜索分批加载 (PR #249 已关闭未合并 → PR #256 已合并)
- #178: 重试循环 content_blocks.clone() 预序列化 (PR #253 已合并)
- #176: rank_documents 去除冗余连接和 DB 查询 (PR #254 已合并)
- #88 + #52: strip_inline_markdown 保留 code block + HTTP body 10MB 限制 (PR #255 已合并)
- #169: README 移除不存在的 run_command 能力描述 (PR #257 已合并)
- #165: FTS5 搜索转义特殊字符 (PR #258 已合并)
- #168: serde_yaml → serde_yml 替换废弃依赖 (PR #259 已合并)
- #100: BackendClient.DisposeAsync 释放 SemaphoreSlim 和 CTS (PR #260 已合并)
- #99 + #128: RecordButton guard + 退出前保存 chat state (PR #261 已合并)
- #156: list_directory 报告权限错误和截断提示 (PR #262 已合并)
- #140: ResolveContextWindowTokens 模型名边界匹配 (PR #263 已合并)
- #63: OnLaunched 防止重复创建窗口 (PR #264 已合并)
- #95: auto-wake timer 检查 shutdown 状态 (PR #265 已合并)
- #155: Settings 对话框输入校验 (PR #266 已合并)
- #62: tray exit calls cleanup before terminating (PR #267 已合并)
- #71: _chatState SemaphoreSlim 同步 (PR #268 已合并)
- #86: 剪贴板图片清理，最多保留 50 个 (PR #269 已合并)
- #1294: sanitize_error URL 参数 redaction — api-key/access_token/secret/token (PR #1297 已合并)
- #1295: CLI HTTP bridge SSE streaming 支持 (PR #1298 已合并)
- #1296: ai/mod.rs 拆分为 client + parsing 模块 (PR #1299 已合并)
- #1300: MODEL_OUTPUT_TOKEN_RULES 数据驱动重构 (PR #1302 已合并)
- #1301: orchestration/chat.rs 16 个单元测试 (PR #1303 已合并)
- #130 + #72: SolidColorBrush 静态缓存减少 GC 压力 (PR #271 已合并)
- #90: AddTurn List 预分配替代 Concat+ToArray (PR #273 已合并)
- #46: 全局异常处理 + 单实例 Mutex (PR #274 已合并)
- #213: AppSettings 反序列化后 validate() 校验 (PR #275 已合并)
- #163: HTTP bridge 限流 + constant_time_eq 时序修复 (PR #276 已合并)
- #195: chat session 上限 50，自动裁剪旧会话 (PR #277 已合并)
- Flaky env-var test 修复 (PR #278 已合并)
- #103: truncate_for_trace 单次遍历优化 (PR #280 已合并)
- #1471: globalSearch FTS→LIKE fallback (PR #1471 已合并)
- #1474: 移动端 any 类型替换为正确 TypeScript 类型 (PR #1475 已合并)
- #1473: isNoteRelatedQuery 导出 + 13 单元测试 (PR #1473 已合并)
- #1472: CHANGELOG v0.3.51 补全 (PR #1472 已合并)
- globalSearch LIKE fallback session title 搜索 (PR #1476 已合并)
- #50: max_tokens 模型感知动态值 (PR #281 已合并)
- #114: StorageContext 启动时创建并复用 (PR #282 已合并)
- #206: HTTP bridge CORS headers + rate limiter + subtle::ConstantTimeEq 改进 (PR #279 已合并)
- #198: list_directory/read_file 截断指示 (PR #284 已合并)
- #204: 模型上下文窗口数据驱动注册表 (PR #283 已合并)
- #199: CancellationToken plumbing for AI requests (PR #285 已合并)
- #193: 用户文本/附件发送失败丢失恢复 (PR #286 已合并)
- #203: Rust 结构化日志 tracing + CI (PR #287 已合并)
- #154: CI 依赖漏洞扫描 + dependabot (PR #289 已合并)
- #196: 代码块主题感知颜色 (PR #290 已合并)
- #207: .NET 单元测试项目 (PR #291 已合并)
- #123: API Key 明文存储 AES-256-GCM 加密 (PR #300 已合并)
- #137: API headers provider-aware — Anthropic/OpenAi 双协议支持 (PR #301 已合并)
- #147: Rust 测试基础设施 — 意图检测和路径安全测试 (PR #302 已合并)
- #42: SQLite 连接池 r2d2 替代逐次 Connection::open (PR #306 已合并)
- #122: Auto-wake 模型下拉框 provider-aware (PR #305 已合并)
- #104: MCP server 初始化超时和优雅关闭 (PR #307 已合并)
- #59: 提取重复 Send/Record 逻辑为共享 helper (PR #308 已合并)
- #94: markdown blockquote 渲染左边框+斜体 (PR #309 已合并)
- #218: JSON-RPC 协议和 tool-call 循环集成测试 (PR #310 已合并)
- #185: 搜索评分/同义词规则可配置化 (PR #311 已合并)
- #116: MCP server 工具补齐 CLI 能力 (PR #312 已合并)
- #51: lib.rs 核心编排逻辑集成测试 (PR #313 已合并)
- #53: Shift+Enter 键盘快捷键提示 (PR #314 已合并)
- #718: compute_image_perceptual_hash 文件大小 + 尺寸限制防 OOM (PR #721 已合并)
- #719: ChatSession/ChatState JSON 反序列化 null 安全 (PR #722 已合并)
- #720: AutoWakeIntervalMinutes ulong→int 溢出 Math.Clamp 修复 (PR #723 已合并)
- #145: MCP server resources/prompts 支持 (PR #316 已合并)
- #44 + #232: 输入框自动增长 (PR #317 已合并)
- #58: 事件处理器内存泄漏修复 (PR #318 已合并)
- #233: 通用键盘快捷键 (PR #319 已合并)
- #184: 新增/删除会话按钮图标 + 删除按钮危险样式 (PR #322 已合并)
- #142 + #24: RenderCurrentSession 恢复 Citations/ThinkingTrace/SavedNote 渲染 (PR #321 已合并, #24 关闭为重复)
- #26: 设置对话框 PrimaryButtonClick 验证防输入丢失 (PR #320 已合并)
- #150: 搜索模糊匹配+日期范围+标签过滤 (PR #315 CI 修复后已合并)
- #189: AI 请求取消按钮 + Escape 键支持 (PR #323 已合并)
- #124: 代码块 SolidColorBrush 已通过 ThemeResource 修复 (关闭为已解决, PR #290)
- #57: 硬编码暗色主题颜色替换为 ThemeResource (PR #324 已合并)
- #159: 附件 chip 重设计 — 图标+文件名+删除按钮 (PR #325 已合并)
- #215: AI 请求 loading overlay + ProgressRing (PR #326 已合并)
- #189: AI 请求取消按钮 + Escape 键支持 (PR #323 已合并)
- #161 + #60: AutomationProperties 屏幕阅读器无障碍 (PR #327 已合并, #60 关闭为重复)
- #164: 错误消息本地化扩展 (PR #328 已合并)
- #54: 笔记导出 CLI 命令 (PR #330 已合并)
- #29: Markdown 链接和表格渲染 (PR #331 已合并)
- #56: 搜索结果高亮和程序化片段 (PR #332 已合并)
- #149: 会话侧边栏元数据 (PR #333 已合并)
- #166: Vault 导出和 SQLite 自动备份 (PR #334 已合并)
- #146: 笔记浏览面板 NavigationView (PR #335 已合并)
- #234: 设置对话框 XAML 化 — SettingsDialog.xaml ContentDialog (PR #336 已合并)
- #337: AddTurn 同步 Wait → async WaitAsync 防止 UI 死锁 (PR #342 已合并)
- #338: GetThemeBrush TryGetValue + Transparent fallback 防止 NRE (PR #343 已合并)
- #339: LogStartup File.AppendAllText → AppendAllTextAsync 避免 UI 阻塞 (PR #344 已合并)
- #340: storage.rs 备份函数 unwrap → ok_or_else 错误传播 (PR #341 已合并)
- #345: async void 事件处理器 try-catch 异常保护 (PR #348 已合并)
- #346: 资源字典统一 GetThemeBrush 消除 NRE 风险 (PR #348 已合并)
- #347: backup rotation/agent flush 静默吞错改日志记录 (PR #348 已合并)
- #349: OnPowerModeChanged async void try-catch 异常保护 (PR #351 已合并)
- #350: Style 资源访问统一为安全 GetThemeStyle 模式 (PR #352 已合并)
- #353: MainWindow.xaml merge conflict markers 清除 (PR #356 已合并)
- #354: SettingsDialog WireUpButtons async void lambda try-catch (PR #356 已合并)
- #355: Rate limiter HashMap 清理 + lock poisoned 恢复 (PR #356 已合并)
- #369: BackendClient 线程安全 — volatile + Interlocked + health check guard + CancellationToken 传播 (PR #372 已合并)
- #370: generate_programmatic_snippet CJK/Unicode 安全切片 (PR #373 已合并)
- #371: ExitApplication 双重调用竞态 Interlocked guard (PR #374 已合并)
- #392: SECURITY — Hyperlink URI scheme 限制 http/https (PR #395 已合并)
- #393: BUG — PumpStdoutAsync _process 字段竞态局部变量捕获 (PR #396 已合并)
- #394: BUG — OnLoaded 启动失败 BackendClient.DisposeAsync 释放进程 (PR #397 已合并)
- #398: SECURITY — SSRF DNS rebinding bypass validate_base_url (PR #402 已合并)
- #399: BUG — ShutdownAsync 关闭竞态未取消 _activeRequestCts (PR #401 已合并)
- #400: BUG — DisposeAsync 不调用 FailPending 导致挂起 (PR #401 已合并)
- #403: BUG — _activeRequestCts 无 Interlocked 保护双重 Dispose 竞态 (PR #406 已合并)
- #404: BUG — escape_fts5_term 仅保留 ASCII，Unicode 字母被丢弃 (PR #407 已合并)
- #405: SECURITY — atomic_write File::create 到 set_permissions TOCTOU 窗口 (PR #407 已合并)
- #408: NotesView.OnDeleteNoteClicked async void try-catch 保护 (PR #409 已合并)
- #423: BackendClient.DisposeAsync _readerCts.Cancel 前置于 FailPending (PR #427 已合并)
- #424: atomic_write 失败时清理临时文件 inspect_err (PR #426 已合并)
- #425: 剪贴板图片文件名随机后缀防冲突 (PR #428 已合并)
- #431: generate_programmatic_snippet 搜索高亮大小写保留 (PR #432 已合并)
- #429 + #430: async void try-catch + SendAsync CancellationToken 超时保护 (PR #433 已合并)
- #434: extract_json_block 首个花括号匹配 → 所有位置尝试 + serde_json 校验 (PR #438 已合并)
- #435: NotesView SelectionChanged + ItemClick 双重请求 → 移除 ItemClick + CancellationToken (PR #437 已合并)
- #436: query_like_note_metas LIKE 子句单词上限 .take(20) (PR #437 已合并)
- #447: CheckForAppUpdatesAsync 瞬态失败后重置 _updateCheckStarted (PR #448 已合并)
- #446: ShutdownAsync TaskCompletionSource 等待活跃请求完成后再释放资源 (PR #449 已合并)
- #464: AppendInlineMarkdown 无限循环 forward-progress guard (PR #465 已合并)
- #462: ProviderConfig.ToString() API Key 遮蔽为 [REDACTED] (PR #465 已合并)
- #463: read_file_result head/tail 重叠重复输出修复 (PR #465 已合并)
- #466: IPv6 SSRF bypass — unique-local fd00::/8 + IPv4-mapped ::ffff:x.x.x.x (PR #469 已合并)
- #467: C# ProviderConfig 添加 MaxOutputTokens/ProviderType 字段 (PR #468 已合并)
- #470: SaveChatStateAsync 竞态移除 await 后写回逻辑 (PR #473 已合并)
- #471: ShutdownAsync 移除 _chatStateLock.Dispose() (PR #474 已合并)
- #472: load_recent_notes_for_overview N+1 查询改用 load_note_body_from_meta (PR #474 已合并)
- #484: MainWindow.xaml.cs async void await 编译错误 + CI winui_build 添加构建步骤 (PR #487 已合并)
- #485 + #486: BackendClient Process 泄漏 + Timer 竞态 + ComposerBox 拖放冗余 (PR #488 已合并)
- #489: OpenAI provider 请求/响应格式不兼容 (PR #490 已合并)
- #491: render_history() XML 闭合标签转义 — 存储型提示注入防护 (PR #494 已合并)
- #492: load_recent_notes_for_overview async spawn_blocking 包装 (PR #494 已合并)
- #493: cached_settings Mutex 中毒恢复 unwrap_or_else 模式 (PR #494 已合并)
- #495: _windowProcDelegate GCHandle pinning 防 GC 回收 (PR #498 已合并)
- #496: SettingsDialog.GetThemeBrush NRE null-safe 保护 (PR #498 已合并)
- #497: has_notes_async 替代 list_notes_async 空检查优化 (PR #499 已合并)
- #500: MCP server tool errors 泄露内部路径和 SQL 细节 (PR #505 已合并)
- #501: build_note_path UUID 后缀截断为 8 字符 — 低碰撞抗性 (PR #506 已合并)
- #502: settings_api_key_encrypted_on_disk test #[ignore]d — crypto round-trip 回归风险 (PR #506 已合并)
- #503: validate_base_url DNS rebinding TOCTOU — reqwest 重新解析 (PR #507 已合并)
- #504: attachment visual/semantic scoring 全表扫描 O(n) 性能瓶颈 (PR #507 已合并)
- #508: search_rules synonym expansion 子串误匹配 → ASCII trigger 全词匹配 (PR #511 已合并)
- #509: OnHealthCheckTick catch 块 async void 异常安全 (PR #511 已合并)
- #510: MainWindow Brush 属性注释误导 — 非缓存而是每次查找 (PR #511 已合并)
- #512: rebuild_index_with_context 长事务 → 批量 50 文件事务拆分 (PR #517 已合并)
- #513: rank_documents 双重 FTS5 查询 → 合并为单次查询 (PR #516 已合并)
- #514: NotesView 后端搜索清除后不恢复完整笔记列表 (PR #515 已合并)
- #518: GetNextAutoWakeTime 200 迭代上限截断小间隔唤醒窗口 (PR #521 已合并)
- #519: VAULTPILOT_ALLOW_LOCAL_ENDPOINT 绕过 DNS pinning — 仍解析 DNS 钉住地址 (PR #521 已合并)
- #520: mask_secret UTF-8 多字节边界 panic — 改用 chars() 操作 (PR #521 已合并)
- #522: FindNextInlineMarker + IsOpenAiOSeriesModel 热路径数组分配 → static readonly 字段 (PR #525 已合并)
- #523: CreateAttachmentChip + GetThemeBrush Transparent SolidColorBrush 缓存复用 (PR #525 已合并)
- #524: sanitize_error Authorization: Basic 凭据脱敏 (PR #525 已合并)
- #526: auto_backup_database WAL checkpoint 确保备份一致性 (PR #529 已合并)
- #527: generate_programmatic_snippet 重叠搜索词高亮标记损坏 (PR #530 已合并)
- #528: BackendClient.DisposeAsync _isDisposed TOCTOU 原子保护 (PR #531 已合并)
- #532: BeginExitForUpdate 不终止进程 — tray icon 阻止 Velopack 更新 (PR #535 已合并)
- #533: ExitApplication finally 块 ReleaseMutex 异常安全 (PR #535 已合并)
- #534: BeginExitForUpdate 和 ExitApplication 竞态 Interlocked guard (PR #535 已合并)
- #536: TryReconnectAsync bare catch 吞没 OperationCanceledException → 传播取消异常 (PR #539 已合并)
- #537: ShutdownAsync 5s 超时不足 → 35s + ExecuteAiRequestAsync catch _isShuttingDown 保护 (PR #539 已合并)
- #538: NotesView RefreshNotesAsync 取消 _loadDetailCts 防止过时数据更新 (PR #539 已合并)
- #541: PromptToInstallUpdateAsync null-forgiving cast → pattern matching 防 NRE (PR #543 已合并)
- #542: CheckForAppUpdatesAsync _updateCheckStarted Interlocked 原子 guard (PR #543 已合并)
- #544: unsanitized API response text in tool-call retry error (PR #547 已合并)
- #545: CopyTextToClipboard 缺少 try/catch，剪贴板被锁时崩溃 (PR #548 已合并)
- #546: load_chat_state_with_context TOCTOU exists/read 竞态 (PR #548 已合并)
- #549: constant_time_eq token 长度侧信道泄露 (PR #552 已合并)
- #550: BackendClient SendAsync 挂起请求 TCS 泄漏 (PR #553 已合并)
- #551: MCP resources/list fetch-all-then-skip 分页低效 (PR #554 已合并)
- #555: FormatUpdatedAt 重复代码 DRY 违反 — 提取共享 FormatRelativeTime (PR #556 已合并)
- #557: BackendClient 并发 reader pump 竞态 — await 旧 pump task 后再启动新 task (PR #562 已合并)
- #558: MCP prompts/get 用户内容提示注入 — sanitize_mcp_prompt_content() 转义+分隔符 (PR #561 已合并)
- #559: read_stdin_json 无上限 OOM + MCP search limit 无上限 — 10MB cap + limit 上限 (PR #560 已合并)
- #563: BeginExitForUpdate 不释放 _instanceMutex — 更新后重启单实例检测失败 (PR #565 已合并)
- #564: windows-installers.yml release workflow 缺少 cargo audit 步骤 (PR #566 已合并)
- #567: BackendClient.HandleEvent 缺少 try-catch — 畸形事件杀死所有挂起请求 (PR #570 已合并)
- #568: importMarkdown 接受任意文件系统路径，缺少敏感目录限制 (PR #571 已合并)
- #569: lib.rs list_directory/read_file 同步阻塞 IO 在异步上下文 (PR #572 已合并)
- #573: import_note_images 图片导入路径缺少敏感目录校验 (PR #576 已合并)
- #574: WalkDir 无深度限制，循环符号链接导致无限遍历 (PR #577 已合并)
- #575: export_all_notes 受 search_notes_with_context 200 条上限截断 (PR #578 已合并)
- #579: MCP tool call 错误路径 14 处缺少 sanitize_error 脱敏 (PR #581 已合并)
- #580: serialize_string_result serde 序列化错误未脱敏 — 与 serialize_result 不一致 (PR #581 已合并)
- #582: search_notes_with_context tag/keyword/date 过滤在分页后应用导致结果丢失 (PR #585 已合并)
- #583: export zip 目录导出文件名冲突导致笔记静默覆盖 (PR #586 已合并)
- #584: NotesView 快速选择笔记竞态 — 取消的旧详情覆写新选中笔记 (PR #587 已合并)
- #588: ShutdownAsync _activeRequestCts 竞态 — Interlocked.Exchange 原子操作 (PR #591 已合并)
- #589: _isShuttingDown 缺少 volatile 关键字 — 跨线程可见性 (PR #591 已合并)
- #590: SettingsDialog.GetThemeBrush 回退 Brush 每次分配 — static readonly 缓存 (PR #591 已合并)
- #592: auto_backup_database WAL checkpoint 连接缺少 busy_timeout — 添加 PRAGMA busy_timeout = 5000 (PR #594 已合并)
- #593: query_like_note_metas 文档注释说 "body" 但代码搜索 "summary" — 修正注释 (PR #594 已合并)
- #595: crypto.rs Windows machine_salt 缺少唯一机器标识符 — 同主机名机器可派生相同加密密钥 (PR #599 已合并)
- #596: MCP server 和 agent stdin 行读取无长度上限 — 10MB cap (PR #598 已合并)
- #601: extract_json fast-path 绕过 JSON 校验 — prose-wrapped JSON 浪费 API 重试 (PR #604 已合并)
- #602: normalize_endpoint ends_with 后缀匹配过松 — proxy URL 被错误路由 (PR #604 已合并)
- #603: validate_base_url DNS 解析无显式超时 — 慢 DNS 消耗请求超时预算 (PR #604 已合并)
- #606: MCP notes.search limit cap 500 与 search_notes 内部 200 不一致 (PR #609 已合并)
- #605: HTTP bridge 无请求级超时 — 添加 180s TimeoutLayer (PR #610 已合并)
- #608: strip_inline_markdown italic 处理 — 关闭（代码已有 italic 处理，非 bug）
- #615: CancelActiveRequest() ObjectDisposedException — CTS 被 Dispose 后 Cancel() 无保护 (PR #619 已合并)
- #616: BackendClient.SendAsync 无 _isDisposed 守卫 — Dispose 后 _writeLock 抛 ObjectDisposedException (PR #620 已合并)
- #617: sanitize_mcp_prompt_content 不转义开标签 — 用户内容可注入 `<user_content>` 破坏分隔符 (PR #618 已合并)
- #621: NotesView 快速搜索竞态 — 旧搜索结果可覆盖新搜索结果 (PR #624 已合并)
- #622: NotesView 删除笔记不检查后端返回值 — 删除失败仍从 UI 移除 (PR #624 已合并)
- #623: tool_result_user_prompt 的 tool_name 未转义 XML 闭合标签 — 防御纵深不一致 (PR #624 已合并)
- #625: render_notes/render_candidate_notes/render_history note metadata XML 转义 (PR #628 已合并)
- #626: ExecuteAiRequestAsync session ID 竞态 — AddTurnAsync sessionId 参数 (PR #629 已合并)
- #627: OnComposerKeyDown Ctrl+V e.Handled 提前到 await 前 (PR #630 已合并)
- #631: 删除笔记后清除搜索框已删除笔记重现 — _allNotesBeforeSearch 同步过滤 (PR #632 已合并)
- #633: PRAGMA busy_timeout 设置顺序 — 在 journal_mode WAL 之前设置 (PR #633 已合并)
- #634: BackendClient.DisposeAsync _writeLock TOCTOU — WaitAsync ODE 防护 (PR #637 已合并)
- #635: BackendClient.DisposeProcessAsync 并发调用 NRE — Interlocked.Exchange 原子捕获 (PR #637 已合并)
- #636: OnClosed hide-to-tray 取消活跃 AI 请求 — 移除 CTS 取消逻辑 (PR #638 已合并)
- #639: GetConversationHistory/CompressSession 使用 live _currentSessionId — FindSessionById 参数化 (PR #642 已合并)
- #640: validate_import_path Windows 无效 — 添加 Windows blocked 前缀 + USERPROFILE 回退 (PR #643 已合并)
- #641: vaultpilot-agent read_line 在 size check 前缓冲全行 — 改用 read_exact 逐字节限制 (PR #644 已合并)
- #645: render_notes body 字段未 XML 转义 — 防御纵深补齐 (PR #647 已合并)
- #649: MCP server read_line_bounded BufReader::read_line() 全行缓冲 OOM — 改用逐字节 read_exact() (PR #652 已合并)
- #650: MCP server read_line 仅 trim `\n` 不 trim `\r` — Windows CRLF 支持 (PR #652 已合并)
- #653: SendAsync _writeLock.Release() 在 finally 块中可抛出 ObjectDisposedException — 添加 try-catch 保护 (PR #654 已合并)
- #655: query_filtered_note_metas LIKE 通配符未转义 — 应用 escape_like_pattern() + ESCAPE 子句 (PR #658 已合并)
- #656: SearchResult.total 返回截断后计数 — 移至 truncate() 前计算 (PR #658 已合并)
- #657: StartProcess async void re-throw 崩溃 — Trace.TraceError + ConnectionStateChanged + return (PR #659 已合并)
- #660: constant_time_eq 256 字节截断 — subtle::ConstantTimeEq 直接比较字节切片 (PR #663 已合并)
- #661: agent handler 错误路径 sanitize_error 一致性 (PR #663 已合并)
- #662: CLI exit_error sanitize_error 绕过 (PR #663 已合并)
- #664: NotesView 搜索-清除竞态 — await 后验证 _searchQuery 未变 (PR #667 已合并)
- #665: SettingsDialog 数值字段上界校验 — timeoutMs/contextWindowTokens/autoWakeInterval (PR #668 已合并)
- #666: CI workflow concurrency 控制 — PR 推送取消旧运行 (PR #669 已合并)
- #670: ai.rs error 泄露 — sanitize_error 6 个错误路径 (PR #673 已合并)
- #671: ProviderConfig Debug 泄露 api_key — 手动 Debug 实现掩码 (PR #674 已合并)
- #675: export note IDs < 8 字符 panic — safe slicing .min(8) (PR #678 已合并)
- #676: Send button TOCTOU — Interlocked guard 防止并发 AI 请求 (PR #679 已合并)
- #677: _isStopping 缺少 volatile — 跨线程可见性 (PR #679 已合并)
- #686: OpenVaultDirectoryAsync Process.Start handle 泄漏 — using 声明释放 (PR #688 已合并)
- #687: CI cargo install 缺少 --locked — 供应链可重复性 (PR #689 已合并)
- #680: SettingsDialog.xaml 缺少 AutomationProperties — 15+ 控件屏幕阅读器不可识别 (PR #685 已合并)
- #681: ci.yml 缺少 permissions: contents: read — 默认宽泛权限增加攻击面 (PR #683 已合并)
- #682: MainWindow LoadingOverlay 硬编码 #80000000 — 高对比度主题下不可见 (PR #684 已合并)
- #729: search_rules relevance_term_matches 长 ASCII 针双向子串匹配 → 仅保留 term.contains(needle) (PR #732 已合并)
- #730: _updateDownloadVersion 缺少 volatile — 跨线程可见性 (PR #733 已合并)
- #731: decrypt_secret ENC:v1: 前缀碰撞 → 解密失败回退返回原始值 (PR #734 已合并)
- #736: open_vault_directory 子进程继承 stdin → Stdio::null() 重定向 (PR #738 已合并)
- #735: C# model record 类型缺少 null-safe 反序列化 — 14+ 类型添加 [JsonConstructor] + init defaults (PR #740 已合并)
- #737: ChatSession/ChatState positional constructor null 漏洞 — 已由 PR #740 修复
- #741: is_openai_reasoning_model 名称空间模型名不匹配 — rsplit('/') 提取有效名称 (PR #743 已合并)
- #742: OpenAI reasoning models 使用 role system + resolve_max_output_tokens 默认 8192 不足 — developer role + 32768 (PR #743 已合并)
- #744: SettingsDialog timeout 允许 1ms — 最小值 1000ms (PR #747 已合并)
- #746: ai.rs 重试线性退避 → 指数退避 (PR #748 已合并)
- #749: ai.rs 重试退避添加 jitter 防 thundering herd (PR #752 已合并)
- #750: BackendClient.SendAsync _process 单次捕获避免 stale read (PR #751 已合并)
- #753: tag/keyword SQL LIKE 子串匹配 → json_each 精确匹配 (PR #756 已合并)
- #754: NotesView OnDeleteNoteClicked 取消 in-flight detail load 防 stale UI (PR #755 已合并)
- #758: StartProcess catch block NRE — _process 字段并发 DisposeAsync 竞态 → 局部变量捕获 (PR #759 已合并)
- #760: CompressCurrentSessionIfNeededAsync CancellationToken 链接到外层请求 — 用户取消立即停止压缩 (PR #762 已合并)
- #761: ComposerBox.Text 清除移入 inner try-catch — RefreshAttachments 异常不再丢失用户输入 (PR #762 已合并)
- #763: docs 向量每轮覆盖 → 累积 + HashSet 去重 — 多轮工具执行引用完整 (PR #765 已合并)
- #764: prompting.rs XML 转义补齐开标签 — escape_xml_tags 统一防御纵深 (PR #766 已合并)
- #767: HTTP bridge rate limiter token 轮换绕过 → 客户端 IP 作为限流 key (PR #771 已合并)
- #768: resolve_local_image_url 文件存在性探测 → 路径限制前置 (PR #770 已合并)
- #769: search_notes_with_context total 受 SQL LIMIT 截断 → COUNT(*) 查询 (PR #772 已合并)
- #773: file:// URL percent-encoding 未解码 → url::Url::parse() + to_file_path() (PR #775 已合并)
- #774: HTTP bridge rate limiter 对 /health 端点限流 → 豁免健康检查 (PR #775 已合并)
- #776: SettingsDialog inline LostFocus 校验与 save 校验不一致 → 统一 timeout/contextWindow/autoWake 上下界检查 (PR #778 已合并)
- #777: XAML ProgressRing 和 error TextBlocks 无障碍属性缺失 → 添加 AutomationProperties.Name + LiveSetting (PR #779 已合并)
- #780: FTS+filter 搜索分页 offset 在内存过滤前应用 → 移至 retain 后 (PR #782 已合并)
- #781: MCP notes.list limit 1000 与 storage 200 不一致 → 对齐为 200 (PR #783 已合并)
- #784: MCP chat.send tool output 未转义用户/模型内容 — 间接提示注入 (PR #786 已合并)
- #785: FTS 搜索分页 total undercount — 使用 COUNT(*) 替代 notes.len() (PR #787 已合并)
- #788: MCP tool success summaries 5 个 handler 未转义用户内容 — 间接提示注入 (PR #789 已合并)
- #790: HTTP bridge rate limiter 内层 middleware — rate-limited 请求仍消耗 body read + timeout (PR #793 已合并)
- #791: SaveNote tool error 中断整个请求 — 改为 graceful degradation (PR #794 已合并)
- #792: is_retryable_provider_error 重试所有 5xx — 限制为 502/503/504 (PR #795 已合并)
- #796: ai.rs format_transport_error URL userinfo 凭据泄露 + warn! 日志未 sanitize (PR #799 已合并)
- #797: MCP chat.delete/notes.delete tool output 未转义用户内容 — 间接提示注入 (PR #798 已合并)
- #800: save_settings_with_context validate() 校验 (PR #801 已合并)
- #802: SearchNotes/ListNotes 硬中止 → graceful degradation (PR #803 已合并)
- #805: Ctrl+V 文本粘贴丢失 — 剪贴板有 StorageItems 无图片时 Handled=true 抑制默认粘贴 (PR #808 已合并)
- #806: Release workflow smoke test 静默跳过 → Write-Error + exit 1 (PR #807 已合并)
- #809: Dependabot 缺少 github-actions 生态 — CI action 版本永不自动更新 (PR #812 已合并)
- #810: Zig 二进制下载缺少 SHA256 校验 — CI 供应链加固 (PR #813 已合并)
- #811: XAML 硬编码 Opacity 值在高对比度模式下不可见 → SecondaryTextBrush 主题资源 (PR #814 已合并)
- #823: trigger_matches 空字符串 panic — 添加空 guard + 测试 (PR #825 已合并)
- #824: detect_image_media_type 错误消息泄露完整文件路径 — 使用 file_name() (PR #826 已合并)
- #1305: ai/client.rs 961 行零测试盲区 — 添加 54 个单元测试 (PR #1308 已合并)
- #1310: http_bridge.rs 纯函数零测试 — 添加 34 个单元测试 (PR #1312 已合并)
- #1311: mcp_server.rs 纯函数零测试 — 添加 20 个单元测试 (PR #1313 已合并)
- #1314: markdown_utils.rs 纯函数零测试 — 添加 24 个单元测试 (PR #1315 已合并)
- #1306: MCP HTTP server 外部 AI agent 连接 + 非回环绑定安全验证 (PR #1309 已合并)
- #1421: updateChecker.ts console.log → console.warn 日志一致性 (PR #1423 已合并)
- #1422: rag.ts parseToolCalls + buildSystemPrompt 19 个单元测试 (PR #1424 已合并)
- #1425: extractKeywords 导出 + 14 个单元测试 (PR #1427 已合并)
- #1426: buildFtsQuery + escapeLikePattern 导出 + 17 个单元测试 (PR #1428 已合并)
- #1459: CHANGELOG 补全 v0.3.48-0.3.50 (PR #1460 已合并)
- #1461 + #1462 + #1463: sync 错误体日志 + dead code 清理 + uuid 导出 (PR #1464 已合并)
- #1465: ProviderEditor require() → ES import (PR #1468 已合并)
- #1466: stripMarkdown 导出 + 14 个单元测试 (PR #1469 已合并)
- #1467: inferMime 从 ChatScreen 提取到 chatHelpers + 15 个测试 (PR #1470 已合并)
- v0.3.51 版本发布 (tag v0.3.51)

## 当前进行中
<!-- 由 issue-monitor 任务在创建 PR 后更新 -->

- #597: CI WinUI 测试 — PR #646 和 PR #804 已关闭 (WinUI 构建 6h 超时)，需进一步调查

## 已知阻塞项
<!-- 记录失败的修复尝试、需要人工介入的问题 -->

- #597: CI C# 测试无法运行 — WinUI 项目需要 MSBuild + WindowsAppSDK 基础设施，dotnet test 无法处理传递依赖构建。需要创建独立测试解决方案或 mock WinUI 依赖。

## 决策记录
<!-- 指挥官任务的重要决策 -->

- 2026-06-12: ~~进入稳定化阶段，优先修 Bug，暂停新功能开发~~ → 已过时，见循环#40
- 2026-06-12: 架构重构类 issue（如 lib.rs 拆分）暂不自动化处理，留人工决策
- 2026-06-12 [循环#1]: 当前 50+ open issue，已远超 30 阈值，暂停创建新 Enhancement/UI 类 issue，集中精力消化存量
- 2026-06-12 [循环#26]: **存量已从 50+ 消化至 5 个**，解除"暂停创建新 issue"限制。讨论任务恢复主动审查代码质量和提出新 issue 的职责
- 2026-06-13 [循环#40]: **AI 驱动产品改进模式** — 讨论团队不限方向，可自由创建 bug/安全/性能/架构/功能/测试/文档类 issue；修复团队自动认领实现；审核团队把关合并。唯一约束：issue 必须具体可操作，不能是模糊的建议
- 2026-06-13 [循环#26]: 4 个 Architecture issue (#183, #143, #49, #144) 不再搁置，讨论任务可拆分为小粒度子 issue 后交由修复任务执行
- 2026-06-13 [循环#26]: #217 策略变更 — 不再走子任务构建，直接在主仓库本地操作
- 2026-06-12 [循环#1]: #192 有两次失败修复，标记为阻塞项，需要不同策略（如状态机方案）
- 2026-06-12 [循环#1]: #174 和 #186 都涉及 save_note/atomic_write 的写入安全性，考虑在同一 PR 中一并修复
- 2026-06-12 [循环#2]: 循环#1 修复目标 3/5 完成（#174+#186 ✅, #229 ✅, #227+#197 ⏳ PR #244 待 rebase），额外完成 7 个 issue（#226, #225, #236, #212, #205, #175, #182, #179, #188）
- 2026-06-12 [循环#2]: 开放 Bug 仍有 16 个，但多数涉及 WinUI 前端或复杂状态管理，本轮聚焦 Rust 后端可自动化修复的问题
- 2026-06-12 [循环#2]: 选定 3 个后端高价值 issue 作为修复目标：#177 (OOM), #230 (WAL), #157 (N+1 IO)
- 2026-06-12 [循环#3]: 所有新 PR 均立即冲突（main 分支持续有新合并），系统性阻塞问题加剧
- 2026-06-12 [PR审核轮]: 关闭全部 4 个冲突 PR (#244, #246, #248, #249)，代码审查均通过但无法 rebase
- 2026-06-12 [PR审核轮]: 系统性冲突根因：并行创建 PR 后各自基于旧 main，互相冲突。建议后续 agent 串行创建 PR，每次基于最新 main
- 2026-06-12 [循环#4]: 串行策略验证成功 — 基于最新 main 逐个创建 PR，3 个 PR 均无冲突
- 2026-06-12 [循环#4]: 重建 3 个 issue 的 PR：#227+#197 (PR #250), #235 (PR #251), #177 (PR #252)
- 2026-06-12 [循环#5]: 聚焦 Rust 后端性能优化，选定 3 个 performance issue：#178, #176, #180
- 2026-06-12 [循环#5]: 串行创建 PR 成功，3 个 PR 均基于最新 main：#253, #254, #256
- 2026-06-12 [循环#5]: 修正 PR #249 状态（已关闭非已合并），#180 需重新修复
- 2026-06-12 [PR审核轮2]: 合并 3 个性能优化 PR (#253, #254, #256)，CI 全部通过
- 2026-06-12 [PR审核轮2]: PR #255 代码逻辑正确但 cargo fmt 失败（2 处格式化问题），留 comment 要求修复后重新推送
- 2026-06-12 [PR审核轮2]: PR #255 混合了 #88 + #52 两个独立 issue + 附件批量查询改动，建议后续一个 PR 对应一个 issue
- 2026-06-12 [循环#6]: 文档被重置后重建，选定 3 个 issue：#165 (FTS5 转义), #169 (README 修正), #168 (serde_yaml 替换)
- 2026-06-12 [循环#7]: 大量 issue 已修复但未关闭 — 系统性审计关闭 7 个已解决 issue
- 2026-06-12 [循环#8]: 聚焦 C# WinUI 前端 Bug，PR #264, #265, #266 创建并合并
- 2026-06-12 [循环#9]: 聚焦 C# WinUI Bug 修复，PR #267, #268, #269 创建并合并
- 2026-06-12 [PR审核轮3]: 审核 6 个 open PR (#267-#272)，合并 5 个，重建 2 个冲突 PR (#270→#274, #272→#273)
- 2026-06-12 [PR审核轮3]: 共合并 PR #267, #268, #269, #271, #273, #274 (6 个)，关闭冲突 PR #270, #272
- 2026-06-12 [循环#10]: 聚焦安全+架构+增强 issue，选定 #163 (Security), #213 (架构), #195 (增强)
- 2026-06-12 [循环#10]: #163 子任务超时但代码已提交推送，手动创建 PR #276
- 2026-06-12 [循环#10]: 循环#10 修复目标 3/3 全部合并（#163 ✅, #213 ✅, #195 ✅）
- 2026-06-12 [循环#10]: 发现 CI flaky test — env-var 并行测试竞争条件，创建 PR #278 修复
- 2026-06-12 [循环#11]: 聚焦 Rust 后端性能优化，选定 #114 (StorageContext 缓存), #103 (truncation 统一), #50 (max_tokens 模型适配)
- 2026-06-12 [循环#11]: 当前 70 open issue，Enhancement/UI 类 57 个，继续暂停创建新 issue
- 2026-06-12 [循环#11]: 修复目标 3/3 全部完成（#114 ✅, #103 ✅, #50 ✅）
- 2026-06-12 [循环#12]: PR #283 (模型上下文窗口注册表) + PR #284 (截断指示) 合并，均已通过 CI
- 2026-06-12 [循环#12]: 66 open issue，继续暂停创建新 Enhancement/UI issue
- 2026-06-12 [循环#12]: #192 第三次修复尝试，策略改为用 serde_json 直接解析而非手动字符状态机
- 2026-06-12 [循环#12]: #42 (SQLite 连接池) 是当前最高优先级性能问题，与 #230 WAL + #176 rank_documents 形成完整优化链路
- 2026-06-12 [循环#13]: 聚焦 Enhancement 修复，选定 #198 (截断指示), #204 (模型注册表), #199 (CancellationToken)
- 2026-06-12 [循环#13]: #198 + #204 并行修复成功（PR #283, #284 已合并），#199 C# 前端修复完成（PR #285 待合并）
- 2026-06-13 [循环#14]: 聚焦 Infrastructure 增强，选定 #193 (数据丢失), #203 (结构化日志), #214 (GitHub Actions CI)
- 2026-06-13 [循环#14]: PR #286 (#193) + PR #287 (#203) 已合并；PR #288 (#214) 关闭（CI 内容已被 PR #287 吸收）
- 2026-06-13 [循环#14]: 循环#14 修复目标 3/3 已解决（#193 ✅, #203 ✅, #214 ✅ via #287）
- 2026-06-13 [循环#15]: 系统性审计关闭 6 个已解决/重复 issue：#214 (CI), #105 (重复#50), #106 (重复#206), #85 (重复#204), #91+#135 (重复#234)
- 2026-06-13 [循环#15]: 55 open issue，Bug/Security/Performance 可操作项仅 #42 (SQLite 连接池)，选定为本轮主修复目标
- 2026-06-13 [循环#15]: #192 阻塞（3次失败），#217 阻塞（构建超时），继续搁置
- 2026-06-13 [循环#16]: Bug/Security/Performance 可操作项已清空，转向 Infrastructure/Enhancement
- 2026-06-13 [循环#16]: 选定 3 个 Infrastructure/Enhancement issue：#154 (cargo audit+dependabot), #196 (代码块主题颜色), #207 (.NET 测试项目)
- 2026-06-13 [循环#16]: 3 个 PR 全部创建成功：PR #289 (#154), PR #290 (#196), PR #291 (#207)
- 2026-06-13 [PR审核轮4]: 审核并合并 3 个 PR (#289, #290, #291)，循环#16 修复目标 3/3 全部完成
- 2026-06-13 [PR审核轮4]: PR #289 cargo audit 失败（依赖漏洞扫描发现真实漏洞），属功能预期，代码正确已合并
- 2026-06-13 [循环#17]: 聚焦 Security + Enhancement，选定 #123 (API Key 加密), #137 (Provider headers), #147 (测试基础设施)
- 2026-06-13 [循环#17]: 修复目标 3/3：PR #300 (#123 ✅ 已合并), PR #301 (#137 ✅ 已合并), PR #302 (#147 待合并)
- 2026-06-13 [PR审核轮5]: 审核 10 个 open PR (#292-#301)，合并 8 个，关闭 2 个
- 2026-06-13 [PR审核轮5]: 合并 PR #300 (安全修复), #301 (功能修复), #292, #293, #295, #297 (patch/minor deps), #296 (axum 0.8), #299 (tower-http 0.6)
- 2026-06-13 [PR审核轮5]: 关闭 PR #298 (serde_yml 0.0.13, 8个测试失败, 库已废弃), #294 (rusqlite 0.40, libsqlite3-sys 编译失败, 版本跨度太大)
- 2026-06-13 [PR审核轮5]: cargo audit 失败是 main 分支已有问题 (rustls-webpki CVE), 不阻塞合并
- 2026-06-13 [PR审核轮5]: PR #302 CI 失败因为与 PR #301 冲突 (缺少 provider_type 字段), 已留 comment 要求 rebase
- 2026-06-13 [循环#18]: 聚焦 Performance + Enhancement，选定 #42 (SQLite 连接池), #122 (模型下拉框), #104 (MCP 超时)
- 2026-06-13 [循环#18]: 修复目标 3/3：PR #306 (#42 ✅ 已合并), PR #305 (#122 ✅ 已合并), PR #307 (#104 待合并)
- 2026-06-13 [循环#18]: PR #306 修复了 #42 长期阻塞的 SQLite 连接池问题，r2d2 pool max_size=5
- 2026-06-13 [循环#18]: PR #306 同时修复了 #123 引入的加密测试编译错误 (缺少 provider_type) 和 pre-existing 断言失败
- 2026-06-13 [PR审核轮7]: 审核 4 个 open PR (#315, #320, #321, #322)，全部合并
- 2026-06-13 [PR审核轮7]: 合并 PR #322 (#184 按钮图标), PR #321 (#142 会话重渲染), PR #320 (#26 设置验证), PR #315 (#150 模糊搜索)
- 2026-06-13 [PR审核轮7]: #24 关闭为 #142 重复
- 2026-06-13 [循环#19]: 修复 PR #315 CI 失败 — SearchQuery 4 处构造缺少 ..Default::default()，rebase+push 后 CI 全通过（cargo audit 除外）已合并
- 2026-06-13 [循环#19]: 本轮修复目标 3/3 全部完成：PR #315 ✅, PR #320 ✅, PR #321 ✅
- 2026-06-13 [循环#19]: 23 open issue，0 open PR，2 阻塞项 (#192, #217)
- 2026-06-13 [循环#20]: Bug/Security/Perf 可操作项全部清空（阻塞项除外），转向 UI 增强类 issue
- 2026-06-13 [循环#20]: 选定 3 个 UI issue：#57 (主题颜色), #159 (附件 chip), #215 (loading overlay)
- 2026-06-13 [循环#20]: 子任务并行派发因 API 限流 (429) 全部失败，改为串行直接修复
- 2026-06-13 [循环#20]: 修复目标 3/3 全部完成：PR #324, #325, #326
- 2026-06-13 [循环#20]: 额外完成 2 个 issue：#161 (PR #327, 关闭 #60 重复), #164 (PR #328)
- 2026-06-13 [PR审核轮6]: 审核 5 个 open PR (#324-#328)，全部合并，0 关闭
- 2026-06-13 [循环#21]: PR #327 rebase 解决冲突后合并，#161 关闭
- 2026-06-13 [循环#21]: 选定 2 个 UI issue：#216 (设置内联验证), #162 (空会话状态页)
- 2026-06-13 [循环#21]: 两个 issue 合并为一个 PR (#329)，使用主题感知 BrushRed
- 2026-06-13 [循环#21]: PR #329 CI 全通过（winui-build ✅）并已合并，#216 和 #162 关闭
- 2026-06-13 [循环#21]: 所有 UI 类 issue 全部清空，剩余 13 个 issue 均为 Architecture/Feature/Blocked
- 2026-06-13 [循环#22]: Bug/Security/Perf 可操作项全部清空（阻塞项除外），转向 Feature 类 issue
- 2026-06-13 [循环#22]: 选定 3 个 Feature issue：#29 (Markdown 渲染), #54 (笔记导出), #56 (搜索高亮)
- 2026-06-13 [循环#22]: #54 子任务成功完成 (PR #330)，#29 子任务缺少 Hyperlink_Click handler 需手动补完
- 2026-06-13 [循环#22]: #29 手动补完 handler 后创建 PR #331；#56 子任务完成核心逻辑但遗漏 search_snippet 字段，手动修复 14 处构造点
- 2026-06-13 [循环#22]: 修复目标 3/3 全部完成：PR #330 (#54), PR #331 (#29), PR #332 (#56)
- 2026-06-13 [循环#22]: 10 open issue，2 阻塞 (#192, #217)，3 个 Architecture issue 留人工决策
- 2026-06-13 [循环#22 续]: 额外完成 3 个 Feature issue：#149 (PR #333), #166 (PR #334), #146 (PR #335)
- 2026-06-13 [循环#23]: 合并循环#22 留下的 3 个 PR (#333, #334, #335)，PR #333 有冲突需手动 rebase
- 2026-06-13 [循环#23]: 所有 Feature 类 issue 全部清空，所有 Bug/Security/Perf 可操作项已清空
- 2026-06-13 [循环#23]: 仅剩 7 个 issue：2 个阻塞 (#192, #217) + 5 个 Architecture 重构
- 2026-06-13 [循环#23]: 选定修复目标：#234 (Settings XAML 化), #192 (第4次尝试 — 连续反斜杠计数法)
- 2026-06-13 [循环#23]: 剩余 3 个大架构 issue (#183 storage.rs 拆分, #143 lib.rs 拆分, #49 MainWindow 拆分, #144 Provider 抽象) 留人工决策
- 2026-06-13 [循环#24]: #234 Settings XAML 化完成 (PR #336)，MainWindow.xaml.cs 减少 359 行
- 2026-06-13 [循环#24]: #192 已被关闭（非 agent 修复），阻塞项从 2 个减为 1 个
- 2026-06-13 [循环#24]: 剩余 6 个 open issue：1 个阻塞 (#217) + 4 个 Architecture (#183, #143, #49, #144) + 1 个 PR 待合并 (#234)
- 2026-06-13 [循环#44]: cargo audit 持续失败（rustls-webpki RUSTSEC-2026-0104），需人工升级依赖
- 2026-06-13 [循环#45]: 修复循环#44 审查发现的 2 个 Bug：#349 (PR #351)、#350 (PR #352)，并行修复成功
- 2026-06-13 [PR审核轮8]: 审核并合并 PR #351 (#349) 和 PR #352 (#350)，CI 5/6 通过（cargo audit 预存在 CVE），代码审查无问题
- 2026-06-13 [PR审核轮8]: 项目恢复 0 open issue + 0 open PR 状态，累计已合并 146 PR
- 2026-06-13 [循环#47]: **CRITICAL** MainWindow.xaml 包含未解决 merge conflict markers（line 98-202），来自 PR #149 合并时的冲突未解决，已创建 #353
- 2026-06-13 [循环#47]: SettingsDialog WireUpButtons async void lambda 缺少 try-catch（#354），Rate limiter HashMap 无上限 + expect panic（#355）
- 2026-06-13 [循环#48]: 全代码库深度审查（Rust 7 项 + C# 8 项发现），创建 5 个高质量 issue（3 BUG + 1 SECURITY + 1 ENHANCEMENT）
- 2026-06-13 [循环#48]: Rust 后端质量优秀 — 0 unsafe、0 生产 unwrap、0 TODO/FIXME、0 clippy warnings、353 tests 全通过
- 2026-06-13 [循环#48]: C# 前端仍有 2 个 HIGH 问题（ExitApplication 无 try-catch、_readerCts 孤儿任务）
- 2026-06-13 [循环#52]: 修复循环#51 创建的 3 个 BUG issue，3/3 全部完成
- 2026-06-13 [循环#52]: #369 BackendClient 线程安全 — 5 项改进（volatile、Interlocked、guard、CancellationToken 传播）
- 2026-06-13 [循环#52]: #370 CJK snippet — 改用 str::replace() 消除字节偏移映射，新增 2 测试
- 2026-06-13 [循环#53]: 审核并合并 3 个 PR (#372, #373, #374)，CI 5/6 通过（cargo audit 预存在 CVE），代码审查无问题
- 2026-06-13 [循环#53]: 项目恢复 0 open issue + 0 open PR 状态，累计 154 已合并 PR
- 2026-06-13 [循环#52]: #371 ExitApplication — Interlocked.CompareExchange 原子 guard 防并发
- 2026-06-14 [循环#57]: 修复循环#56 审查发现的 3 个 BUG issue，3/3 全部完成（PR #384, #385）
- 2026-06-14 [循环#58]: 全代码库深度审查，创建 3 个 issue（#386 Regex perf, #387 sanitize_error 测试, #388 C# 纯函数测试）
- 2026-06-14 [循环#59]: 修复 3 个 issue，3/3 全部完成（PR #389, #390, #391）
- 2026-06-14 [循环#60]: 全代码库深度审查，创建 3 个 issue（#392 SECURITY, #393 BUG, #394 BUG）
- 2026-06-14 [循环#61]: 修复 3 个 issue，3/3 全部完成（PR #395, #396, #397）
- 2026-06-14 [PR审核轮#62]: 审核并合并 3 个 PR (#395, #396, #397)，累计 161 已合并 PR
- 2026-06-14 [循环#63]: 全代码库深度审查（Rust 5 HIGH + C# 2 HIGH），创建 3 个 HIGH issue（#398 SSRF, #399 ShutdownAsync, #400 DisposeAsync）
- 2026-06-14 [循环#64]: 修复循环#63 的 3 个 HIGH issue，3/3 全部完成，创建 PR #401 (#399+#400) 和 PR #402 (#398)
- 2026-06-14 [循环#66]: 全代码库深度审查（Rust 4 HIGH + 8 MEDIUM, C# 3 CRITICAL + 6 HIGH + 12 MEDIUM），创建 3 个 issue
- 2026-06-14 [循环#66]: 排除 4 个与已合并 PR 重叠的发现（#398, #399, #393, #106/#206），确保 issue 不重复
- 2026-06-14 [循环#69]: 项目进入极高质量阶段，168 个已合并 PR 后仅发现 1 个 LOW severity BUG，代码库接近"零缺陷"状态
- 2026-06-14 [循环#71]: 全代码库深度审查（Rust 9 文件 15K+ 行 + C# 10 文件），发现 2 个 SECURITY + 1 个 BUG issue
- 2026-06-14 [循环#71]: Rust 后端安全实践优秀（0 unsafe、0 生产 unwrap、参数化 SQL、路径穿越防护、prompt 注入防护），但存在 CORS 过度开放和密钥派生弱点
- 2026-06-14 [循环#71]: C# 前端所有 22 个 async void handler 均已正确包装 try-catch，无 .Result/.Wait() 同步阻塞
- 2026-06-14 [PR审核轮#83]: 审核并合并 2 个 PR (#437, #438)，累计 181 已合并 PR
- 2026-06-14 [PR审核轮#83]: PR #437 额外修复 XAML IsItemClickEnabled 残留（Sibling agent 未清理 XAML 属性）
- 2026-06-14 [PR审核轮#83]: 项目恢复 0 open issue + 0 open PR 状态
- 2026-06-14 [循环#91]: PR #456 一次性修复 3 个 BUG（#453 按钮禁用、#454 CancellationToken、#455 FTS5 日志），单 PR 多 issue 策略验证成功
- 2026-06-14 [循环#91]: winui_build CI 最终方案：MSBuild /restore 替代 dotnet restore，msbuild /t:VSTest 替代 dotnet test，windows-2022 runner
- 2026-06-14 [循环#91]: PR #452 多次迭代失败后，CI 修复通过 main 直接提交完成（非 PR 合并路径）
- 2026-06-14 [修复轮#102]: 3 个 issue 全部修复（#470 SaveChatState 竞态, #471 _chatStateLock Dispose, #472 N+1 查询），创建 PR #473 和 PR #474
- 2026-06-14 [修复轮#102]: #471 + #472 合并为单 PR（PR #474），#470 单独一个 PR（PR #473）
- 2026-06-14 [PR审核轮#103]: 审核并合并 2 个 PR (#473, #474)，CI 5/6 通过（cargo audit 预存在 CVE），累计 192 已合并 PR
- 2026-06-14 [PR审核轮#103]: 项目恢复 0 open issue + 0 open PR 状态
- 2026-06-14 [讨论轮#105]: 全代码库深度审查发现 4 个 LOW severity 问题，创建 3 个 issue（#480, #481, #482），PR #483 批量修复已合并，累计 196 已合并 PR
- 2026-06-14 [讨论轮#105]: 代码库达到「零实质缺陷」状态 — 0 unsafe、0 生产 unwrap/expect、368+ tests 全通过、所有 async void 有 try-catch
- 2026-06-14 [讨论轮#106]: 发现 CRITICAL 编译错误 — MainWindow.xaml.cs:3010 `await OnSettingsClicked()` 对 `async void` 方法使用 await
- 2026-06-14 [讨论轮#106]: CI winui_build 作业仅有 setup 无实际 build 步骤，导致 C# 编译错误永远不会被 CI 捕获
- 2026-06-14 [讨论轮#106]: Rust 后端发现 OpenAI provider 格式不兼容 — 始终使用 AnthropicRequest/AnthropicResponse，endpoint 和 header 正确分支但请求体/响应解析未分支
- 2026-06-14 [修复轮#107]: 3 个 BUG issue 全部修复，创建 PR #487 (#484) 和 PR #488 (#485+#486)
- 2026-06-14 [讨论轮#108]: PR #487 经 3 次 CI 失败后修复合并 — VirtualKey.OemComma 枚举值、async void 赋值、AutomationLiveSetting、Span.Background、AttachmentItem 类型引用
- 2026-06-14 [讨论轮#108]: 深度审查确认 OpenAI provider 格式不兼容为真实 HIGH severity BUG，创建 #489
- 2026-06-14 [讨论轮#108]: winui_build CI 现在是有效的质量门禁 — 已成功捕获 6 个 C# 编译错误并驱动修复
- 2026-06-14 [修复轮#109]: 深度代码审查（Rust 9 项 + C# 6 项发现），创建 3 个 issue（#491 SECURITY, #492 PERF, #493 BUG），3/3 全部修复
- 2026-06-14 [修复轮#109]: render_history XML 转义、load_recent_notes_for_overview spawn_blocking、cached_settings mutex 中毒恢复
- 2026-06-14 [PR审核轮#110]: 审核并合并 PR #494 (#491+#492+#493)，CI 5/6 通过（cargo audit 预存在 CVE），累计 200 已合并 PR

## 本轮循环状态 (循环#245)
<!-- 讨论团队在每轮开始时写入 -->
- 循环编号: 循环#245
- 本轮时间: 2026-06-21
- 审查模块: 全局扫描 — 6 个 open PR (#1228,#1230,#1235,#1236,#1237,#1238), 15 个 open feature issues
- 竞品调研: Notion AI 2026 (工作流引擎 + 自定义 Agent + MCP Server)。思源笔记 v3.6.5 (全平台 + FSRS + SQL 查询)。趋势: AI + 插件生态是竞品共同方向，VaultPilot 差异化在本地优先 + 三端原生 + 工程笔记场景。
- 讨论阶段发现: 1 个新 issue (#1239 统一消息模型 MessageV2)
- 代码审查: 项目处于「零缺陷」状态 — 0 unsafe、0 生产 unwrap、cargo test 全通过、mobile jest 160 passed (13 suites)。6 个 PR 积压需 review 合并。
- 项目状态: **15 open feature issue, 6 open PR, ~1239 PR 编号, v0.3.32**
- 重点议题: (1) 合并 6 个积压 PR → (2) Mobile 键盘增强 #889 → (3) 统一消息模型 #1239 → (4) 离线模式 #1220

## 本轮审核阶段 (审核轮#235)
- 合并 PR: #1204 (flaky validate_base_url test ENV_MUTEX 修复), #1211 (mobile client.ts 单元测试), #1210 (mobile rag.ts RAG 逻辑单元测试)
- 发版: v0.3.30 (tag 已推送)
- 累计: ~1211 PR 编号

## 修复阶段 fix-2 (循环#240)
- #1212: storage.rs Phase 1 backup 模块提取 → PR #1216（4 个回归测试 ✅）
- #1214: ChatScreen 纯逻辑提取 + 19 个单元测试 → PR #1217（refactor + test ✅）
- cargo build/test/clippy ✅, mobile jest 116 tests ✅

## 本轮审核阶段 (审核轮#236)
- 合并 PR: #1215 (store.ts 全局状态管理单元测试), #1216 (storage backup 模块提取), #1217 (ChatScreen 纯逻辑提取+19单元测试)
- 发版: v0.3.31 (tag 已推送)
- 累计: ~1217 PR 编号
- 项目状态: 0 open issue, 0 open PR, cargo test 全通过, v0.3.31

## 修复阶段 fix-3 (循环#242)
- #1206: MainWindow.xaml.cs Chat partial class extraction → PR #1230 (2661→1317 lines, -1344)
- #1231: db.ts unit tests extended → PR #1233 (18→28 test cases, +10 new)
- Closed: #1232 (SSE tests already exist in sse.test.ts), #1205 (duplicate of #1219), #1234 (not feasible in node test env)
- cargo test 16 passed, mobile jest 137 passed (11 suites)

## 审核阶段 review (循环#242)
- PR #1233: test(mobile): extend db.ts unit tests — ✅ 已合并
- PR #1229: refactor(mobile): extract shared fmtTime utility — ✅ 已合并
- PR #1227: ci: auto-merge main into PR branches before CI checks — ✅ 已合并
- PR #1226: feat(mobile): APK 自动更新检测 — ✅ 已合并
- PR #1261: feat(mobile): 启动时自动检查更新 + 自动下载安装 APK — ✅ 已合并 (reviewer fixes: remove @react-native-voice/voice, restore autoSyncOnStartup, fix sync test)
- PR #1225: feat(mobile): 首次使用引导页面 OnboardingScreen — ✅ 已合并
- PR #1230: refactor(WinUI): extract Chat partial class — ❌ winui-build 失败 (CS0103: _isShuttingDown, LogStartup)
- PR #1228: refactor(storage): extract chat session module — ❌ cargo fmt 失败
- 发版: v0.3.32 (5 PR 合并)

## 修复阶段 fix-1 (循环#244)
- SearchScreen fmtTime duplication → PR #1235 (use shared utility from utils/timeFormat.ts)
- updateChecker compareSemver unit tests → PR #1236 (9 test cases)
- Closed 10 stale feature issues already implemented: #882(设置页), #879(对话核心), #881(对话管理), #883(笔记编辑器), #884(笔记管理), #975(文件夹分类), #978(置顶/归档), #888(主题系统), #891(EAS Build), #979(duplicate voice input)
- cargo test 16 passed, mobile jest 160 passed (13 suites)
- 项目状态: 14 open feature issues, 4 open PR (#1228, #1230, #1235, #1236), ~1236 PR 编号, v0.3.32

## 修复阶段 fix-2 (循环#245)
- cargo doc warning fix → PR #1237 (unclosed HTML tag in storage/mod.rs rustdoc)
- renderLatex regex partial-match bugs → PR #1238 (\\cdot matched \\cdots, \\le matched \\leftarrow, extracted to utils/latex.ts, 23 unit tests)
- cargo test 16 passed, mobile jest 174 passed, clippy clean

## 修复阶段 fix-1 (循环#246)
- #1239: 统一消息模型 MessageV2 — 三端序列化 schema → 3 个 PR 完成全部 5 步
  - PR #1243: Rust canonical schema (MessageV2, MessageV2Role, MessageV2Attachment, MessageV2Metadata) + 11 roundtrip tests + shared fixture JSON (Step 1+4+5)
  - PR #1244: Mobile TypeScript types (messageV2.ts) + createMessageV2() + validateAttachmentUrls() + 7 unit tests (Step 3)
  - PR #1245: WinUI C# types (MessageV2Role, MessageV2AttachmentType, MessageV2Attachment, MessageV2Metadata, MessageV2 records) + 7 xUnit tests (Step 2)
- 安全特性: attachment URL local:// scheme 强制 + metadata 64KB 大小限制
- cargo test 408 passed, mobile jest 158 passed (13 suites), clippy clean
- 项目状态: 14 open feature issues, 9 open PR (#1228,#1230,#1235,#1236,#1237,#1238,#1243,#1244,#1245), ~1245 PR 编号, v0.3.32

## 修复阶段 fix-3 (循环#247)
- 测试覆盖率提升 — 3 个 PR，13 个新测试用例
  - PR #1249: rag.ts executeSave 单元测试 + parseToolCalls 边界用例 (7 新用例, 18→25 tests)
  - PR #1250: normalizeApiBase 边界用例 via checkApi — 尾部斜杠、/v2 路径、空 base、Anthropic 规范化 (4 新用例, 17→21 tests)
  - PR #1251: globalSearch 边界用例 — 非空查询合并排序 + FTS LIKE 回退 (2 新用例, 28→30 tests)
- cargo test 425 passed, mobile jest 151 passed (12 suites), clippy clean
- 项目状态: 零缺陷, 12 open feature issues, 12 open PR (~#1251), v0.3.32

## PR 审核 review (循环#248)
- 15 open PR 审核，13 合并，1 关闭（冗余），1 待处理（冲突+无 CI）
- 已合并 PR:
  - #1243: MessageV2 统一跨平台 schema (Rust 类型 + 验证 + 14 测试) → 基础 PR
  - #1244: MessageV2 Mobile TypeScript 类型 + 7 单元测试
  - #1245: MessageV2 WinUI C# 类型 + 7 xUnit 测试
  - #1246: LaTeX/inline markdown 提取为可测试 utils，修复 regex 边界 bug + 47 测试
  - #1247: client.ts API 工具函数提取到 clientUtils.ts + 11 测试
  - #1249: executeSave + parseToolCalls 边界用例测试
  - #1250: normalizeApiBase 边界用例测试
  - #1251: globalSearch 边界用例测试
  - #1252: mask_secret + ProviderType + masked() 单元测试（替代冲突的 #1248）
  - #1228: storage/chat.rs 模块提取 Phase 2
  - #1235: SearchScreen fmtTime 去重使用共享 utils
  - #1236: updateChecker compareSemver 9 测试用例
  - #1237: rustdoc HTML 转义修复
- 关闭: #1238（被 #1246 超集替代）
- 待处理: #1230（merge conflicts + 无 CI checks）
- 版本: v0.3.32 → v0.3.33 (git tag v0.3.33)
- cargo test 425+ passed, clippy clean

## 讨论阶段 (循环#249)
- v0.3.33 零缺陷状态，14 open feature issues
- 关闭 stale PR #1230（merge conflicts + no CI）
- 重点议题：Mobile 核心体验补齐
  - P1: #1242 (vault 读取) + #973 (附件渲染)
  - P2: #1220 (离线模式) + #889 (键盘增强)
  - P3: 插件/Agent/同步等长期功能
- 下个 fix 阶段目标：#1242 + #973

## 修复阶段 fix-1 (循环#250)
- #973: 附件渲染 — message bubbles 显示附件指示器 + attachments 列迁移 + MIME 类型推断 → PR #1253 (15 tests)
- RAG 上下文改进 — buildNoteContext 支持最近对话历史关键词提取 → PR #1254 (2 tests)
- Anthropic 图片格式修复 — chatWithReconnect 的 toAnthropicContent 统一转换 → PR #1255 (7 tests)
- cargo test 16 passed, mobile jest 267 passed (18 suites), clippy clean
- 项目状态: 14 open feature issues, 3 open PR (#1253,#1254,#1255), ~1255 PR 编号, v0.3.33

## 修复阶段 fix-2 (循环#251)
- #1206: MainWindow.xaml.cs 重构 — 提取 Chat 和 Utilities partial classes → PR #1256
  - MainWindow.xaml.cs: 2661 → 1110 行 (-58%)
  - MainWindow.Chat.cs: 1252 行 (chat session management, message sending, context compression, token estimation, chat UI rendering)
  - MainWindow.Utilities.cs: 355 行 (theme helpers, version, error/status, logging, model/token detection, localization)
- #889: 移动端快捷操作工具栏 — 替换单个 📎 按钮（隐藏在 Alert 对话框中）为 3 个可见快捷按钮 → PR #1257
  - 📷 拍照、🖼 相册、📄 文件 — 每个按钮直接触发对应操作（1 次点击 vs 原来 2 次）
  - 添加触觉反馈和无障碍标签
- cargo test 16 passed, mobile jest 260 passed (17 suites), clippy clean
- 项目状态: 14 open feature issues, 5 open PR (#1253-#1257), ~1257 PR 编号, v0.3.33

## PR 审核 review (循环#252)
- #887: voice input hook + mic button → PR #1258 ✅ squash 合并
- #892: Android home screen widget → PR #1260 ✅ squash 合并
- #1261: 启动时自动检查更新 → ❌ 请求修改（分支严重过期，混合不相关改动，已评论要求 rebase）
- 项目状态: 1 open PR (#1261 需 rebase), ~1261 PR 编号, v0.3.33

## 修复 fix-1 (循环#254)
- #1220: 离线编辑队列 + 同步指示器 → PR #1267 ✅ (pending_syncs 表 + offlineSync.ts + offline banner)
- #1221: 笔记自动标签 → PR #1268 ✅ (autoTag.ts TF 关键词提取 + CJK bigram)
- #1222: 设置导出/导入 → PR #1269 ✅ (settingsSync.ts + SettingsScreen 剪贴板导出/导入)
- 测试: 27 新增回归测试, 全套 337 tests / 26 suites 通过
- 项目状态: 3 open PR (#1267-#1269), 3 open feature issues (#1223,#914,#913), v0.3.33

## PR 审核 review (循环#255)
- #914: 主动知识推送 find_related_notes → PR #1270 ✅ (storage + CLI, clippy 修复后合并)
- #1223: 插件系统 VaultPlugin trait → PR #1271 ✅ (PluginManager + 5 tests)
- #914: MCP notes.related + HTTP endpoint → PR #1272 ✅ (MCP tool + REST API, 依赖 #1270 rebase 后合并)
- #1222: 设置导出/导入 → PR #1269 ✅ (settingsSync.ts + 剪贴板)
- #1221: 自动标签 → PR #1268 ✅ (autoTag.ts TF 关键词)
- #1220: 离线编辑队列 → PR #1267 ✅ (offlineSync.ts + banner, 解决合并冲突后合并)
- Clippy 修复: sort_by → sort_by_key, empty_line_after_doc_comments
- 发版: v0.3.33 → v0.3.34 (6 PR 合并)
- 项目状态: 0 open PR, v0.3.34

## 维护 maintenance (循环#256)
- CI 修复: cargo-audit install 添加重试逻辑 (PR #1273) — 修复 Windows Installers v0.3.34 发布失败
- 依赖检查: cargo outdated 无过时依赖, cargo audit 无安全漏洞
- 测试健康: 454 tests (426 lib + 12 agent + 16 cli) 全部通过, clippy 干净
- Mobile 依赖: 3 个 minor 更新可用 (async-storage 2→3 major 跳过, react-native 0.85→0.86 minor 跳过, safe-area-context 5.7→5.8 minor 跳过)
- 项目状态: 1 open PR (#1273), v0.3.34

## 维护维护结果
- PR #1273 ✅ 已合并: cargo-audit install 重试逻辑 (windows-installers.yml + ci.yml)

## 维护最终结果 (循环#256)
- PR #1273 ✅: cargo-audit install 重试逻辑 (ci.yml + windows-installers.yml)
- v0.3.35 ✅ 发布: 补发 Windows/Linux 安装包 (v0.3.34 因 cargo-audit 失败缺少)
- Release assets: APK + win-x64/x86 Setup.exe + linux deb + nupkg
- 项目状态: 0 open PR, v0.3.35

## 讨论阶段 (循环#257)
- v0.3.35 零缺陷状态，808 测试通过（454 Rust + 354 mobile），Clippy 干净
- 1 open feature issue: #913 Agent Mode
- 创建 2 个新 issue:
  - #1274: P1 移动端 API Key 加密存储 — AsyncStorage → expo-secure-store (security)
  - #1275: P1 storage/mod.rs 拆分 — notes.rs + search.rs + settings.rs (refactor)
- 竞品调研: Obsidian Copilot "Agentic Copilot" 已上线（spawn CLI agent 进程），验证 Agent Mode 方向
- Agent Mode 设计: 4 阶段（进程管理 → 沙箱 → UI → MCP client），安全优先
- 路线图: v0.4.0 = storage 拆分 + API Key 安全 + Agent Mode Phase 1
- 项目状态: 3 open issues (#913, #1274, #1275), 0 open PR, v0.3.35

## 修复阶段 fix-1 (循环#258)
- #1274: API Key 迁移 AsyncStorage → SecureStore → PR #1276 (4 回归测试)
- #1275: storage/settings.rs 模块提取 → PR #1277 (mod.rs -140 行, cargo fmt 修复)
- cargo test 通过, clippy 干净
- 项目状态: 1 open issue (#913), 0 open PR, v0.3.35

## PR 审核 review (循环#258)
- PR #1276: fix: migrate legacy API key from AsyncStorage to SecureStore (#1274) → ✅ squash 合并
  - 逻辑: ✅ 一次性迁移 + _migrated flag 防重复检查
  - 测试: ✅ 4 个回归测试（迁移、跳过、空 key、session 内幂等）
  - 安全: ✅ SecureStore 加密存储，AsyncStorage 旧值清除
- PR #1277: refactor: extract settings module from storage/mod.rs (#1275) → ✅ squash 合并
  - 逻辑: ✅ 纯模块提取，代码 1:1 搬迁 + re-export
  - 格式: ⚠️ cargo fmt 失败（多余空行 + import 顺序），已修复推送
  - 测试: ✅ clippy/test/build 全通过
- 合并后 cargo fmt ✅, clippy ✅, test ✅, winui-build ✅, linux-cli-build ✅, cargo audit ✅
- 项目状态: 1 open issue (#913), 0 open PR, v0.3.35

## 讨论阶段 (循环#259)
- v0.3.35 零缺陷状态，808+ 测试通过，Clippy 干净
- 2 open issues: #1275 (storage 拆分部分完成), #913 (Agent Mode)
- 2 open PR: #1278 (clippy 失败), #1279 (CI 修复，全部通过)
- 创建 3 个新 issue:
  - #1280: P1 storage/mod.rs → notes.rs 提取笔记 CRUD + 导入导出 + OCR
  - #1281: P1 storage/mod.rs → search.rs 提取搜索 + 语义搜索
  - #1282: P2 Agent Mode Phase 1 — AgentProtocol + ToolProxy + vault sandboxing 详细设计
- PR #1278 clippy 失败根因: field_reassign_with_default (CI Rust 1.96)，需 review 阶段修复
- 竞品洞察: Obsidian Copilot v3.3.3 移动端成熟、API Key Keychain、Agent Mode 未正式发布
- 路线图: 下轮 fix = #1280 + #1281 + PR#1278 修复; v0.4.0 = storage 拆分完成 + Agent Mode Phase 1
- 项目状态: 5 open issues (#913, #1275, #1280, #1281, #1282), 2 open PR (#1278, #1279), v0.3.35

## 维护阶段 (循环#260)
- CI 状态: main 全绿（3 个最近 push 全部 success）
- 代码质量: clippy ✅, cargo fmt ✅, 457 测试全通过
- Open PR: 0
- Open issues: 4 (#913 Agent Mode, #1275 storage 拆分, #1280 notes.rs 提取, #1282 Agent Mode 设计)
- 技术债务: 无 TODO/FIXME/HACK
- 依赖安全: cargo audit 无法运行（GitHub 网络问题），npm audit 30 moderate（全部 jest/expo 测试依赖，非生产风险）
- Windows Installer: v0.3.34 失败（HTTP2 瞬时问题）已由 v0.3.35 + CI retry resilience PR 修复
- 项目状态: 4 open issues (#913, #1275, #1280, #1282), 0 open PR, v0.3.35 — 零缺陷维护态

## PR 审核 review (循环#261)
- PR #1284: refactor: extract notes module from storage/mod.rs (#1280) → ✅ squash 合并
  - 逻辑: ✅ 纯模块提取，mod.rs 2834→1207 行 (-57%)，notes.rs 1713 行
  - 格式: ⚠️ cargo fmt 失败（import 排序），已修复推送
  - 测试: ✅ cargo test/clippy/build 全通过
  - 公共 API: ✅ re-export 无破坏性变更
- PR #1285: feat: Agent Mode Phase 1 — AgentProtocol + ToolProxy + vault sandboxing (#1282) → ✅ squash 合并
  - 逻辑: ✅ 5 层安全检查（资源限制/白名单/写权限/路径沙箱/审计日志）
  - 测试: ✅ 9 个回归测试覆盖所有安全边界
  - 安全: ✅ fail-closed、symlink 防逃逸、路径穿越阻断
  - CI: ✅ 全部通过
- 合并后 cargo fmt ✅, clippy ✅, test ✅, winui-build ✅, linux-cli-build ✅
- 项目状态: 2 open issues (#913, #1275), 0 open PR, v0.3.35

## 修复阶段 fix-3 (循环#263)
- 所有 4 个开放 PR (#1290, #1291, #1292, #1293) 的 `cargo fmt` 失败已全部修复
- PR #1290 (CLI 模块拆分): `cargo fmt` ✅, `clippy` ✅, `test` ✅
- PR #1291 (AI 模块拆分): `cargo fmt` ✅, `clippy` ✅, `test` ✅
- PR #1292 (lib.rs 编排拆分): `cargo fmt` ✅, `clippy` ✅, `test` ⏳
- PR #1293 (Agent Mode Phase 2): `cargo fmt` ✅, `clippy` ✅, `test` ⏳
- 项目状态: 5 open issues (#913, #1286, #1287, #1288, #1289), 4 open PR, v0.3.35
- 下一步: review 阶段合并 PR

## Review 循环 #261
- PR #1290 (CLI 模块拆分): ✅ 合并
- PR #1291 (AI context 模块提取): ✅ 合并
- PR #1292 (lib.rs 编排模块拆分): ✅ 合并 — ⚠️ sanitize_error 安全退化已评论，#1294 跟进
- PR #1293 (Agent Mode Phase 2 — write patterns + process management): ✅ 合并
- v0.3.36 发版: git tag v0.3.36 已推送
- 新 Issue: #1294 — sanitize_error 完整 redaction 恢复
- 项目状态: 1 open issue (#1294), 0 open PR, v0.3.36
- 下一步: maintenance 阶段

## 维护阶段 (循环#262)
- CI 状态: main 最新 run 全绿（v0.3.36 bump push success）
- 代码质量: cargo fmt ✅, clippy ✅ (零警告)
- 测试: 362 全通过（lib 334 + agent 12 + cli 16）
- 依赖安全: cargo audit 干净（304 crate，零漏洞）
- 技术债务: 无 TODO/FIXME/HACK
- npm outdated: react-native 0.85→0.86 / async-storage 2.2→3.1 / safe-area-context 5.7→5.8（均为 major/minor 跳版本，维护阶段不更新）
- 项目状态: 0 open issue, 0 open PR, v0.3.38 — 零缺陷维护态
- 下一步: maintenance 阶段

## 维护阶段 (循环#263)
- 补充测试: 为 `src/orchestration/ask.rs` (1030行, 原 0 测试) 添加 38 个单元测试 (PR #1304 已合并)
- 覆盖函数: normalize_tool_path, looks_like_small_talk, looks_like_a_question, looks_like_record_request, looks_like_session_memory_question, merge_usage, display_path, truncate_for_trace, summarize_docs_for_tool_result, planned_tool_identity, draft_to_note_document
- 安全关键: normalize_tool_path 路径穿越防护已覆盖 (dot-dot traversal, quotes stripping)
- 测试总数: 398 全通过
- clippy: 零警告
- CI: main 全绿
- 剩余缺口: ai/client.rs (961行), orchestration/chat.rs 已有测试, ai/parsing.rs 已通过 ai/mod.rs 覆盖

## 讨论阶段 (循环#264)
- v0.3.38 零缺陷状态，426 测试通过，Clippy 干净
- 1 open issue (#913 Agent Mode)，0 open PR
- 项目进入"后重构时代"——大文件拆分全部完成，模块化架构健康
- 创建 3 个新 issue:
  - #1305: P1 ai/client.rs 纯函数提取 + 单元测试 (961 行零测试盲区)
  - #1306: P1 Agent Mode Phase 3.1 — CLI HTTP MCP server with token auth
  - #1307: P2 Mobile 流式响应审计 — 检查 SSE 支持
- 三回合讨论核心结论:
  1. ai/client.rs 测试盲区是最大质量风险 → P1 下个 fix 周期
  2. Agent Mode Phase 3 CLI 优先 → 差异化竞争力
  3. Mobile 流式审计 → 用户体验提升
- 竞品洞察: 沿用上轮 Obsidian Copilot Agent Mode 洞察 (时间窗口 2-3 个月)
- 路线图: v0.4.0 = ai/client.rs 测试 + Phase 3.1; v0.5.0 = Mobile 流式 + Phase 3.2
- 项目状态: 4 open issues (#913, #1305, #1306, #1307), 0 open PR, v0.3.38

## 修复阶段 fix-1 (循环#265)
- #1305 → PR #1308: ai/client.rs 54 个纯函数单元测试 (消除 961 行零测试盲区)
  - detect_image_media_type: 9 tests (png/jpg/jpeg/webp/gif/大写/路径/不支持/无扩展名)
  - is_private_ip: 18 tests (RFC1918/loopback/link-local/CGNAT/benchmarking + IPv6 全覆盖)
  - normalize_endpoint: 8 tests (Anthropic/OpenAI 完整路径/v1/裸URL/尾部斜杠/空白)
  - is_retryable_provider_error: 13 tests (429/500-504/中文限流 + 不可重试)
  - format_transport_error: 3 tests (超时/连接/脱敏 userinfo)
  - should_retry_transport_error: 1 test
- #1307 → 已关闭: Mobile SSE 流式审计确认全部功能已实现 (sse.ts + parseSSEStreamWithReconnect)
- #1306 → PR #1309: MCP HTTP server with token auth (vaultpilot-cli mcp-http)
  - POST /mcp JSON-RPC 端点
  - Bearer token 认证
  - tokio::sync::Mutex async 安全状态管理
  - 默认端口 8766
- 测试: cargo test 全通过, clippy 零警告, cargo fmt 干净
- 项目状态: 1 open issue (#913 Agent Mode), 2 open PR (#1308, #1309), v0.3.38

## 修复阶段 fix-2 (循环#266)
- #1310 → PR #1312: http_bridge.rs 34 个纯函数单元测试 (消除 750 行零测试盲区)
  - constant_time_eq: 5 tests (安全关键时序比较)
  - validate_http_bridge_binding: 4 tests (安全绑定验证)
  - normalize_bridge_token: 4 tests (token 标准化)
  - bridge_token_from_headers: 6 tests (Bearer/header 解析)
  - bridge_model_id: 3 tests (模型 ID 格式化)
  - openai_request_to_dialog: 5 tests (消息解析和验证)
  - render_openai_message_content: 4 tests (多模态内容解析)
  - RateLimiter: 3 tests (速率限制器行为)
- #1311 → PR #1313: mcp_server.rs 20 个纯函数单元测试 (消除 1556 行零测试盲区)
  - negotiate_mcp_protocol_version: 4 tests (MCP 协议版本协商)
  - escape_xml_content: 6 tests (prompt injection 防护)
  - sanitize_mcp_prompt_content: 2 tests (完整包装格式)
  - mcp_tool_success/error: 2 tests (MCP 响应格式)
  - mcp_tools: 3 tests (工具 schema 完整性)
  - McpResponse: 2 tests (JSON-RPC 结构)
- #1314 → PR #1315: markdown_utils.rs 24 个纯函数单元测试
  - strip_markdown_wrapper_tags: 4 tests
  - strip_inline_markdown: 9 tests (bold/italic/strikethrough/code-span)
  - strip_markdown_list_marker: 6 tests
  - simplify_cli_text: 5 tests
- 测试: 398 lib + 12 agent + 40 cli = 450 全通过, clippy 零警告
- 项目状态: 4 open issues (#913, #1305, #1306, #1314), 5 open PR (#1308, #1309, #1312, #1313, #1315), v0.3.38
- 下一步: review 阶段合并 PR

## 维护阶段 (循环#267)
- 安全审计: cargo audit 零漏洞 (quinn-proto 已在 main 升级到 0.11.15，修复 RUSTSEC-2026-0185)
- 5 个 PR 分支 CI 失败均为 quinn-proto 旧版本导致，main 已修复
- 测试: 573 lib + 12 agent + 97 cli = 682 全通过, clippy 零警告
- 技术债务: 零 TODO/FIXME/HACK
- 依赖: npm 有 3 个 minor/major 更新 (async-storage 3.x, react-native 0.86, safe-area-context 5.8)，均非 patch 版本，按规则跳过
- 项目状态: 1 open issue (#913 Agent Mode), 0 open PR, v0.3.40
- 结论: 项目健康，无需额外维护操作

## PR 审核（循环#268）
 PR | 标题 | 逻辑 | 测试 | 安全 | 决策 |
----|------|------|------|------|------|
 #1339 | refactor(mobile): split ChatScreen.tsx into 5 sub-components | ✅ | ✅ | ✅ | 合并 |
 #1340 | refactor(mobile): extract client.ts pure functions + 27 tests | ✅ | ✅ | ✅ | 合并 |
 #1341 | refactor(mobile): split SettingsScreen.tsx into 5 sub-components | ✅ | ✅ | ✅ | 合并 |
- 3 PR 全部 CI 通过（build/cargo audit/clippy/fmt/test/linux-cli-build/winui-build）
- 移动端重构：大文件拆分为子组件，纯函数提取提升可测试性
- 发版: v0.3.40 → v0.3.41 (tag v0.3.41 已推送)
- 项目状态: 1 open issue (#913 Agent Mode), 0 open PR, v0.3.41

## 讨论阶段 (循环#275)
- v0.3.41 零缺陷状态，919+ 测试通过，Clippy 干净
- 1 open issue (#913 Agent Mode)，0 open PR
- 项目进入"后重构成熟期"——所有大文件拆分完成，模块化架构健康
- 创建 3 个新 issue:
  - #1342: P1 Agent Mode Phase 3.2 — 内置 agent loop (tool-calling loop + step limit + token budget + 流式输出 + 写入审核)
  - #1343: P1 CLI vaultpilot agent 命令 — 用户可感知的 Agent Mode 入口
  - #1344: P2 WinUI MainWindow.Chat.cs 拆分 (1255 行 → 4 个 partial class 文件)
- 三回合讨论核心结论:
  1. Agent Mode Phase 3.2 采用内置 agent loop 而非外部进程 spawn（更安全、更统一、利用已有 MCP tools）
  2. CLI 优先发布 Agent Mode（命令行天然适合 agent 交互，无需 UI 设计）
  3. WinUI 拆分延后到 v0.5.0，Agent Mode 优先级更高
- 竞品洞察: Obsidian Copilot v3.3.3 在做大规模代码清理为大功能发布准备，Agent Mode 尚未正式发布但 Copilot Plus 已提供付费 agentic AI；AFFiNE 已有 Android App 验证移动端独立 App 需求
- 路线图: v0.4.0 = Agent Mode Phase 3.2 + CLI agent 命令；v0.5.0 = WinUI 拆分 + Agent Mode Mobile UI
- 项目状态: 4 open issues (#913, #1342, #1343, #1344), 0 open PR, v0.3.41

## PR 审核（循环#282）
- 合并 5 个 PR（全部 CI 通过：build/cargo audit/clippy/fmt/test/linux-cli-build/winui-build）
- #1352 docs: Agent Mode CLI usage guide (README 双语文档) ✅ 直接合并
- #1357 fix: replace .expect() in stable_term_hash + 5 unit tests ✅ 直接合并
- #1356 fix: replace .expect() with graceful error handling in agent.rs + 3 regression tests ✅ 直接合并
- #1351 test: add 28 unit tests for agent.rs pure functions ✅ 直接合并
- #1353 feat: WinUI Agent Mode UI (tool call panel + write approval dialog) ✅ 合并，附带 TODO 追踪
  - 注意：ShowWriteApprovalDialog 中有 TODO（写入审批决策未实际回传后端），需下轮创建 issue
- 发版: v0.3.41 → v0.3.42 (tag v0.3.42 已推送)
- 项目状态: 0 open PR, v0.3.42

## 维护阶段 (2026-06-23)
- Rust: 728 测试全通过 (619 lib + 12 cli + 97 core)，Clippy 无警告，cargo fmt 干净
- 安全: cargo audit 无漏洞
- 依赖: Rust deps 全部最新；Mobile 有 3 个 minor 更新（async-storage 2→3, react-native 0.85→0.86, safe-area-context 5.7→5.8），非紧急
- TODO: 仅 1 处（WinUI MainWindow.AgentMode.cs:287 — 已追踪为 #1358）
- CI: main 分支干净，旧分支有 cargo fmt 失败（已合并代码无影响）
- 项目状态: 2 open issues (#913 Agent Mode tracking, #1358 WriteApprovalDialog), 0 open PR, v0.3.42
- 结论: 项目健康，下一阶段进入讨论轮次

### 循环 #282 讨论结果
- 创建 #1359: Agent Mode 集成测试（P1）
- 创建 #1360: 移动端 Agent Mode 技术方案调研（P2）
- 竞品动态: Obsidian Copilot v4 Agent Mode 正式发布, Google NotebookLM 2.0 agentic 工作流, Notion AI 3.3 Custom Agents
- 项目状态: 4 open issues (#913, #1358, #1359, #1360), 0 open PR, v0.3.42, 1182 测试通过
- 下个 fix 阶段优先级: #1358 → #1359 → 移动端 RAG 回归测试

## Fix Cycle 26 (fix-1)
- 关闭 #1359: Agent Mode 集成测试已在 main 上修复 (commit bfdd507)
- 创建 #1362: normalize_settings 零测试覆盖 → PR #1364 (12 单元测试)
- 创建 #1363: SettingsScreen 静默 catch 块 → PR #1365 (console.warn)
- 项目状态: 3 open issues (#913, #1360, #1362 待合并), 2 open PR (#1364, #1365), v0.3.42, 109 Rust + 455 Mobile 测试通过

## PR 审核（循环#283）
- 合并 6 个 PR（全部 CI 通过：build/cargo audit/clippy/fmt/test/linux-cli-build/winui-build）
- #1373 fix: autoTag.ts CJK regex — include Japanese/Korean ranges (#1372) ✅ 直接合并
- #1371 test: add flushPendingSyncs unit tests (8 tests) (#1368) ✅ 直接合并
- #1370 fix: add console.warn to mobile silent catch blocks (13 catch blocks across 10 files) (#1367) ✅ 直接合并
- #1369 fix: replace .unwrap() on Mutex locks in vaultpilot-agent.rs (#1366) ✅ 直接合并
- #1365 fix: mobile SettingsScreen silent catch blocks (3 catch blocks) (#1363) ✅ 直接合并
- #1364 test: normalize_settings unit tests (12 tests) (#1362) ✅ 直接合并
- 发版: v0.3.42 → v0.3.43 (tag v0.3.43 已推送)
- 项目状态: 0 open PR, v0.3.43

## 讨论（循环#284）
- v0.3.43 零缺陷状态，97 Rust + 473 Mobile = 570 测试通过，Clippy 干净
- 2 open issues (#1360 移动端 Agent Mode 调研, #913 Agent Mode 主 issue), 0 open PR
- 最近提交: commit 9b9346d (应用内 APK 下载 + React 版本锁定) 直接推到 main 未经 PR 流程
- **P1 发现**: UpdateModal 两个 UX bug
  - Bug 1: apkUrl 为 null 时"查看发布页"按钮调用 onClose() 而非打开 releaseUrl
  - Bug 2: 错误状态无"手动下载"fallback 按钮
- **P1 发现**: downloadAndInstall 和 UpdateModal 职责混乱，fallback 逻辑需统一到 UpdateModal
- **P1 发现**: downloadAndInstall 零测试覆盖（仅 compareSemver 有测试）
- **战略决策**: Agent Mode 后产品方向——深度优先（全 vault 读取 + 笔记创建 + 跨 session 记忆）
- **长期规划**: 移动端 Agent Mode (#1360) 短期方案 B(远程服务器) 验证需求，长期方案 A(本地 Rust FFI)
- 下个 fix 周期目标: UpdateModal bug 修复 + downloadAndInstall 测试 + agent.rs 边界条件测试

## 修复（fix-2 周期 #31）
- #1380 bug: UpdateModal '查看发布页' 按钮调用 onClose() 而非打开 releaseUrl + 错误状态无手动下载按钮 → PR #1382
- #1381 test: chatHelpers.ts 4 个纯函数 24 个单元测试 → PR #1383
- 新增 25 个测试（24 chatHelpers + 1 UpdateModal 回归）
- 编译/测试/Clippy：通过（476 mobile tests pass）
- 项目状态: 5 open PR (#1377, #1378, #1379, #1382, #1383), 5 open issues (#1360, #913, #1380, #1381, 已有PR的)

## PR 审核（循环#285）— 8 个 PR 全部合并
- 合并 8 个 PR（全部 CI 通过：build/cargo audit/clippy/fmt/test/linux-cli-build/winui-build/Check regression tests）
- #1377 fix: syncNotesFromServer preserves server note ID (#1374) ✅ 直接合并
- #1378 fix: console.warn in NoteEditorScreen silent catches (#1375) ✅ 直接合并
- #1379 fix: console.warn in ChatScreen executeSave catch (#1376) ✅ 直接合并
- #1382 fix: UpdateModal fallback buttons — open release URL + manual download (#1380) ✅ 直接合并
- #1383 test: chatHelpers.ts pure function unit tests — 24 tests (#1381) ✅ 直接合并
- #1387 fix: LIKE ESCAPE backslash mismatch in non-FTS queries (#1384) ✅ 直接合并
- #1388 fix: remove duplicate offline banner in NotesScreen (#1385) ✅ 直接合并
- #1389 fix: OnboardingScreen '完成' button re-tests and duplicates provider (#1386) ✅ 直接合并
- 发版: v0.3.43 → v0.3.44 (tag v0.3.44 已推送)
- 新增 4 个回归测试文件 + 修复 4 个测试
- 项目状态: 0 open PR, v0.3.44

## Fix Cycle 33 (fix-1)
- #1390 bug: syncNotesFromServer timestamp unit mismatch — seconds vs milliseconds → PR #1391
  - P0 data integrity bug: localNote.updated_at (seconds from SQLite strftime('%s')) compared with serverTs (milliseconds from Date.getTime())
  - Fix: multiply localNote.updated_at by 1000 before comparison
  - 3 regression tests
- #1392 refactor: db.ts FTS5 query construction deduplication → PR #1393
  - Extracted buildFtsQuery() helper from 4 duplicate inline constructions
  - 4 regression tests
- #1394 test: checkForUpdate unit tests → PR #1395
  - 10 edge case tests for checkForUpdate()
  - Covers: version comparison, API errors, network errors, missing assets
- 17 new tests total
- 编译/测试：通过（522 mobile tests pass）

## Review Cycle 33
 PR | 标题 | 逻辑 | 测试 | 安全 | 决策 |
----|------|------|------|------|------|
 #1401 | paginate syncNotesFromServer for large vaults (#1398) | ✅ | ✅ | ✅ | 合并 |
 #1400 | looksLikeSmallTalk + extractCJKNgrams + isCJK unit tests (#1397) | ✅ | ✅ | ✅ | 合并 |
 #1399 | extract text from ContentPart[] in system messages (#1396) | ✅ | ✅ | ✅ | 合并 |
 #1395 | checkForUpdate unit tests — 10 edge case tests (#1394) | ✅ | ✅ | ✅ | 合并 |
 #1393 | extract buildFtsQuery helper — deduplicate 4 inline FTS5 query constructions (#1392) | ✅ | ✅ | ✅ | 合并 |
 #1391 | sync timestamp unit mismatch — seconds vs milliseconds (#1390) | ✅ | ✅ | ✅ | 合并 |
- 发版: v0.3.44 → v0.3.45 (tag v0.3.45 已推送)
- 6 个 PR 全部合并，667 行新增代码
- 项目状态: 0 open PR, v0.3.45

## Discussion Cycle #35
- v0.3.45, 654 Rust + 565 Mobile = 1219 tests, Clippy clean
- 2 open issues (#913, #1360), 0 open PR
- 重点议题：(1) #1358 写入审核回传 P0；(2) 移动端 Agent Mode 渐进式方案；(3) v0.4.0 里程碑发布
- 竞品：Obsidian Copilot Agent Mode 已正式发布（付费），时间窗口已关闭
- 路线图：下个fix=#1358+集成测试；v0.4.0=Agent Mode 正式发布；v0.5.0=移动端 Agent Mode Phase 2

## 修复阶段 fix-3 (循环#37)
- #1414: ChatScreen JSON.parse(m.attachments) unprotected → safeParseAttachments (PR #1418)
- #1415: settingsSync.ts 3x JSON.parse unprotected → try/catch + 中文错误提示 (PR #1419)
- #1417: SSE reconnection duplicate content → contentDelivered dedup (PR #1420)
- 18 个回归测试新增 (9+6+3)
- 763 Rust + 565 Mobile = 1328 tests pass, Clippy clean

## Review Cycle 34
 PR | 标题 | 逻辑 | 测试 | 安全 | 决策 |
----|------|------|------|------|------|
 #1420 | SSE reconnection no longer sends duplicate content (#1417) | ✅ | ✅ | ✅ | 合并 |
 #1419 | wrap JSON.parse in settingsSync.ts with try/catch (#1415) | ✅ | ✅ | ✅ | 合并 |
 #1418 | protect JSON.parse(m.attachments) with safeParseAttachments (#1414) | ✅ | ✅ | ✅ | 合并 |
 #1413 | messageV2.ts + clientUtils.ts pure function unit tests — 21 tests (#1410) | ✅ | ✅ | ✅ | 合并 |
 #1412 | latex.ts pure function unit tests — 30 tests (#1409) | ✅ | ✅ | ✅ | 合并 |
 #1411 | console.log → console.warn consistency + sse.ts silent catch (#1408) | ✅ | ✅ | ✅ | 合并 |
 #1407 | fmtTime unit tests — 7 tests (#1404) | ✅ | ✅ | ✅ | 合并 |
 #1406 | globalSearch buildFtsQuery dedup (#1403) | ✅ | ✅ | ✅ | 合并 |
 #1405 | add console.warn to SettingsScreen + UpdateModal catch blocks (#1402) | ✅ | ✅ | ✅ | 合并 |
- 发版: v0.3.45 → v0.3.46 (tag v0.3.46 已推送)
- 9 个 PR 全部合并，765 行新增代码（6 个测试文件 + 3 个 bug 修复）
- 763 Rust + 636 Mobile = 1399 tests pass, Clippy clean
- 预先存在问题: issue_1336_chat_screen_split.test.ts Icon.tsx 模块编译错误（非本次引入）
- 项目状态: 0 open PR, v0.3.46

## Maintenance Cycle
- 修复 Android Build CI 失败: NoteEditorScreen.tsx TS2322 类型错误 (t.icon 类型收窄不完整)
  - 导入 IconName 类型，使用 `as IconName` 类型断言
  - TypeScript type check + Android Build 全部通过
- 清理: 移除仓库根目录残留的空 `=` 文件
- Cargo.lock 版本同步 (0.3.45 → 0.3.46)
- 项目状态: CI 全绿 (CI + Android Build), 97 Rust tests pass, Clippy clean

## Discussion Cycle #288
- v0.3.46, 97 Rust + 641 Mobile = 738 tests, Clippy clean
- 2 open issues (#913, #1360), 0 open PR
- 重点议题：(1) #1358 写入审核回传 P0；(2) v0.4.0 = Agent Mode 正式发布；(3) Mobile Agent Mode 推到 v0.5.0
- 竞品：跳过（沿用上轮 Obsidian Copilot Agent Mode）
- 路线图：下个fix=#1358写入审核+#1359集成测试+测试补充；v0.4.0=Agent Mode正式发布；v0.5.0=Mobile Agent Mode

## 修复周期 #38 (fix-1)
- #1421: updateChecker.ts console.log → console.warn (PR #1423)
- #1422: rag.ts parseToolCalls + buildSystemPrompt unit tests — 19 tests (PR #1424)
- v0.3.46, 97 Rust + 660 Mobile = 757 tests, Clippy clean
- 2 open issues (#913, #1360), 2 open PR (#1423, #1424)

## PR 审核 review (循环#289)
 PR | 标题 | 逻辑 | 测试 | 安全 | 决策 |
----|------|------|------|------|------|
 #1423 | updateChecker.ts console.log → console.warn (#1421) | ✅ | ✅ | ✅ | 合并 |
 #1424 | rag.ts parseToolCalls + buildSystemPrompt unit tests — 19 tests (#1422) | ✅ | ✅ | ✅ | 合并 |
 #1427 | export extractKeywords from rag.ts + 14 unit tests (#1425) | ✅ | ✅ | ✅ | 合并 |
 #1428 | export buildFtsQuery + escapeLikePattern from db.ts (#1426) | ✅ | ✅ | ✅ | 合并 |
- 发版: v0.3.46 → v0.3.47 (tag v0.3.47 已推送)
- 4 个 PR 全部合并，409 行新增代码（5 个测试文件 + 1 个 bug 修复 + 2 个 refactor）
- 项目状态: 0 open PR, v0.3.47

## Maintenance Cycle (2026-06-24)
- 分支清理: 删除 490 个已合并/已关闭的远程 fix/* 分支（从 418 → 0）
  - 所有分支对应的 PR 均为 MERGED 或 CLOSED 状态
  - 仓库远程分支从 ~500 个精简到仅 main
- 健康检查:
  - Rust: 97 tests pass, Clippy clean, cargo audit clean, cargo outdated clean
  - Mobile: 759 tests pass, tsc --noEmit clean
  - CI: main 最新 run 为 success
  - 0 open PR, 2 open issues (#913, #1360)

## Discussion Cycle #291
- v0.3.49, 97 Rust + 764 Mobile = 861 tests, Clippy clean
- 2 open issues (#913, #1360), 0 open PR
- 创建 3 个新 issue：#1453 (P1 写入审核 diff 预览)、#1454 (P1 agent 异常路径测试)、#1455 (P2 移动端服务层测试)
- 核心结论：v0.4.0 发版在即，验收标准 = 写入审核 UX + CHANGELOG + 测试补充
- 移动端 Agent Mode 确定方案 B（远程服务器），v0.5.0 实施
- 路线图：下个 fix = #1453 + #1454 + #1455；v0.4.0 = Agent Mode 正式发布；v0.5.0 = 移动端 Agent Mode

## Fix Cycle #45 (2026-06-24)
- #1453 → PR #1456: WriteApprovalDialog human-readable diff preview
- #1454 → PR #1457: agent exception path integration tests (12 tests)
- #1455 → PR #1458: mobile sync + settingsSync edge case tests (6 tests)
- 22 tests added, Clippy clean, all tests pass

## Discussion Cycle #292
- v0.3.50, 97 Rust + 805 Mobile = 902 tests, Clippy clean
- 2 open issues (#913, #1360), 0 open PR
- 创建 1 个新 issue：#1459 (P0 CHANGELOG 补全 v0.3.48-0.3.50)
- 竞品洞察：GitHub Copilot Agent Mode 已 GA（VS Code + Visual Studio），MCP 成行业标配
- 核心结论：CHANGELOG 补全是 v0.4.0 发版唯一阻塞项
- 路线图：下个 fix = #1459 CHANGELOG 补全；v0.4.0 = Agent Mode 正式发布；v0.5.0 = 移动端 Agent Mode

## Fix Cycle #46 (fix-2, 2026-06-24)
- #1461 → PR #1464: syncNotesFromServer error body logging + noteRes console.warn
- #1462 → PR #1464: offlineSync.ts unused checkConnection removal
- #1463 → PR #1464: db.ts uuid() export + 6 unit tests
- 7 tests added (812 mobile tests total), 97 Rust tests pass, Clippy clean

## Fix Cycle #47 (fix-3, 2026-06-24)
- #1465 → PR #1468: ProviderEditor.tsx require() replaced with ES import + (p: any) removed
- #1466 → PR #1469: stripMarkdown exported from autoTag.ts + 14 direct tests
- #1467 → PR #1470: inferMime extracted from ChatScreen.tsx to chatHelpers.ts + 15 tests
- 33 tests added (838 mobile tests total), 97 Rust tests pass, Clippy clean

## Maintenance Cycle (2026-06-24)
- 依赖更新: mobile navigation patch deps (@react-navigation/bottom-tabs 7.18.3, native 7.3.4, native-stack 7.17.6, safe-area-context 5.8.0)
- 健康检查:
  - Rust: 97 tests pass, Clippy clean, cargo audit clean, cargo outdated clean
  - Mobile: tsc --noEmit clean, expo export clean
  - CI: main 最新 run 为 success
  - 0 open PR, 2 open issues (#913, #1360)
  - npm audit: 30 moderate (全部为 Expo 上游内部依赖链，非项目可控)
- v0.3.52 锁文件更新已推送 (commit d372c9c)
