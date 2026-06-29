## 竞品参考

Obsidian Copilot Plus 提供 `@websearch` 和 `@youtube` 工具：
- **@websearch**：让 AI 执行实时网络搜索，结果作为 RAG 上下文注入
- **@youtube**：提取 YouTube 视频转录文本，用于摘要和问答
- **URL 上下文**：直接在聊天中拖入 URL，AI 自动提取内容

Obsidian Copilot v4 的 Agent Mode 进一步整合了这些能力，agent 可以在需要时自主执行网络搜索。用户反馈"@web 是 Plus 最值得付费的功能，让知识库不再是孤岛"。

Mem.ai 提供 Chrome 扩展一键保存网页，以及 Claude Connector 让外部 LLM 可以访问笔记上下文。

Notion AI 2026 版的 Agent 功能也包含了自主网页搜索能力。

## 差距分析

**VaultPilot 当前状态**：VaultPilot 的知识检索完全局限在本地 vault 内。AI agent 的搜索工具 `search_notes` 只搜索本地索引。用户无法：
1. 让 AI 搜索互联网获取最新信息（如最新 API 文档、技术规范、市场价格）
2. 让 AI 读取网页链接内容
3. 在聊天中获取 YouTube 视频摘要
4. 将网页内容与本地笔记结合进行综合分析

**竞品做法**：Obsidian Copilot 在 Plus 订阅中提供 `@websearch` 工具，该工具调用后端搜索 API（可能是 Tavily/SerpAPI 等），将搜索结果格式化为文本注入到 AI 的上下文中。Plus 模式下的 agent 可以自主决定何时需要 web search。

**差距**：VaultPilot 的知识库是完全封闭的，对于需要实时信息的问题（如"最新的 Linux 内核 6.12 特性"），AI 只能依赖训练数据中的过时知识。

## 建议方案

### 后端（Rust）新增工具

1. **`web_search(query: String, max_results: u8)` tool**
   - 可选搜索引擎：可配置 Tavily API、SerpAPI 或自定义搜索引擎
   - 搜索结果结构化格式化（标题+摘要+URL）注入 AI 上下文
   - 受配置开关控制——用户可选择是否启用

2. **`read_url(url: String)` tool**
   - 使用 reqwest 获取 URL 内容
   - 智能提取文章正文（支持 HTML→Markdown 转换）
   - 50KB 上限防 OOM

3. **`read_youtube(url: String)` tool**
   - 提取 YouTube 视频字幕/转录文本
   - 支持 YouTube API 或 yt-dlp 式提取

### 配置层
- `settings.rs` 新增 `web_search_enabled: bool`、`web_search_provider: enum`（None/Tavily/SerpAPI/自建）、`web_search_api_key` 字段
- `AgentConfig` 新增 `allow_web_search: bool` 控制 agent 是否可自主调用 web search

### 前端（WinUI + Mobile）
1. Settings 页面新增 "Web Search" 配置区
2. Chat 中当使用 @web 时自动触发 web_search tool
3. Agent 使用 web_search 时在 chat 中显示来源标记 🌐

### 安全考虑
- Web search 仅在用户明确配置 API key 后才启用
- 禁止 web search 访问内网地址（SSRF 防护——复用现有 `base_url` SSRF 检查）
- 所有 HTTP 请求走现有 reqwest client 的 timeout 和 size limit 机制

## 优先级
**P2** — 重要功能，但不是 MVP blocker。

**理由**：面向工程师的产品中，"查最新技术文档+本地笔记"是非常自然的场景。没有 web search 的 AI 知识助手在 2026 年显得过于封闭。建议在 Agent Mode 稳定后立即启动。

## 预期影响
- 覆盖"最新技术查询+本地笔记"混合场景
- 提升 AI 回答的时效性（不再依赖训练截止日期）
- 为 VaultPilot 增加"联网知识库"产品卖点
- 工程成本：后端 ~4 天，前端（双端）~3 天，文档 ~0.5 天
