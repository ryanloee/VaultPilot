<p align="center">
  <img src="assets/icon.png" alt="VaultPilot" width="96" height="96" />
</p>

<h1 align="center">VaultPilot</h1>

<p align="center">
  <strong>Local-first AI knowledge assistant for engineers</strong>
</p>

<p align="center">
  Record, search, and organize engineering notes — powered by local indexing and grounded AI Q&A.
</p>

<p align="center">
  <img src="https://img.shields.io/badge/platform-Windows%2010%2B-0078D4" alt="Windows" />
  <img src="https://img.shields.io/badge/platform-Linux-FCC624" alt="Linux" />
  <img src="https://img.shields.io/badge/platform-Android-3DDC84" alt="Android" />
  <img src="https://img.shields.io/badge/rust-2021-orange" alt="Rust" />
  <img src="https://img.shields.io/badge/React-19-61DAFB" alt="React" />
  <img src="https://img.shields.io/badge/license-MIT-green" alt="MIT License" />
</p>

---

## Why VaultPilot?

Engineering teams accumulate scattered notes — boot logs, pin mux tables, flash commands, board bring-up checklists. Traditional wikis are too heavy. Plain folders lack search. VaultPilot gives you a lightweight, offline-first vault where every note is a Markdown file, and every question gets an answer backed by your own data.

## Key Features

| Feature | Description |
|---------|-------------|
| **Grounded Q&A** | Ask in natural language. VaultPilot searches your vault, feeds context to AI, and returns answers with source citations. |
| **Full-Text Search** | SQLite FTS5 index with CJK-aware tokenization, synonym expansion, and multi-signal ranking. |
| **Note Management** | Markdown notes with structured frontmatter: tags, keywords, platform, board, kernel, status. |
| **Image Intelligence** | OCR text extraction, perceptual hashing for near-duplicate detection, and semantic similarity for image-based search. |
| **Conversation Memory** | Multi-session chat with automatic context compression when conversations get long. |
| **AI Tool Use** | The agent can search notes, read files, list directories, and save notes — all grounded in your local vault. |
| **Agent Mode** | Autonomous multi-step tool-calling loop — the AI plans, executes tools, and iterates until the task is done. |
| **Markdown Import** | Bulk-import existing `.md` files into the indexed vault. |
| **Offline-First** | Notes and search work without a network. AI features only need an API key. |

## Architecture

```
┌───────────────────────────────────────────────────────────────┐
│  Tauri v2 app shell (desktop/ + desktop/src-tauri/)           │
│  Windows · Linux · Android 共用同一套 React UI                │
│  ┌─────────────────────────────────────────────────────────┐  │
│  │  React 前端 (Tailwind + Zustand)                          │  │
│  │  WebView2 (Win) / WebKitGTK (Linux) / WebView (Android) │  │
│  └──────────────────────┬──────────────────────────────────┘  │
│                         │ invoke('command') 零 IPC            │
│  ┌──────────────────────▼──────────────────────────────────┐  │
│  │  Tauri 后端 (Rust, #[tauri::command])                   │  │
│  │  直接依赖 vaultpilot_lib（无子进程）                     │  │
│  └──────────────────────┬──────────────────────────────────┘  │
└─────────────────────────┼─────────────────────────────────────┘
                          │
┌─────────────────────────▼─────────────────────────────────────┐
│  vaultpilot_lib (Rust) / vaultpilot-cli                       │
│  ai · agent · storage · knowledge_graph · feeds · mail        │
│  SQLite (FTS5) + Markdown                                     │
└───────────────────────────────────────────────────────────────┘
```

## Quick Start

### Prerequisites

- Windows 10 version 1809+ (10.0.17763+)
- For AI features: an API key (Anthropic, OpenAI-compatible, or any provider you configure)

### Linux CLI

Linux ships both the Tauri UI (deb / AppImage) and the CLI.

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

Main outputs:

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### Desktop UI (Windows / Linux)

The desktop UI is built with Tauri v2 + React. The same frontend also targets Android.

```bash
cd desktop
pnpm install
pnpm tauri dev          # development (opens app window)
pnpm tauri build        # production build
```

### Android

Android uses the same Tauri + React frontend:

```bash
cd desktop
pnpm install
pnpm tauri android init    # one-time native project generation
pnpm tauri android build --target aarch64-linux-android --apk
```

### Install

Download the latest installer from [Releases](https://github.com/ryanloee/VaultPilot/releases):

- Windows: NSIS installer (`*-setup.exe`)
- Linux: `.deb` / `.AppImage`
- Android: APK

### First Run

1. Launch VaultPilot
2. Open **Settings** and configure your vault directory and API key
3. Import existing Markdown notes or start writing new ones
4. Ask questions in the chat

### Agent Mode (CLI)

VaultPilot's Agent Mode lets you run an autonomous AI agent that plans, executes tools, and iterates until your task is complete.

```bash
# Basic usage — agent will search your vault, read files, and answer
vaultpilot agent "Summarize my notes from this week"

# Limit the number of tool-calling steps (default: 20)
vaultpilot agent --max-steps 10 "Find all TODO items in my vault"

# Auto-approve write operations (⚠️ use with caution)
vaultpilot agent --auto-approve "Create a daily note for today"
```

**How it works:**
1. The agent receives your prompt and plans a strategy
2. It calls tools (search_notes, read_file, list_directory, save_note) to gather information
3. Results are fed back to the LLM for the next step
4. The loop continues until the agent produces a final answer or hits a limit

**Available tools:**
| Tool | Description |
|------|-------------|
| `search_notes` | Full-text search across your vault |
| `read_file` | Read a specific file (capped at 50KB) |
| `list_directory` | List files in a directory |
| `list_notes` | List recent notes |
| `save_note` | Save a new note (requires approval) |

**Safety features:**
- 🔒 **Vault-scoped**: All file operations are confined to your vault directory
- 🛡️ **Read-only by default**: Write operations require explicit approval
- 📋 **Audit log**: Every tool call is logged for review
- ⏱️ **Resource limits**: 5-minute timeout, 100 tool calls max, configurable step limit

> ⚠️ **Warning**: The `--auto-approve` flag skips write confirmation. Only use it for trusted tasks.

**MCP Server Integration:**

External AI agents (Claude Code, Codex, etc.) can also interact with your vault via the MCP protocol:

```bash
# Start MCP stdio server (for local agent integration)
vaultpilot mcp

# Start MCP HTTP server (for remote agent integration)
vaultpilot mcp-http --token YOUR_SECRET_TOKEN
```

## CLI Command Reference

VaultPilot's CLI (`vaultpilot` or `vp`) exposes 50+ top-level commands covering note management, AI features, search, configuration, and integrations. Commands are grouped by functional area below.

| Category | Command | Description |
|----------|---------|-------------|
| **Vault & Notes** | `init` | Initialize storage and show resolved settings |
| | `notes` | CRUD note operations: list, show, create, update, delete, search, import |
| | `daily` | Open or create today's daily note with optional template |
| | `capture` | Quick-capture a one-line note (append to daily note or inbox) |
| | `clip` | Clip a web page URL into a Markdown vault note |
| | `mirror` | Real-time mirror SQLite vault to Markdown files on disk |
| **AI & Agent** | `chat` | Interactive chat sessions with persisted state |
| | `agent` | Autonomous multi-step AI agent (tool-calling loop) |
| | `ask` | One-shot Q&A against your vault (no chat persistence) |
| | `ai` | AI quick actions: summarize, rewrite, translate, explain, etc. |
| | `deep-research` | Multi-round research with citations, saves result as a note |
| | `write` | AI-powered writing assistance (write, edit, expand, summarize) |
| | `edit` | Edit a note via natural-language instruction (with preview) |
| | `revert-edit` | Revert the last AI-applied edit to a note |
| | `table` | Generate AI-powered Markdown comparison tables |
| | `compress` | Compress long chat history into a summary |
| | `digest` | Daily knowledge digest of recently changed notes |
| **Search & Discovery** | `search` | Full-text search across the vault (via `notes search`) |
| | `ask` | Natural-language Q&A grounded in vault content |
| | `serendipity` | Discover forgotten notes scored against recent activity |
| | `graph` | Generate knowledge graph from `[[wikilink]]` references |
| | `context-surface` | Real-time "relevant notes" for text you are editing |
| **Configuration** | `config` | View/edit vault-facing configuration (vault root, `.vaultpilot/` layout) |
| | `settings` | View/update raw JSON settings |
| | `prompt` | Manage system prompt templates stored in `.vaultpilot/prompts/` |
| **Collections & Projects** | `collections` | Multi-group notes across projects |
| | `project` | Isolated knowledge spaces with independent contexts |
| **Index & Storage** | `index` | Manage search index (rebuild, stats) |
| | `vault` | Vault operations: export zip, backup |
| **Automation** | `subscriptions` | Manage AI scheduled research subscriptions |
| | `trigger` | Agent trigger rules (events + cron schedules) |
| | `organize` | Self-organizing vault: auto-link, categorize, suggest collections |
| **Integration** | `serve` | Start a local chat-completions HTTP bridge (OpenAI-compatible endpoint) |
| | `mcp` | Start MCP stdio server for local agent integration |
| | `mcp-http` | Start MCP HTTP server with optional token auth |
| | `feed` | Manage RSS/Atom/JSON Feed subscriptions → auto-ingest as notes |
| | `mail` | Email-to-Vault integration: sync IMAP emails into vault |
| | `connector` | List/manage external service connectors |
| **People & Context** | `people` | People-aware context: index notes by person name, manage aliases |
| | `calendar` | Render vault notes on a month-grid calendar by frontmatter dates |
| | `canvas` | Inspect Obsidian-compatible `.canvas` whiteboard files |
| | `health` | Vault health dashboard: note counts, orphan analysis, suggestions |
| | `changelog` | Show recent vault changes grouped by date |
| **Skills & Knowledge** | `skill` | Run built-in knowledge-work skills (summarize, weekly-review, etc.) |
| | `skill-saved` | Manage and invoke user-saved AI skills (custom commands) |
| **Learning** | `flashcard` | Manage spaced-repetition flashcards |
| | `review` | Run FSRS spaced-repetition reviews |
| **Media** | `voice` | Voice note capture: transcribe audio → save as vault note |
| | `meeting` | Transcribe meeting audio and generate structured AI summary |
| | `pdf` | Extract text content from PDF files |
| | `present` | Preview a note as a reveal.js slide presentation |
| **Utilities** | `diff` | Compute line-level diff between two notes |
| | `publish` | Publish a Markdown note as a self-contained HTML page |
| | `completions` | Generate shell completion scripts (bash/zsh/fish/powershell) |
| | `plugins` | List registered plugins |
| | `agent-engine` | Manage external agent engines (Claude Code / Codex adapter) |

### Shell Completions

VaultPilot ships with a built-in `completions` command that generates shell completion scripts for **bash**, **zsh**, **fish**, and **PowerShell**:

```bash
# Print completion script
vp completions bash
vp completions zsh
vp completions fish
vp completions powershell

# Source in your shell init
eval "$(vp completions bash)"          # bash
source <(vp completions zsh)          # zsh
vp completions fish | source          # fish
vp completions powershell | Out-String | Invoke-Expression  # PowerShell
```

The generated completions include static completions for all subcommands and flags, plus dynamic completions for `vp skill-saved` that query the database for saved skill IDs.

### Build from Source

See [docs/build.md](docs/build.md) for detailed build instructions.

```powershell
# Quick build (requires Rust + Node.js + pnpm)
cd desktop
pnpm tauri build
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Frontend (Windows/Linux) | Tauri v2 + React 19 + TypeScript + Tailwind + Zustand |
| Mobile Frontend (Android) | Tauri v2 + React（同一套代码） |
| Backend | Rust (Tokio, Axum, Reqwest) |
| Storage | SQLite (FTS5) + Markdown files |
| AI | Anthropic Messages API with tool use + Agent Mode |
| Packaging | Tauri bundler (NSIS / deb / AppImage / APK) |

## Documentation

- [Usage Guide](docs/usage.md)
- [Build Guide](docs/build.md)

## Contributing

Contributions are welcome! Feel free to open issues or pull requests.

1. Fork the repository
2. Create your feature branch
3. Make your changes
4. Submit a pull request

## License

This project is licensed under the **MIT License** — see [LICENSE](LICENSE) for details.

### Third-Party Licenses

VaultPilot uses the following open-source libraries:

| Library | License |
|---------|---------|
| anyhow | MIT / Apache-2.0 |
| axum | MIT |
| base64 | MIT / Apache-2.0 |
| chrono | MIT / Apache-2.0 |
| clap | MIT / Apache-2.0 |
| deunicode | BSD-3-Clause |
| image | MIT / Apache-2.0 |
| reqwest | MIT / Apache-2.0 |
| rusqlite | MIT |
| serde / serde_json | MIT / Apache-2.0 |
| serde_yml | MIT / Apache-2.0 |
| sha2 | MIT / Apache-2.0 |
| tokio | MIT |
| uuid | MIT / Apache-2.0 |
| walkdir | MIT / Unlicense |
| Microsoft.WindowsAppSDK | MIT |
| Velopack | MIT |

---

<p align="center">
  <strong>中文说明</strong>
</p>

## VaultPilot 是什么？

VaultPilot 是一个面向工程师的**本地优先 AI 知识助手**。帮助你把散落在各处的工程笔记（启动日志、引脚配置、刷机命令、板卡调试记录...）统一管理，并通过自然语言提问获得有据可依的 AI 回答。

## 核心功能

- **有据可依的 AI 问答** — 用自然语言提问，VaultPilot 会先检索你的本地笔记库，再让 AI 基于这些笔记生成回答，并附上引用来源
- **Agent 模式** — AI 自主执行多步工具调用循环：规划、执行工具、迭代直到完成任务
- **全文搜索** — SQLite FTS5 索引，支持中文分词、同义词扩展和多信号排序
- **结构化笔记管理** — Markdown 文件 + 元数据（标签、关键词、平台、板卡、内核、状态）
- **图片智能检索** — OCR 文字提取、感知哈希去重、语义相似度匹配
- **多会话记忆** — 支持多个独立聊天会话，长对话自动压缩上下文
- **AI 工具调用** — AI 可以搜索笔记、读取文件、列出目录、保存笔记
- **Markdown 批量导入** — 一键导入现有 `.md` 文件到知识库
- **离线可用** — 笔记管理和搜索完全离线，AI 功能仅需配置 API Key

## 快速开始

### 系统要求

- Windows 10 1809 及以上版本
- AI 功能需要配置 API Key（支持 Anthropic、OpenAI 兼容等）

### Linux CLI

Linux 端同时提供 Tauri 图形界面（deb / AppImage）和 CLI。

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

主要产物：

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### 桌面 UI（Windows / Linux）

桌面 UI 使用 Tauri v2 + React 构建，同一套前端也用于 Android。

```bash
cd desktop
pnpm install
pnpm tauri dev          # 开发模式（打开应用窗口）
pnpm tauri build        # 生产构建
```

### Android

Android 使用与桌面端相同的 Tauri + React 前端：

```bash
cd desktop
pnpm install
pnpm tauri android init    # 一次性生成原生工程
pnpm tauri android build --target aarch64-linux-android --apk
```

### 安装

从 [Releases](https://github.com/ryanloee/VaultPilot/releases) 下载最新版本：

- Windows：NSIS 安装包（`*-setup.exe`）
- Linux：`.deb` / `.AppImage`
- Android：APK

### 使用流程

1. 启动 VaultPilot
2. 在设置中配置知识库目录和 API Key
3. 导入现有 Markdown 笔记，或直接开始记录
4. 在聊天框中提问

### Agent 模式（CLI）

VaultPilot 的 Agent 模式让 AI 自主执行多步工具调用循环：规划、执行工具、迭代直到完成任务。

```bash
# 基本用法 — Agent 会搜索笔记库、读取文件、回答问题
vaultpilot agent "总结我这周的笔记"

# 限制工具调用步骤数（默认：20）
vaultpilot agent --max-steps 10 "找到我笔记库中所有的 TODO"

# 自动批准写入操作（⚠️ 谨慎使用）
vaultpilot agent --auto-approve "创建今天的日记"
```

**工作流程：**
1. Agent 接收你的提示并制定策略
2. 调用工具（search_notes、read_file、list_directory、save_note）收集信息
3. 将结果反馈给 LLM 进行下一步
4. 循环继续直到 Agent 给出最终答案或达到限制

**可用工具：**
| 工具 | 描述 |
|------|------|
| `search_notes` | 全文搜索笔记库 |
| `read_file` | 读取指定文件（上限 50KB） |
| `list_directory` | 列出目录中的文件 |
| `list_notes` | 列出最近的笔记 |
| `save_note` | 保存新笔记（需要批准） |

**安全特性：**
- 🔒 **Vault 范围限制**：所有文件操作限制在笔记库目录内
- 🛡️ **默认只读**：写入操作需要显式批准
- 📋 **审计日志**：每次工具调用都有记录
- ⏱️ **资源限制**：5 分钟超时、最多 100 次工具调用、可配置步骤限制

> ⚠️ **警告**：`--auto-approve` 跳过写入确认，仅用于可信任务。

### 从源码构建

详细说明请参考 [构建指南](docs/build.md)。

```powershell
cd desktop
pnpm tauri build
```

## 技术栈

| 层级 | 技术 |
|------|------|
| 桌面前端（Windows/Linux） | Tauri v2 + React 19 + TypeScript + Tailwind + Zustand |
| 移动端前端（Android） | Tauri v2 + React（同一套代码） |
| 后端 | Rust (Tokio, Axum, Reqwest) |
| 存储 | SQLite (FTS5) + Markdown 文件 |
| AI | Anthropic Messages API (工具调用 + Agent 模式) |
| 打包 | Tauri bundler (NSIS / deb / AppImage / APK) |

## 许可证

本项目基于 **MIT 许可证** 开源，详见 [LICENSE](LICENSE)。
