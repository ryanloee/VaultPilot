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
- #412: SECURITY — PBKDF2-HMAC-SHA256 密钥派生 600k 迭代 (PR #413 已合并)
- #410: SECURITY — HTTP bridge CORS very_permissive → localhost 来源限制 (PR #414 已合并)
- #411: BUG — LIKE 通配符 % _ \ 转义 (PR #415 已合并)

## 当前进行中
<!-- 由 issue-monitor 任务在创建 PR 后更新 -->

（无）

## 已知阻塞项
<!-- 记录失败的修复尝试、需要人工介入的问题 -->

（无 — 所有阻塞项已清空）

## 决策记录
<!-- 指挥官任务的重要决策 -->

- 2026-06-12: ~~进入稳定化阶段，优先修 Bug，暂停新功能开发~~ → 已过时，见循环#40
- 2026-06-12: 架构重构类 issue（如 lib.rs 拆分）暂不自动化处理，留人工决策
- 2026-06-12 [循环#1]: 当前 50+ open issue，已远超 30 阈值，暂停创建新 Enhancement/UI 类 issue，集中精力消化存量
- 2026-06-13 [循环#26]: **存量已从 50+ 消化至 5 个**，解除"暂停创建新 issue"限制。讨论任务恢复主动审查代码质量和提出新 issue 的职责
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
- 2026-06-14 [修复轮#72]: 修复循环#71 创建的 3 个 issue，3/3 全部完成
- 2026-06-14 [修复轮#72]: #412 PBKDF2-HMAC-SHA256 密钥派生 600k 迭代 (PR #413)
- 2026-06-14 [修复轮#72]: #410 CORS 限制为 localhost 来源 (PR #414)
- 2026-06-14 [修复轮#72]: #411 LIKE 通配符转义 (PR #415)
- 2026-06-14 [PR审核轮#72]: 审核并合并 3 个 PR (#413, #414, #415)，CI 5/6 通过（cargo audit 预存在 CVE），代码审查无问题
- 2026-06-14 [PR审核轮#72]: 项目恢复 0 open issue + 0 open PR 状态，累计 172 已合并 PR
- 2026-06-14 [循环#73]: 全代码库三路并行深度审查（Rust 后端 + C# 前端 + 依赖/CI），创建 3 个高质量 issue
- 2026-06-14 [循环#73]: Rust 后端审查发现 15 项（1 HIGH + 6 MEDIUM + 5 LOW），C# 前端发现 14 项（2 HIGH + 4 MEDIUM + 8 LOW）
- 2026-06-14 [循环#73]: 新发现：crypto.rs derive_machine_key 裸 SHA-256 KDF 无密钥拉伸（#416），CancelActiveRequest 无 Volatile.Read（#417），serde_yml unmaintained（#418）
- 2026-06-14 [循环#73]: cargo audit 新增 2 个警告（serde_yml + libyml），rustls-webpki CVE 从 1 个增至 3 个

## 项目健康度快照
<!-- 每轮循环更新 -->

| 指标 | 循环#43 | 循环#44 | 循环#45 | PR审核轮 | 循环#47 | 循环#48 | 循环#53 |
|------|---------|---------|---------|----------|---------|---------|---------|
| Open issues 总数 | 3 | 2 | 2 | 0 ✅ | 3 | 5 | 0 ✅ |
| Open Bug 数 | 3 | 2 | 2 | 0 ✅ | 2 | 3 | 0 ✅ |
| Open Security 数 | 0 | 0 | 0 | 0 | 1 | 1 | 0 ✅ |
| Open Performance 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| Open Enhancement 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 1 | 0 ✅ |
| Open UI 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| Open Feature 数 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |
| 已合并 PR | 115+ | 116+ | 116+ | 146 | 146 | 146 | 154 |
| 进行中 PR | 0 | 0 | 2 | 0 | 0 | 0 | 0 |
| 阻塞项 | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ | 0 ✅ |

## 本轮循环状态
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->

- 循环编号: #73
- 本轮时间: 2026-06-14
- 讨论重点: 全代码库深度审查（Rust 后端 + C# 前端 + 依赖/CI）
- 项目状态: **3 open issue, 0 open PR, 172 已合并 PR, 0 阻塞项**
- 本轮修复目标:
  - #416 (SECURITY — crypto.rs derive_machine_key 裸 SHA-256 KDF)
  - #417 (BUG — CancelActiveRequest 读取 _activeRequestCts 缺少 Volatile.Read)
  - #418 (DEPS — serde_yml 0.0.12 unsound & unmaintained)
- 本轮审查目标:
  - 全部 3 个 PR 的代码质量和 CI 通过情况
- 创建的 issue:
  - #416: SECURITY — crypto.rs derive_machine_key 使用自定义 SHA-256 KDF 缺少密钥拉伸
  - #417: BUG — CancelActiveRequest 读取 _activeRequestCts 缺少 Volatile.Read 保护
  - #418: DEPS — serde_yml 0.0.12 unsound & unmaintained (RUSTSEC-2025-0068)
- 审查发现摘要:
  - Rust 后端: 15 项（1 HIGH + 6 MEDIUM + 5 LOW），代码质量优秀，无 unsafe/生产 unwrap
  - C# 前端: 14 项（2 HIGH + 4 MEDIUM + 8 LOW），async void 保护全面
  - 依赖: 3 CVE (rustls-webpki) + 4 warnings (serde_yml, libyml, rand, time)
  - CI: 5 项改进建议（audit.toml、缓存策略统一、rustfmt 冗余等）
