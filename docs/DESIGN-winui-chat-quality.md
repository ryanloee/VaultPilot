# VaultPilot WinUI 聊天体验与发布质量设计说明

> 适用端：Windows WinUI 3（native/VaultPilot.WinUI）
> 最后更新：2026-08-02（v0.6.68+）
> 状态：经 2026-08-02 后端断开事故复盘后修订

本文档记录三类反复出现的问题的设计决策与防回归机制：
**请求处理反馈（全屏弹窗）**、**Markdown 渲染（性能）**、**后端连接稳定性（断开）**，
以及发布前的质量门禁。修改相关代码前必须先读本文档。

---

## 1. 请求处理中的用户反馈设计

### 1.1 决策：不使用全屏遮罩弹窗

AI 请求进行中，客户端使用**三重非阻塞反馈**，禁止全屏遮罩：

| 反馈层 | 实现 | 作用 |
|--------|------|------|
| 状态栏 | `UpdateStatusBar("info", statusTitle, statusDetail)` | 轻量文字提示（如"助手处理中 / 正在准备请求..."） |
| 消息区 | `ShowThinkingIndicator()`（`__thinking__` 特殊 MessageItem） | 在对话流中显示"思考中"气泡，用户可继续阅读上下文 |
| Composer 取消 | `CancelButton.Visibility = Visible` | 请求期间显示取消按钮，`CancelActiveRequest()` 取消 CTS |

### 1.2 为什么禁止全屏遮罩（历史教训）

全屏遮罩（`LoadingOverlay`：50% 黑 + ProgressRing + 取消按钮）在本项目**出现又删除过三次**：

1. `0e32c18b`（#326）：首次加入全屏 LoadingOverlay
2. `d79bcf00`：**已决定删除**，替换为内联进度指示
3. `44349b97`（#3607 / #3616）：被重新加回，且后续 `5f1d8902`（#3607）又加了遮罩内取消按钮
4. `3a394c78`（2026-08-02）：再次彻底删除（XAML + Show/Hide 方法 + 取消按钮 handler）

**反复回归的根因**：每次"请求无反馈"类 issue 出现时，修复者默认选择全屏遮罩，
没有意识到三重非阻塞反馈已经覆盖需求。全屏遮罩的害处：
- 遮挡聊天内容，用户无法查看/复制已生成的上下文，观感等同"卡死"
- 与 thinking 气泡、状态栏、composer 取消按钮功能三重冗余
- 旋转的 ProgressRing 在长请求（LLM 可到 2-3 分钟）期间制造"卡住"焦虑

### 1.3 防回归规则

- **新增任何"处理中"UI 前必须读本节**。反馈需求应先评估三重反馈是否已满足。
- 代码评审中发现新增全屏遮罩（`Grid` + `Background="#80..."` + `ProgressRing` 覆盖全窗），
  默认打回。
- 聊天主流程的全屏 `LoadingOverlay`（XAML 元素、Show/Hide 方法、取消按钮 handler）
  已全部删除；若在主聊天视图发现新的全窗遮罩，属于回归，按本规则处理。
- `QuickAskOverlay`（快速提问）与 `AiCommandPalette`（命令面板）各自保留的
  局部 `LoadingOverlay`（540×140 卡片，仅覆盖各自弹出层，不是全窗遮罩）不违反
  本节规则，属于各自控件的局部反馈，不是聊天三重反馈的回归（#3854）。

---

## 2. Markdown 渲染设计

### 2.1 渲染链

```
MessageItem.Text (assistant)
  → AppendMessageTo → CreateMessageContent(text, isAssistant, isUser)
    → TryExtractMarkdownPayload(text, out markdown)
      → CreateMarkdownContent(markdown)
        → ParseMarkdownBlocks (代码块 / 表格 / 文本块)
          → CreateCodeBlock / CreateMarkdownTable / CreateMarkdownTextElements
            → AppendInlineMarkdown (行内代码 `、[[wikilink]]、[链接](url)、**粗体**、*斜体*)
              → AppendNoteRefText (笔记标题自动链接)
```

### 2.2 触发条件

仅 assistant 消息且文本满足任一条件才走 Markdown 渲染：

1. 以 `<vp-markdown>` 开头并以 `</vp-markdown>` 结尾（后端 prompt 要求 LLM 包装）
2. `LooksLikeMarkdownPayload` 特征检测：含 ```、**、行内 `、表格分隔、markdown 链接、
   标题（#）、≥2 条列表项，或 1 条列表 + ≥4 行

注意：**纯文本回复（无任何 markdown 特征）不走渲染分支，显示为普通 TextBlock 是正常行为**，
不是 bug。若 AI 回复含 `**`/`#`/列表却显示纯文本，才是渲染回归。

### 2.3 性能保障（渲染必须快）

渲染发生在 UI 线程，长会话/长消息必须满足以下架构约束：

| 机制 | 位置 | 说明 |
|------|------|------|
| ItemsRepeater 虚拟化 | MainWindow.xaml `MessagesRepeater` | 仅视口内消息在视觉树中，500 轮会话 ≈ 10 个活跃元素 |
| 渲染缓存 | `_itemRenderCache`（按 TurnId，上限 300） | 滚动回看不重建；会话切换/压缩时 `ClearRenderCache()` |
| 增量渲染 | `AppendNewTurns()` | 只追加新增轮次，禁止每次发送全量 clear+rebuild |
| 解析线性化 | `ParseMarkdownBlocks` / `AppendInlineMarkdown` | 均为 O(n) 线性扫描，禁止引入嵌套循环或每调用一次排序 |
| NoteRefs 缓存 | `NoteRefs._sortedTitlesCache` | 标题表按长度排序+小写只做一次，后续命中缓存 |
| 估算代替 Measure | composer 高度 | 禁止在 TextChanged/KeyDown 里调 `UIElement.Measure()` |

**铁律**：任何渲染热路径（每次发送/滚动触发）不得出现 `Measure()`、O(n log n) 排序、
或正则回溯。违反此条的性能回归按 P1 处理。

---

## 3. 后端连接稳定性设计

### 3.1 IPC 架构

WinUI ↔ `vaultpilot-agent.exe`（Rust）通过 stdin/stdout JSON-RPC 通信。
agent 是**单进程长驻**：主线程 `runtime.block_on(handle_line(...))` 串行处理请求；
**任何未捕获 panic 都会终止进程** → stdout EOF → WinUI 判定"后端断开"。

### 3.2 超时分层（必须保持对齐）

| 层 | 值 | 位置 |
|----|----|------|
| agent 请求超时 | 120s | `vaultpilot-agent.rs` `REQUEST_TIMEOUT` |
| AI 调用超时 | 120s/次 | `ask.rs` `AI_CALL_TIMEOUT`（select_tool_call / answer_*） |
| WinUI IPC 超时（非 AI 请求默认） | 180s | `BackendClient.cs` `DefaultIpcTimeout` |
| WinUI AI 请求的 IPC 超时 | RequestTimeoutMs + 90s（作为 `SendAsync` 的 `requestTimeout` 传入） | `ChatInputHandler.cs` `aiTimeout` |

规则：WinUI 对 AI 请求实际使用的 IPC 超时是 `aiTimeout = RequestTimeoutMs + 90s`，
不是固定的 180s。必须保证 `aiTimeout` > agent 内部超时（120s），即
`RequestTimeoutMs` 必须大于 30s（默认 60s）；设置里目前允许 1s 之类的极低值
（#3801），此时 WinUI 会先于 agent 超时，触发 `TryReconnectWithRetryAsync`
杀掉 agent 进程重启，用户看到"后端断开"。

### 3.3 断开判定与修复记录（2026-08-02 复盘）

现象：发送消息 → 转圈 → "后端断开"。crash.log 反复出现
`UNOBSERVED: (Rust 后端已关闭输出通道。)`。

判定链条：

```
agent stdout EOF（进程退出/被杀）
  → PumpStdoutAsync FailPending("Rust 后端已关闭输出通道。")
  → SendAsync 抛 InvalidOperationException → 聊天显示错误
  → TryReconnectWithRetryAsync 重启 agent
```

已修复的两个 WinUI 侧缺陷：

1. **unobserved 异常竞态**（`5fcc0cd1`）：SendAsync 超时抛 TimeoutException 后，
   finally 移除 pending 条目；若 `FailPending` 恰在此前已对该 TCS `TrySetException`，
   异常无人 await → `TaskScheduler.UnobservedTaskException` → crash.log 假崩溃记录。
   修复：每个 TCS 挂 `OnlyOnFaulted` ContinueWith 观察异常。

2. **agent stderr 不落盘**（`3a394c78`）：此前 stderr 仅内存 50 行，
   重连重启后丢失，断开无法事后诊断。修复：落盘到
   `%LOCALAPPDATA%\com.local.vaultpilot\logs\agent.log`（带时间戳，写失败不影响主流程）。

**待取证**：agent 进程退出的根因（panic / 外部杀 / 挂起被超时误杀）由
`%APPDATA%\com.local.vaultpilot\agent-crash.log` 判定（agent panic hook 会写
`panic on thread 'main': ... at src/...`）。拿到日志后按 3.4 流程处置。

### 3.4 诊断设施清单（排查"后端断开"先看这些）

| 文件 | 路径 | 内容 |
|------|------|------|
| agent 崩溃日志 | `%APPDATA%\com.local.vaultpilot\agent-crash.log` | agent panic 记录（512KB 轮转） |
| agent stderr | `%LOCALAPPDATA%\com.local.vaultpilot\logs\agent.log` | agent stderr 逐行落盘（v0.6.69+） |
| WinUI 崩溃 | `%LOCALAPPDATA%\com.local.vaultpilot\logs\crash.log` | WinUI 异常/unobserved 记录 |
| 启动日志 | `%LOCALAPPDATA%\com.local.vaultpilot\startup.log` | 启动各步骤时间线 |

---

## 4. 发布质量保障（CI 门禁）

### 4.1 教训

- 2026-08-02 前：发版流程（windows-installers.yml）打包后**冒烟测试被跳过**
  （runner 预装 WinAppSDK 与 self-contained 包冲突假崩 0xC000027B），
  **发版包零运行时验证**；C# 单元测试只编译不运行（#597）；
  后端 agent 无任何启动/IPC 测试。→ 每次发版是否可用完全靠用户人工发现。
- 2026-07 前：Windows Installers CI 静默失败 18 个版本无人发现（已在 Release
  条件加入 Windows CI 检查）。

### 4.2 测试矩阵（2026-08-02 补齐后）

| 检查 | 执行位置 | 门禁 |
|------|----------|------|
| cargo test --workspace（Windows） | windows-installers validate | 阻塞发版 |
| cargo clippy -D warnings + cargo audit | windows-installers validate | 阻塞发版 |
| WinUI Debug 构建 | windows-installers validate | 阻塞发版 |
| **WinUI 启动冒烟**（smoke-test-winui.ps1，8s 存活 + crash.log 检查） | windows-installers validate + ci.yml | 阻塞发版/PR |
| **agent 后端冒烟**（smoke-test-agent.ps1：ping→getSettings→listNotes→20s 心跳 + panic 检查） | windows-installers validate + ci.yml | 阻塞发版/PR |
| Linux CLI 冒烟（smoke-test-linux-cli.sh） | windows-installers linux_build | 阻塞发版 |
| Velopack 打包 | windows-installers build | 阻塞发版 |

release job `needs: [build, linux_build]`，build `needs: validate` →
**validate 的冒烟测试全部通过后才会打包发版**。

### 4.3 冒烟测试脚本

- `scripts/smoke-test-agent.ps1`：启动 agent（隔离 LOCALAPPDATA/APPDATA/HOME）→
  ping / getSettings / listNotes / 每 5s 心跳共 20s → 检查 agent-crash.log 无 panic。
  这是"后端断开"类回归的直接防线。
- `scripts/smoke-test-winui.ps1`：启动 WinUI exe → 存活 8s → crash.log 无致命异常。
- 两者均可在本地 Windows 上手动运行（PowerShell）：
  `.\scripts\smoke-test-agent.ps1 -ExePath .\path\to\vaultpilot-agent.exe`

### 4.4 发版前检查清单

1. validate job 全绿（含两个冒烟测试）
2. WinUI 构建产物在 runner 上启动存活（smoke-test-winui.ps1 通过）
3. agent 后端 JSON-RPC 全链路响应 + 20s 心跳（smoke-test-agent.ps1 通过）
4. 已知开放问题：WinUI 打包后安装器的运行时冒烟受 runner WinAppSDK 冲突限制，
   由 Debug exe 冒烟替代覆盖；C# 单元测试受 #597 限制仅编译。
