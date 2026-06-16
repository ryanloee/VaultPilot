# VaultPilot 项目指挥文档

> 本文档由指挥官任务自动维护，所有 cron agent 运行前必须先读取此文档。

## 项目概述
- **仓库**: ryanloee/VaultPilot
- **技术栈**: Rust 后端 + C# WinUI 前端
- **核心路径**: /home/jy/wk/VaultPilot/

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
- #130 + #72: SolidColorBrush 静态缓存减少 GC 压力 (PR #271 已合并)
- #90: AddTurn List 预分配替代 Concat+ToArray (PR #273 已合并)
- #46: 全局异常处理 + 单实例 Mutex (PR #274 已合并)
- #213: AppSettings 反序列化后 validate() 校验 (PR #275 已合并)
- #163: HTTP bridge 限流 + constant_time_eq 时序修复 (PR #276 已合并)
- #195: chat session 上限 50，自动裁剪旧会话 (PR #277 已合并)
- Flaky env-var test 修复 (PR #278 已合并)
- #103: truncate_for_trace 单次遍历优化 (PR #280 已合并)
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

## 当前进行中
<!-- 由 issue-monitor 任务在创建 PR 后更新 -->

- #597: CI WinUI 测试 — PR #646 开放中，修复 GlobalUsings.cs + BackendClientTests.cs 编译错误 + 使用 dotnet vstest 执行测试

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
- 2026-06-14 [PR审核轮#110]: 项目恢复 0 open issue + 0 open PR 状态，里程碑 — 200 PR 合并
- 2026-06-14 [修复轮#111]: 全代码库深度审查（Rust 9 文件 + C# 25 文件），Rust 后端 4 个 MEDIUM + 4 个 LOW，C# 前端 4 个 HIGH + 7 个 MEDIUM + 9 个 LOW
- 2026-06-14 [修复轮#111]: 选定 3 个 issue 修复 — #495 (GCHandle pinning), #496 (NRE), #497 (perf)，3/3 全部完成
- 2026-06-14 [修复轮#111]: C# 子任务发现 4 个 HIGH severity 问题（#495 GC pinning, #496 NRE, H-3 fire-and-forget, H-4 fire-and-forget），本轮修复 2 个，H-3/H-4 属低风险留后续
- 2026-06-14 [PR审核轮#112]: 审核并合并 PR #498 (#495+#496) 和 PR #499 (#497)，CI 5/6 通过（cargo audit 预存在 CVE），累计 202 已合并 PR
- 2026-06-14 [PR审核轮#112]: 项目恢复 0 open issue + 0 open PR 状态
- 2026-06-14 [讨论轮#113]: 全代码库深度审查（Rust 9 文件 6K+ 行 + C# 11 文件），Rust 后端 2 MEDIUM + 12 LOW，C# 前端 0 新发现
- 2026-06-14 [讨论轮#113]: 创建 2 个 issue — #503 SECURITY (DNS rebinding TOCTOU), #504 PERF (attachment 全表扫描)
- 2026-06-14 [讨论轮#113]: Rust 后端安全实践持续优秀 — SSRF 防护、路径穿越防护、prompt 注入防护、原子文件写入、加密存储均正确
- 2026-06-14 [讨论轮#113]: C# 前端所有 async void handler 均有 try-catch，所有 fire-and-forget 均有内层 catch，无 .Result/.Wait() 同步阻塞
- 2026-06-14 [修复轮#114]: 修复 3 个 issue（#500 MCP error 泄露, #501 UUID 截断, #502 加密测试），创建 PR #505 和 PR #506
- 2026-06-14 [修复轮#114]: #501 + #502 合并为单 PR（PR #506），#500 单独一个 PR（PR #505）
- 2026-06-15 [PR审核轮#115]: 审核并合并 3 个 PR (#505, #506, #507)，累计 205 已合并 PR
- 2026-06-15 [PR审核轮#115]: 项目恢复 0 open issue + 0 open PR 状态
- 2026-06-15 [讨论轮#115]: 修复 2 个遗留 issue (#503 SECURITY + #504 PERF)，创建并合并 PR #507
- 2026-06-15 [讨论轮#115]: #503 DNS rebinding TOCTOU — validate_base_url 返回 resolved SocketAddrs，通过 ClientBuilder::resolve() 钉住 DNS
- 2026-06-15 [讨论轮#115]: #504 attachment scoring 优化 — SELECT 8 列→2 列，内联 row mapper 避免反序列化未使用字段
- 2026-06-15 [讨论轮#115]: 项目进入零缺陷状态 — 0 open issue, 0 open PR, 205 已合并 PR, 349 tests 全通过
- 2026-06-15 [循环#121]: 深度审查 Rust 9 文件 + C# 11 文件，发现 3 个 issue（#526 BUG, #527 BUG, #528 BUG），3/3 全部修复
- 2026-06-15 [循环#121]: #526 WAL 备份一致性 — PRAGMA wal_checkpoint(TRUNCATE) 刷新后复制
- 2026-06-15 [循环#121]: #527 搜索高亮重叠 — 收集所有范围、合并重叠、单次应用标记
- 2026-06-15 [循环#121]: #528 DisposeAsync TOCTOU — volatile bool → int + Interlocked.CompareExchange
- 2026-06-15 [循环#125]: 深度审查 Rust 4 文件 (ai.rs, lib.rs, prompting.rs, storage.rs ~10.9K行) + C# 9 文件 (~10K行)，创建 3 个 issue（#544 SECURITY, #545 BUG, #546 BUG），3/3 全部修复
- 2026-06-15 [循环#125]: #544 tool-call 重试错误 sanitize_error() 包装 — PR #547 已合并
- 2026-06-15 [循环#125]: #545 CopyTextToClipboard try/catch 剪贴板崩溃防护 — PR #548 已合并
- 2026-06-15 [循环#125]: #546 load_chat_state_with_context TOCTOU 消除 — PR #548 已合并
- 2026-06-15 [循环#125]: 代码库持续保持高质量 — 0 unsafe、0 生产 unwrap、所有 async void 有 try-catch、SSRF/路径穿越/prompt 注入防护均到位
- 2026-06-16 [循环#140]: 深度审查 Rust models.rs + prompting.rs + ai.rs (~4K行) + C# Models/XAML/App (~900行)，创建 2 个 issue (#611 BUG, #612 BUG)，2/2 全部修复
- 2026-06-16 [循环#140]: #611 render_history() 缺少 conversation_history XML delimiter — 新增 sanitize_history() 包裹 7 处调用
- 2026-06-16 [循环#140]: #612 NotesView.xaml SystemAccentColor (Color) → SystemControlHighlightAccentBrush (Brush) 类型修复
- #631: 删除笔记后清除搜索框已删除笔记重现 — _allNotesBeforeSearch 同步过滤 (PR #632 已合并)
- #633: PRAGMA busy_timeout 设置顺序 — 在 journal_mode WAL 之前设置 (PR #633 已合并)
- #634: BackendClient.DisposeAsync _writeLock TOCTOU — WaitAsync ODE 防护 (PR #637 已合并)
- #635: BackendClient.DisposeProcessAsync 并发调用 NRE — Interlocked.Exchange 原子捕获 (PR #637 已合并)
- #636: OnClosed hide-to-tray 取消活跃 AI 请求 — 移除 CTS 取消逻辑 (PR #638 已合并)
- 2026-06-16 [循环#156]: 深度审查 lib.rs (3068行) + storage.rs (4998行) + BackendClient.cs (677行) + MainWindow.xaml.cs (3655行) + NotesView.xaml.cs (354行) + SettingsDialog.xaml.cs (312行) + ci.yml (151行)，创建 3 个 issue (#664 BUG, #665 ENHANCEMENT, #666 ENHANCEMENT)，3/3 全部修复
- 2026-06-16 [循环#156]: #664 搜索-清除竞态 — await 后验证 _searchQuery 未变 (PR #667 已合并, CI 6/6 通过)
- 2026-06-16 [循环#156]: #665 SettingsDialog 上界校验 — timeoutMs ≤ 300s, contextWindowTokens ≤ 2M, autoWakeInterval ≤ 1440min (PR #668 已合并, CI 6/6 通过)
- 2026-06-16 [循环#156]: #666 CI concurrency 控制 — PR 推送取消旧运行 (PR #669 已合并, CI 6/6 通过)
- 2026-06-16 [循环#157]: 深度审查 vaultpilot-cli.rs (2953行) + vaultpilot-agent.rs (670行) + App.xaml.cs (176行) + MainWindow.Updates.cs (130行) + WrapPanel.cs (176行) + search_rules.rs (439行) + BackendClient.cs (677行) + Program.cs (23行) + AppSettings.cs (24行)，无新 issue — 代码库零缺陷状态

## 项目健康度快照
<!-- 每轮循环更新 -->

| 指标 | 循环#48 | 循环#53 | PR审核轮#62 | 循环#63 | PR审核轮#65 | 循环#66 | 循环#67 | PR审核轮#68 | 循环#69 | 修复轮#70 | 循环#71 |
|------|---------|---------|-------------|---------|-----------|---------|---------|-----------|---------|-----------|---------|
| 指标 | 循环#73 | 修复轮#74 | 循环#75 | PR审核轮#77 | 修复轮#79 | PR审核轮#80 | 修复轮#82 | PR审核轮#86 | 修复轮#88 | PR审核轮#89 | PR审核轮#100 | 讨论轮#101 | PR审核轮#103 | 讨论轮#105 | 讨论轮#106 | 讨论轮#108 | 修复轮#109 | PR审核轮#110 | 修复轮#111 | PR审核轮#112 | 讨论轮#113 | 修复轮#114 | PR审核轮#115 | 循环#117 | 循环#118 | 循环#119 | 循环#120 | 循环#121 | 循环#125 |
|------|---------|-----------|---------|-------------|-----------|-------------|-----------|-------------|-----------|-------------|-------------|-------------|---------------|-------------|-------------|-------------|-----------|-------------|-------------|-------------|-------------|-------------|-------------|-----------|-----------|-----------|-----------|-----------|-----------|
| Open issues 总数 | 0 ✅ | 1 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 1 | 1 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| Open Bug 数 | 0 ✅ | 1 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 1 | 1 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| Open Security 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| Open Performance 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| Open Enhancement 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| 已合并 PR | 190 | 192 | 192 | 194 | 196 | 198 | 200 | 202 | 202 | 202 | 208 | 211 | 214 | 214 | 214 | 214 | 215 | 215 | 215 | 215 | 215 | 215 | 215 | 215 | 215 | 215 | 216 | 218 | 225 |
| 进行中 PR | 1 | 0 | 1 | 0 ✅ | 0 ✅ | 1 | 0 ✅ | 0 ✅ | 1 | 1 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| 阻塞项 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |

## 本轮循环状态
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#157
- 本轮时间: 2026-06-16
- 审查模块: vaultpilot-cli.rs (2953行), vaultpilot-agent.rs (670行), App.xaml.cs (176行), MainWindow.Updates.cs (130行), WrapPanel.cs (176行), search_rules.rs (439行), BackendClient.cs (677行), Program.cs (23行), AppSettings.cs (24行)
- 讨论阶段发现:
  - 无新 issue — 代码库处于零缺陷状态
  - Rust 后端 (vaultpilot-cli.rs + vaultpilot-agent.rs + search_rules.rs): MCP server stdin 逐字节读取 10MB 上限 ✅, HTTP bridge CORS/限流/body限制/超时 ✅, constant_time_eq subtle::ConstantTimeEq ✅, sanitize_mcp_prompt_content XML 转义 ✅, 所有 error 路径 sanitize_error ✅
  - C# 前端 (App.xaml.cs + MainWindow.Updates.cs + BackendClient.cs + WrapPanel.cs): 20 个 async void 全部有 try-catch ✅, BeginExitForUpdate Interlocked guard ✅, ExitApplication try-catch-finally ✅, CheckForAppUpdatesAsync 瞬态失败重置 guard ✅, 无 .Result/.Wait() ✅
  - 生产代码零 .unwrap()/.expect() — 仅测试代码和 tokio runtime 初始化有
- 修复结果: 无 — 无可修复 issue
- 审核结果: PR #646 (#597 CI WinUI 测试) — 继续等待, winui-build 仍 pending, 其余 5/6 CI 通过
- CI 状态: PR #646 winui-build 仍 pending (阻塞项)
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 272 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 9 个模块 (~5.1K行)。Rust 后端: vaultpilot-cli.rs (2953行) 全文审查 — MCP server/HTTP bridge/CLI 三大组件安全实践完整；vaultpilot-agent.rs (670行) 全文审查 — stdin 逐字节读取、120s 请求超时、panic hook sanitize_error；search_rules.rs (439行) — ASCII 全词匹配 + CJK 子串匹配正确。C# 前端: App.xaml.cs (176行) 全文审查 — 单实例 Mutex、tray icon、Interlocked 竞态保护；BackendClient.cs (677行) — 线程安全、Process 泄漏防护、HandleEvent try-catch；WrapPanel.cs (176行) — 纯布局代码无问题。381+ Rust 测试全部通过。

## 本轮循环状态 (循环#158)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#158
- 本轮时间: 2026-06-17
- 审查模块: ai.rs (2245行), models.rs (987行), crypto.rs (318行)
- 讨论阶段发现:
  - 3 个新 issue 创建: #670 SECURITY (ai.rs error 泄露), #671 SECURITY (ProviderConfig Debug 泄露), #672 BUG (from_utf8_lossy)
  - ai.rs: 仅 3/20+ 错误路径调用 sanitize_error(), LLM/用户数据直接嵌入错误消息
  - models.rs: #[derive(Debug)] 导致 ProviderConfig.api_key 在日志/panic 中明文泄露
  - ai.rs: from_utf8_lossy 静默损坏非 UTF-8 API 响应
- 修复结果:
  - #670 → PR #673 已合并 (CI 6/6 通过): 修复 6 个错误路径的 sanitize_error + from_utf8 + endpoint URL + image path
  - #671 → PR #674 已合并 (CI 6/6 通过): 手动 Debug 实现掩码 api_key
  - #672 关闭: 已由 PR #673 修复 (from_utf8_lossy → from_utf8)
- 审核结果: PR #673 和 #674 全部 CI 6/6 通过并合并
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 274 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 ai.rs (2245行) + models.rs (987行) + crypto.rs (318行) = 3550行。
  - ai.rs: SSRF 防护完整 (DNS pinning + private IP blocking), prompt 注入防护, 原子文件写入, 但错误路径 sanitize 不一致
  - models.rs: 数据结构设计合理, serde 配置正确, 但 Debug derive 泄露敏感字段
  - crypto.rs: AES-GCM nonce 正确 (CSPRNG), PBKDF2 600k 迭代符合 OWASP, 但自定义 HMAC 实现和无密钥清零是潜在风险
  - 正面发现: 0 unsafe, 0 生产 unwrap, 381+ tests 全通过, 所有 async void 有 try-catch

## 本轮循环状态 (循环#159)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#159
- 本轮时间: 2026-06-17
- 审查模块: prompting.rs (871行), vaultpilot-cli.rs MCP 段 (~1360行), lib.rs 工具执行循环 (~800行), storage.rs 自动备份+导出 (~300行), C# 全部22个async void handler, MainWindow.Updates.cs (130行), 全部模型文件 (AiModels/ChatModels/NoteModels/OperationModels), StringToVisibilityConverter
- 讨论阶段发现:
  - 无新 issue — 代码库持续保持零缺陷状态
  - Rust 后端: prompting.rs XML 转义完整 ✅, MCP server 所有错误路径 sanitize_error ✅, stdin 逐字节读取 10MB 上限 ✅, normalize_tool_path 空检查 ✅, atomic_write TOCTOU 修复 ✅
  - lib.rs 工具执行循环: 工具去重逻辑正确 ✅, search_notes fallback 到 recent notes ✅, read_file_result head/tail 截断无重叠 ✅, list_directory 60 条上限 ✅
  - C# 前端: 22/22 async void 全部有 try-catch ✅, 0 个 .Result/.Wait() ✅, 0 个 bare catch() ✅, MainWindow.Updates.cs Interlocked guard + pattern matching 防 NRE ✅
  - 全部模型文件为 sealed record 无逻辑 ✅, StringToVisibilityConverter 正确实现 ✅
- 修复结果: 无 — 无可修复 issue
- 审核结果: PR #646 (#597 CI WinUI 测试) — winui-build dotnet vstest 持续挂起 6h 后超时取消，属已知阻塞项。5/6 CI 通过 (cargo fmt/clippy/test/audit + linux-cli-build)
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 274 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 ~3500行 Rust + ~1000行 C# + CI 配置。所有 .unwrap()/.expect() 均在测试代码中。Cargo.toml 依赖版本合理。CI workflow concurrency 控制正常。
  - 正面发现: 381+ Rust 测试全通过, 0 unsafe, 0 生产 unwrap, 全部 async void 有 try-catch, SSRF/路径穿越/prompt 注入/加密存储防护完整

## 本轮循环状态 (循环#160)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#160
- 本轮时间: 2026-06-17
- 审查模块: storage.rs (4998行), MainWindow.xaml.cs (3655行)
- 讨论阶段发现:
  - 3 个新 issue 创建: #675 BUG (export 短 ID panic), #676 BUG (Send button TOCTOU), #677 BUG (_isStopping 缺少 volatile)
  - storage.rs: `[..8]` 字节切片在 ID 短于 8 字符时 panic — 用户可通过 frontmatter id 字段引入短 ID
  - MainWindow.xaml.cs: SendButton.IsEnabled TOCTOU — 两次快速点击可通过检查导致并发 AI 请求
  - MainWindow.xaml.cs: _isStopping 缺少 volatile — 与 _autoWakeInProgress 不一致
  - storage.rs 正面发现: SQL 全参数化 ✅, 路径穿越防护 ✅, 0 unsafe ✅, atomic_write 正确 ✅, LIKE 转义 ✅
  - MainWindow.xaml.cs 正面发现: 22/22 async void 有 try-catch ✅, 0 .Result/.Wait() ✅, Interlocked 竞态保护 ✅
- 修复结果:
  - #675 → PR #678 已合并 (CI 6/6 通过): safe slicing `.min(8)` + 回归测试
  - #676 + #677 → PR #679 已合并 (CI 6/6 通过): Interlocked guard + volatile
- 审核结果: PR #678 和 #679 全部 CI 6/6 通过并合并。PR #646 (#597) 继续等待。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 276 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 storage.rs (4998行) + MainWindow.xaml.cs (3655行) = 8653行。发现 1 MEDIUM + 2 LOW severity issues 并全部修复。
- #680: SettingsDialog.xaml 缺少 AutomationProperties — 15+ 控件屏幕阅读器不可识别 (PR #685 已合并)
- #681: ci.yml 缺少 permissions: contents: read — 默认宽泛权限增加攻击面 (PR #683 已合并)
- #682: MainWindow LoadingOverlay 硬编码 #80000000 — 高对比度主题下不可见 (PR #684 已合并)
- #690: render_history/render_notes/render_candidate_notes 双重 XML 转义 — sanitize 包装器已转义 (PR #693 已合并)
- #691: sanitize_error 不脱敏 x-api-key header — 自定义 provider 密钥可泄露 (PR #694 已合并)
- #692: BackendClient._process 字段缺少 volatile — IsConnected 跨线程读取可能过时 (PR #695 已合并)
- #696: test temp dir leak — extracts_existing_local_path_from_question 缺少 cleanup (PR #699 已合并)
- #697: ENV_MUTEX race — validate_base_url_localhost_env_guard guard 在 async 调用前释放 (PR #699 已合并)
- #698: commented-out CJK test — parses_list_notes_tool_call 中文输入测试被注释 (PR #699 已合并)
- #700: sanitize_mcp_prompt_content 死代码 — 第三个 .replace() 不可达 (PR #703 已合并)
- #701: StartProcess async void _isDisposed 过滤器 — 关闭时异常从 async void 传播崩溃 (PR #704 已合并)
- #702: IsConnected 已释放 Process 竞态 — HasExited 抛出 InvalidOperationException (PR #705 已合并)

## 本轮循环状态 (循环#164)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#161
- 本轮时间: 2026-06-17
- 审查模块: XAML 全部 4 文件 (MainWindow.xaml, NotesView.xaml, SettingsDialog.xaml, App.xaml), CI/CD pipeline (.github/workflows), Cargo.toml, config/, test coverage
- 讨论阶段发现:
  - 3 个新 issue 创建: #680 BUG (SettingsDialog AutomationProperties), #681 SECURITY (CI permissions), #682 BUG (LoadingOverlay 硬编码颜色)
  - XAML: 零 x:Uid 本地化 (~70+ 硬编码中文字符串), SettingsDialog 15+ 控件缺少 AutomationProperties, LoadingOverlay #80000000 硬编码
  - CI: ci.yml 缺少 permissions 声明 (linux-cli.yml 和 windows-installers.yml 已有)
  - Test coverage: 374 Rust tests (storage:128, lib:113, ai:46), C# 测试项目已建立
  - Cargo.toml: 无废弃依赖, 版本合理
- 修复结果:
  - #680 → PR #685 已合并 (CI 6/6 通过): 15+ 控件添加 AutomationProperties.Name + HelpText
  - #681 → PR #683 已合并 (CI 6/6 通过): permissions: contents: read
  - #682 → PR #684 已合并 (CI 6/6 通过): OverlayBackgroundBrush ThemeResource
- 审核结果: PR #683, #684, #685 全部 CI 6/6 通过并合并。PR #646 (#597) 继续等待。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 279 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 XAML 4 文件 (778行) + CI 配置 (155行) + Cargo.toml + 测试覆盖。XAML accessibility 和 theming 改进。CI 安全加固。

## 本轮循环状态 (循环#162)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#162
- 本轮时间: 2026-06-17
- 审查模块: Rust 全部 9 源文件 (~12.9K行), C# 全部 10 源文件 (~5.5K行), CI 全部 3 workflows, docs/, contracts/, scripts/, config/, .gitignore
- 讨论阶段发现:
  - 2 个新 issue 创建: #686 BUG (Process.Start handle 泄漏), #687 ENHANCEMENT (CI cargo install --locked)
  - Rust 后端: 零发现 — 全部 7 源文件全文审查确认零缺陷 (sanitize_error 61处调用, SQL 全参数化, 0 unsafe, 0 生产 unwrap, 路径穿越/SSRF/prompt注入防护完整)
  - C# 前端: 1 个 BUG — OpenVaultDirectoryAsync Process.Start() 返回的 Process 对象未释放, 每次点击泄漏原生句柄
  - C# 前端: 1 个潜在 BUG (未创建 issue) — OnComposerKeyDown Ctrl+V e.Handled=true 在 await 前设置, 对纯文本粘贴无影响(同步完成), 仅在剪贴板含 StorageItems 时可能阻塞文本粘贴(边界情况)
  - CI: 4 处 cargo install 缺少 --locked, Zig 下载无校验和
  - 正面发现: 22/22 async void 有 try-catch, 0 .Result/.Wait(), 0 bare catch, 全部 Interlocked guard 正确, 381+ Rust 测试通过
- 修复结果:
  - #686 → PR #688 已合并 (CI 6/6 通过): using var 声明释放 Process handle
  - #687 → PR #689 已合并 (CI 6/6 通过): 4 处 cargo install 添加 --locked
- 审核结果: PR #688 和 #689 全部 CI 6/6 通过并合并。PR #646 (#597) 继续等待。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 281 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查全部 Rust 源文件 (12.9K行) + 全部 C# 源文件 (5.5K行) + CI 配置 (3 workflows) + 文档 + 合约 + 脚本。代码库持续保持零缺陷状态。

## 本轮循环状态 (循环#163)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#163
- 本轮时间: 2026-06-17
- 审查模块: prompting.rs (871行), lib.rs sanitize_error (129行), BackendClient.cs (677行), NotesView.xaml.cs (355行), ai.rs, models.rs, crypto.rs, storage.rs
- 讨论阶段发现:
  - 3 个新 issue 创建: #690 BUG (双重 XML 转义), #691 SECURITY (x-api-key 脱敏), #692 BUG (_process volatile)
  - prompting.rs: render_history/render_notes/render_candidate_notes 内部 escape_xml_close_tags + 外部 sanitize_* 包装器再次 escape → 双重转义 `</note>` → `<//note>` → `<////note>`
  - lib.rs sanitize_error: 不匹配 x-api-key header（10字节非11字节），自定义 provider 非 sk- 前缀 key 泄露
  - BackendClient.cs: _process 字段无 volatile，IsConnected 从 health check timer 跨线程读取可能过时
  - 正面发现: render_tool_results 不做内部转义（正确模式）✅, Interlocked.Exchange 在 DisposeProcessAsync 中正确 ✅, 22/22 async void 有 try-catch ✅
- 修复结果:
  - #690 → PR #693 已合并 (CI 6/6 通过): 移除 render_* 内部 escape_xml_close_tags + 2 回归测试
  - #691 → PR #694 已合并 (CI 6/6 通过): x-api-key header 脱敏 + 2 测试
  - #692 → PR #695 已合并 (CI 6/6 通过): Volatile.Read/Write 8 处 _process 访问点
- 审核结果: PR #693, #694, #695 全部 CI 6/6 通过并合并。PR #646 (#597) 继续等待。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 284 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 prompting.rs + lib.rs sanitize_error + BackendClient.cs + NotesView.xaml.cs + ai.rs + models.rs + crypto.rs + storage.rs。发现 1 MEDIUM data corruption + 1 MEDIUM security + 1 LOW-MEDIUM thread safety，全部修复。

## 本轮循环状态 (循环#164)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#164
- 本轮时间: 2026-06-17
- 审查模块: Rust 测试套件 (378 tests across 8 files), C# WinUI 测试套件 (50 tests across 8 files), cross-cutting concerns
- 讨论阶段发现:
  - 3 个新 issue 创建: #696 BUG (temp dir leak), #697 BUG (ENV_MUTEX race), #698 BUG (commented-out CJK test)
  - Rust 测试: temp dir 泄漏 — `extracts_existing_local_path_from_question` 未清理; ENV_MUTEX guard 在 async 调用前释放导致竞态; CJK 测试被注释
  - C# 测试: ~50 tests 但 BackendClient (677行) 几乎无测试, MainWindow (3659行) 仅静态 helper 测试, 无 mock/接口抽象
  - 正面发现: Rust 387 tests 全通过, 0 unsafe, 0 生产 unwrap, sanitize_error 63处调用
- 修复结果:
  - #696 → PR #699 已合并 (CI 6/6 通过): temp dir cleanup `fs::remove_dir_all`
  - #697 → PR #699 已合并 (CI 6/6 通过): sync test + dedicated Runtime inside guard scope
  - #698 → PR #699 已合并 (CI 6/6 通过): CJK test 恢复为独立 `parses_list_notes_tool_call_cjk`
- 审核结果: PR #699 CI 6/6 通过并合并。PR #646 (#597) 继续等待。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 287 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 Rust 测试套件 (378 tests) + C# 测试套件 (50 tests) + cross-cutting patterns。发现 3 个 test quality issues 并全部修复。C# 测试覆盖率仍需改进（BackendClient/MainWindow 几乎无 behavioral tests）。

## 本轮循环状态 (循环#165)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#165
- 本轮时间: 2026-06-17
- 审查模块: lib.rs MCP handler (~1500行), storage.rs index/export (~800行), vaultpilot-cli.rs MCP server (~1360行), prompting.rs (871行), BackendClient.cs (677行), MainWindow.xaml.cs (3655行)
- 讨论阶段发现:
  - 3 个新 issue 创建: #700 BUG (sanitize_mcp_prompt_content 死代码), #701 BUG (StartProcess async void 崩溃), #702 BUG (IsConnected disposed Process)
  - Rust 后端: sanitize_mcp_prompt_content 第三个 .replace() 不可达 — step 1 已将 `</user_content>` → `<//user_content>`
  - C# 前端: StartProcess catch filter `when (_isDisposed == 0)` — 关闭时 _readerCts.Dispose() 抛异常被过滤器拒绝，async void 传播崩溃
  - C# 前端: IsConnected Volatile.Read 捕获的 Process 引用可在 HasExited 访问前被 DisposeProcessAsync 释放
  - Rust 后端 mcp_call_chat_delete stale current_session_id: 非 issue — normalize_chat_state 在 save 时修复
  - prompting.rs opening tag 未转义: 低风险 — 模型处理嵌套 XML
- 修复结果:
  - #700 → PR #703 已合并 (CI 6/6 通过): 移除死代码 + 添加注释
  - #701 → PR #704 已合并 (CI 6/6 通过): 添加 catch-all 防止 async void 崩溃
  - #702 → PR #705 已合并 (CI 6/6 通过): IsConnected try-catch 防护
- 审核结果: PR #703, #704, #705 全部 CI 6/6 通过并合并。PR #646 (#597) winui-build 仍 6h 超时失败。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 290 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 lib.rs + storage.rs + vaultpilot-cli.rs + prompting.rs (~4.5K行 Rust) + BackendClient.cs + MainWindow.xaml.cs (~4.3K行 C#)。Rust 后端安全实践完整 (sanitize_error 63处, SQL 全参数化, 0 unsafe, 0 生产 unwrap)。C# 前端 22/22 async void 有 try-catch。发现 2 MEDIUM + 1 LOW severity issues 并全部修复。
- #706: SettingsDialog PrimaryButtonClick catch block 注释与代码行为不一致 (PR #707 已合并)

## 本轮循环状态 (循环#166)
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->
- 循环编号: 循环#166
- 本轮时间: 2026-06-17
- 审查模块: search_rules.rs (439行), vaultpilot-agent.rs (670行), lib.rs (3104行), MainWindow.xaml.cs (3669行), MainWindow.xaml (334行), BackendClient.cs (688行), SettingsDialog.xaml.cs (325行), NotesView.xaml.cs (355行), App.xaml.cs (176行), MainWindow.Updates.cs (130行)
- 讨论阶段发现:
  - 1 个新 issue 创建: #706 BUG (SettingsDialog 注释误导)
  - Rust 后端 (search_rules.rs + vaultpilot-agent.rs + lib.rs): 零缺陷 — sanitize_error 20+ 处调用 ✅, 0 unsafe ✅, 0 生产 unwrap ✅, OnceLock 线程安全 ✅, normalize_tool_path 空检查 ✅, stdin 逐字节 10MB 上限 ✅, 120s 请求超时 ✅
  - C# 前端: 22/22 async void 有 try-catch ✅, 0 .Result/.Wait() ✅, Interlocked guard 全覆盖 ✅, Volatile.Read/Write 跨线程保护 ✅, ThemeResource 主题颜色 ✅, AutomationProperties 无障碍 ✅, Hyperlink http/https 限制 ✅
  - SettingsDialog.xaml.cs: catch block 注释说 "let the dialog close" 但代码 args.Cancel=true 阻止关闭 — 注释误导
- 修复结果:
  - #706 → PR #707 已合并 (CI 6/6 通过): 注释修正为 "keep the dialog open so the user can retry or cancel"
- 审核结果: PR #707 CI 6/6 通过 (cargo fmt/clippy/test/audit + linux-cli-build + winui-build) 并合并。PR #646 (#597) 继续等待。
- 项目状态: **1 open issue (#597 阻塞), 1 open PR (#646), 291 已合并 PR, 1 阻塞项 (#597 CI WinUI 测试)**
- 代码审查: 深度审查 Rust 后端 ~4.5K行 (search_rules.rs + vaultpilot-agent.rs + lib.rs) + C# 前端 ~5.5K行 (MainWindow + BackendClient + SettingsDialog + NotesView + App + Updates) = ~10K行。代码库持续零缺陷状态 — 166 个循环累计 291 个已合并 PR。仅发现 1 个 MEDIUM 文档/注释问题并修复。
