## 竞品参考

Obsidian Copilot 使用开源的 `obsidian-copilot` 插件实现，2025 年更新 v3.3.3 版本，2026 年夏季发布 v4 预览版。最新动态：

### v4 预览版（2026 夏季）
- **Agent Mode 重构**：支持 opencode、Claude Code、Codex 多种 agent 后端，用户可切换
- **Projects Mode**：缓存增强生成（Cache-Augmented Generation），支持 PDF/DOCX/EPUB 等 50+ 文件格式
- **Composer**：通过 `@composer` 在聊天中交互式编辑笔记（Accept/Reject 模式）
- **@ 工具体系**：`@folder`、`@note`、`@tag`、`@websearch`、`@youtube`、`@web` 等上下文限定
- **离线模式**：支持 Ollama/LM Studio 本地模型，数据不离开设备
- **审批工作流**：agent 所有写操作先展示 diff，用户批准后才执行
- **定价**：免费（BYOK）+ Plus $11.67/月（年付）+ Supporter $349.99/生命期

### 用户反馈
- 正面：@ 限定搜索范围是 killer feature，多 provider 支持，开放架构
- 负面：@web 需要 Plus 订阅，免费版功能受限，学习曲线偏高

## 差距分析

| 功能 | VaultPilot | Obsidian Copilot | Mem.ai |
|------|-----------|-----------------|--------|
| 聊天中 @ 限定搜索范围 | ❌ | ✅ @folder/@note/@tag | ❌（自动组织） |
| 网络搜索（@websearch） | ❌ | ✅ Plus 付费 | ✅ Chrome 扩展 |
| Composer 交互式编辑 | ❌ | ✅ Plus | ❌ |
| Projects 工作区 | ❌ | ✅ Plus | ❌（Collections） |
| 多 Provider 支持 | ⚠️ 仅 Anthropic + OpenAI | ✅ 10+ providers | ✅ Claude/Gemini/GPT |
| 日历集成 | ❌ | ❌ | ✅ Google/Outlook |
| 会议转录 | ❌ | ❌ | ✅ Voice Mode |
| 本地模型（Ollama/LM Studio） | ❌ | ✅ | ❌ |
| Web Chrome 扩展 | ❌ | ❌ | ✅ |
| Agent 多后端切换 | ❌ | ✅ v4（opencode/Claude Code/Codex） | ❌ |
| Write Approval | ✅ WriteApprovalDialog | ✅ v4 diff preview | ❌ |
| MCP Server | ✅ stdio + HTTP | ✅ MCP support | ❌（API only） |
| 移动端 | ✅ Android Expo | ⚠️ Obsidian 移动端 | ✅ iOS + Android |

