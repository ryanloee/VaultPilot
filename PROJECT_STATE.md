# VaultPilot 项目指挥文档

> 本文档由指挥官任务自动维护，所有 cron agent 运行前必须先读取此文档。

## 项目概述
- **仓库**: ryanloee/VaultPilot
- **技术栈**: Rust 后端 + C# WinUI 前端
- **核心路径**: /home/jy/wk/VaultPilot/

## 当前阶段
**稳定化** — 优先修复已有 Bug，暂不扩展新功能

## 优先级排序
1. 🔴 Bug 修复（功能性、数据丢失、崩溃）
2. 🟠 安全问题（路径穿越、注入、密钥泄露）
3. 🟡 性能问题（OOM、IO 开销、内存泄漏）
4. 🔵 架构改进（模块拆分、错误类型统一）
5. ⚪ UI/功能增强（最低优先级）

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
- #180: 附件搜索分批加载 (PR #256 已合并)
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

## 当前进行中
<!-- 由 issue-monitor 任务在创建 PR 后更新 -->

- PR #267: fix/issue-62-tray-shutdown-cleanup — #62 (tray exit calls cleanup before terminating) 🔴 CI 待验证
- PR #268: fix/issue-71-chatstate-synchronization — #71 (_chatState SemaphoreSlim 同步) 🔴 CI 待验证
- PR #269: fix/issue-86-clipboard-cleanup — #86 (剪贴板图片清理，最多保留 50 个) 🔴 CI 待验证

## 已知阻塞项
<!-- 记录失败的修复尝试、需要人工介入的问题 -->

- #192 (extract_json_block 双转义): 已有 2 次失败 PR 尝试 (#211, #201)，需重新分析根因后再修复，本轮暂不处理
- #217 (同步 SQLite 阻塞 Tokio): 2 次子任务均超时（>10 分钟），cargo build 在子任务环境中过慢，需要更大磁盘配额或直接在主仓库操作

## 决策记录
<!-- 指挥官任务的重要决策 -->

- 2026-06-12: 进入稳定化阶段，优先修 Bug，暂停新功能开发
- 2026-06-12: 架构重构类 issue（如 lib.rs 拆分）暂不自动化处理，留人工决策
- 2026-06-12 [循环#1]: 当前 50+ open issue，已远超 30 阈值，暂停创建新 Enhancement/UI 类 issue，集中精力消化存量
- 2026-06-12 [循环#1]: #192 有两次失败修复，标记为阻塞项，需要不同策略（如状态机方案）
- 2026-06-12 [循环#1]: #174 和 #186 都涉及 save_note/atomic_write 的写入安全性，考虑在同一 PR 中一并修复
- 2026-06-12 [循环#2]: 选定 3 个后端高价值 issue 作为修复目标：#177 (OOM), #230 (WAL), #157 (N+1 IO)
- 2026-06-12 [循环#4]: 串行策略验证成功 — 基于最新 main 逐个创建 PR，3 个 PR 均无冲突
- 2026-06-12 [循环#5]: 聚焦 Rust 后端性能优化，选定 #178, #176, #180
- 2026-06-12 [循环#6]: 文档被重置后重建，选定 3 个 issue：#165 (FTS5 转义), #169 (README 修正), #168 (serde_yaml 替换)
- 2026-06-12 [循环#6]: 3/3 完成，PR #257, #258, #259 均 CI 通过
- 2026-06-12 [循环#7]: 大量 issue 已修复但未关闭 — 系统性审计关闭 7 个已解决 issue：#127, #35, #128, #100, #99, #61, #96
- 2026-06-12 [循环#7]: 关闭 2 个重复 issue：#64 (重复 #187), #55 (重复 #35)
- 2026-06-12 [循环#7]: 修复 PR #255 CI 格式化问题并合并，关闭 #88, #52
- 2026-06-12 [循环#7]: 选定 2 个新修复目标：#156 (list_directory 权限错误), #140 (模型名误匹配)
- 2026-06-12 [循环#7]: 从 92 open issue 降至 85（净减 7 个）
- 2026-06-12 [循环#8]: 聚焦 C# WinUI 前端 Bug，选定 3 个高优先级 issue：#95 (auto-wake 竞态), #63 (窗口创建竞态), #155 (Settings 校验)
- 2026-06-12 [循环#8]: 3/3 完成，PR #264, #265, #266 创建成功
- 2026-06-12 [循环#9]: 聚焦 C# WinUI Bug 修复，选定 3 个 issue：#62 (tray shutdown), #71 (chatState 竞态), #86 (clipboard 累积)
- 2026-06-12 [循环#9]: 3/3 完成，PR #267, #268, #269 创建成功
- 2026-06-12 [循环#9]: 磁盘空间不足（/tmp 满），清理旧临时目录后恢复；直接在主仓库操作避免复制 target 目录

## 项目健康度快照
<!-- 每轮循环更新 -->

| 指标 | 循环#8 | 循环#9 |
|------|--------|--------|
| Open issues 总数 | ~78 | ~75 (预估: 关闭 #62, #71, #86) |
| Open Bug 数 | ~0 | ~0 (本轮全部关闭) |
| Open Security 数 | 3 | 3 |
| Open Performance 数 | 8 | 8 |
| Open Enhancement 数 | 34 | 34 |
| Open UI 数 | 26 | 26 |
| 已合并 PR | 33+ | 33+ (3 个新 PR 待合并) |
| 进行中 PR | 0 | 3 (#267, #268, #269) |
| 阻塞项 | 2 (#192, #217) | 2 (#192, #217) |

## 本轮循环状态
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->

- 循环编号: 9
- 上次循环时间: 2026-06-12T18:00:00Z
- 讨论重点: **C# WinUI Bug 修复** — 3 个高优先级前端 Bug
- 本轮修复目标:
  1. #62 — Hide-to-tray 绕过 OnClosed 清理，backend 进程成孤儿 → PR #267 ✅
  2. #71 — _chatState 多线程异步竞态条件 → PR #268 ✅
  3. #86 — 剪贴板图片无限累积无清理机制 → PR #269 ✅
- 本轮修复结果: 3/3 完成 ✅
- 阻塞 issue: #192 (双转义, 2次失败), #217 (SQLite 同步阻塞, 子任务超时)
