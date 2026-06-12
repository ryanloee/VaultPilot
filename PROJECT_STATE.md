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
- #177: API 响应体 50MB 大小限制防 OOM (PR #246 待合并 — 合并冲突)
- #157: AppSettings 缓存避免 N+1 磁盘 IO (PR #247 已合并)

## 当前进行中
<!-- 由 issue-monitor 任务在创建 PR 后更新 -->

- PR #244: fix/issue-227-197-cjk-tokens-double-save — #227 (CJK token 估算修复) + #197 (chat 双重保存移除) ⚠️ 状态: OPEN / CONFLICTING，需作者 rebase 到最新 main 分支解决冲突
- PR #246: fix/issue-177-response-size-limit — #177 (API 响应体 50MB 大小限制) ⚠️ 状态: OPEN / CONFLICTING，代码审查通过但需 rebase 解决冲突（main 已合并 #245, #247）

## 已知阻塞项
<!-- 记录失败的修复尝试、需要人工介入的问题 -->

- #192 (extract_json_block 双转义): 已有 2 次失败 PR 尝试 (#211, #201)，需重新分析根因后再修复，本轮暂不处理
- PR #244 (fix/issue-227-197-cjk-tokens-double-save): 合并冲突 + CI 未触发，需作者 rebase 后重新提交
- PR #246 (fix/issue-177-response-size-limit): 代码逻辑正确（streaming 读取 + 50MB 上限），但合并冲突需 rebase（main 已合并 WAL 和缓存相关的 storage.rs 变更）

## 决策记录
<!-- 指挥官任务的重要决策 -->

- 2026-06-12: 进入稳定化阶段，优先修 Bug，暂停新功能开发
- 2026-06-12: 架构重构类 issue（如 lib.rs 拆分）暂不自动化处理，留人工决策
- 2026-06-12 [循环#1]: 当前 50+ open issue，已远超 30 阈值，暂停创建新 Enhancement/UI 类 issue，集中精力消化存量
- 2026-06-12 [循环#1]: #192 有两次失败修复，标记为阻塞项，需要不同策略（如状态机方案）
- 2026-06-12 [循环#1]: #174 和 #186 都涉及 save_note/atomic_write 的写入安全性，考虑在同一 PR 中一并修复
- 2026-06-12 [循环#2]: 循环#1 修复目标 3/5 完成（#174+#186 ✅, #229 ✅, #227+#197 ⏳ PR #244 待 rebase），额外完成 7 个 issue（#226, #225, #236, #212, #205, #175, #182, #179, #188）
- 2026-06-12 [循环#2]: 开放 Bug 仍有 16 个，但多数涉及 WinUI 前端或复杂状态管理，本轮聚焦 Rust 后端可自动化修复的问题
- 2026-06-12 [循环#2]: 选定 3 个后端高价值 issue 作为修复目标：#177 (OOM), #230 (WAL), #157 (N+1 IO)

## 项目健康度快照
<!-- 每轮循环更新 -->

| 指标 | 循环#1 | 循环#2 |
|------|--------|--------|
| Open issues 总数 | 50+ | 50+ |
| Open Bug 数 | 16 | 16 |
| Open Performance 数 | 8 | 8 |
| 已合并 PR | 11 | 13 |
| 进行中 PR | 1 (#244) | 2 (#244, #246) |
| 阻塞项 | 2 | 3 |

## 本轮循环状态
<!-- 指挥官在每轮开始时写入，各任务读取后执行 -->

- 循环编号: 2
- 上次循环时间: 2026-06-12T09:30:00Z
- 讨论重点: Bug 修复 — 崩溃防护与并发安全
- 本轮修复目标:
  1. #177 — API 响应体无大小限制，异常响应可导致 OOM 崩溃 (🟡→🔴 实际是崩溃级风险)
  2. #230 — SQLite 未启用 WAL 模式和 busy_timeout，并发访问 SQLITE_BUSY (🔵→🟠 影响多实例场景)
  3. #157 — open_connection 每次重读 settings.json，循环中 N+1 IO (🟡 性能)
- 本轮审查目标: 检查 PR #244 是否已 rebase，以及循环#1 合并的 PR 是否引入回归
- 本轮新建 issue 预算: 0（50+ open issue，远超阈值，不新建）
- 备注: #155 (Settings 输入验证) 和 #128 (退出时丢失对话) 也是高价值 Bug，但涉及 WinUI 前端改动，留待下一轮或人工处理
- 本轮修复结果: 3/3 完成 ✅
  - #177 → PR #246 (response.body 50MB 大小限制，新增 reqwest stream + bytes + futures-util 依赖)
  - #230 → PR #245 (ensure_schema 启用 WAL + busy_timeout，两个代码路径均覆盖)
  - #157 → PR #247 (StorageContext 新增 cached_settings 字段，load/save 联动更新缓存)
