## 竞品参考

Obsidian Copilot (v3/v4) 已实现 `@` 符号触发上下文限定功能：
- **@folder**：将 AI 搜索范围限定到指定文件夹
- **@note**：将 AI 搜索范围限定到指定笔记
- **@tag**：按标签筛选搜索范围
- **@web**（Plus 付费）：进行实时网络搜索

用户在输入 `@` 后弹出下拉菜单，可浏览文件夹/笔记/标签，AI 优先搜索限定范围再扩展到整个库。Copilot 用户反馈"这个功能是 killer feature，让我不用手工整理 context window"。

Mem.ai 也有类似的"Collections"限定功能，但更偏自动化 tagging 而非手动 @ 选择。

## 差距分析

**VaultPilot 当前状态**：VaultPilot 的 RAG 搜索是全局性的——每次查询都搜索整个 vault。用户无法在聊天中指定搜索范围，导致：
1. 用户想问"这个项目的最新进度"时，AI 可能从无关项目中找到内容
2. 大 vault（上千笔记）中搜索结果噪音大
3. 无法快速在某个文件夹/标签范围内进行深度问答

**竞品做法**：Obsidian Copilot 通过在 chat input 中输入 `@` 触发智能下拉菜单，让用户选择文件夹/标签/笔记作为搜索过滤器。用户选择后，后端将该限定条件作为搜索的前置 filter。

**差距**：缺少一种轻量级、用户在聊天中即可完成的范围限定机制。

## 建议方案

扩展 RAG 搜索 API，支持按文件夹/标签/笔记 ID 过滤：

### 后端（Rust）
1. 在 `search.rs` 中的搜索函数增加可选参数 `scope_filter`：
   - `Folder(String)` — 只搜索指定文件夹内的笔记
   - `Tag(String)` — 只搜索包含指定标签的笔记
   - `Note(Vec<String>)` — 只搜索指定笔记
   - `Union(Vec<ScopeFilter>)` — 多条件并集
2. 在 `prompting.rs` 中为 system prompt 注入当前 scope 描述
3. 在 `agent.rs` 中新增 `search_notes_scoped` tool，接受 scope 参数

### 前端（WinUI + Mobile）
1. 在 chat input 中实现 `@` 触发器：
   - 输入 `@` 后弹出候选列表（文件夹名优先，文件名+标签名混合）
   - 选中后显示为一个 chip/badge（如 `@folder:项目A`）
   - 支持多个 `@` chip 组合（自动取并集）
2. 在 chat 消息中可视化显示当前 scope 过滤条件

### 协议层
- JSON-RPC 新方法 `search_notes_scoped(query, scope_filter)`
- 向后兼容：scope 为空时维持现有全局搜索行为

## 优先级
**P2** — 重要但非阻塞。功能差距明显，但目前有全局搜索替代方案。

**理由**：这是 Obsidian Copilot 用户评价最高的功能之一，对 vault 规模增长后提升搜索精准度至关重要。

## 预期影响
- 用户在 500+ 笔记的 vault 中搜索准确度提升 40-60%
- 支持"项目级知识库"场景——不同项目/客户分文件夹管理
- 为后续 Projects Mode（专注工作区）打下基础
- 工程成本：后端 ~3 天，前端（双端）~4 天，测试 ~1 天
