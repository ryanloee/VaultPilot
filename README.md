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
  <img src="https://img.shields.io/badge/linux-CLI%20only-FCC624" alt="Linux CLI" />
  <img src="https://img.shields.io/badge/rust-2021-orange" alt="Rust" />
  <img src="https://img.shields.io/badge/.NET-8-512BD4" alt=".NET 8" />
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
| **AI Tool Use** | The agent can search notes, read files, list directories, run commands, and save notes — all grounded in your local vault. |
| **Markdown Import** | Bulk-import existing `.md` files into the indexed vault. |
| **Offline-First** | Notes and search work without a network. AI features only need an API key. |

## Architecture

```
┌──────────────────────────────┐
│  VaultPilot.WinUI.exe (C#)   │  WinUI 3 desktop shell
│  ┌──────────┐ ┌────────────┐ │
│  │ Chat UI  │ │ Settings   │ │
│  └────┬─────┘ └────────────┘ │
│       │ JSON-RPC (stdin/out) │
│  ┌────▼──────────────────┐   │
│  │  BackendClient.cs     │   │
│  └────┬──────────────────┘   │
└───────┼──────────────────────┘
        │
┌───────▼──────────────────────┐
│  vaultpilot-agent.exe (Rust) │
│  ┌────────┐ ┌──────┐        │
│  │ ai.rs  │ │storage│        │
│  └────────┘ └──┬───┘        │
│  ┌────────┐    │             │
│  │prompt. │    │             │
│  │  rs    │    ▼             │
│  └────────┘  SQLite + .md   │
└──────────────────────────────┘
```

## Quick Start

### Prerequisites

- Windows 10 version 1809+ (10.0.17763+)
- For AI features: an API key (Anthropic, OpenAI-compatible, or any provider you configure)

### Linux CLI

The Linux build is CLI-only and does not include the WinUI desktop frontend.

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

Main outputs:

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### Install

Download the latest installer from [Releases](https://github.com/ryanloee/VaultPilot/releases):

- `VaultPilot-win-x64-Setup.exe` — installer with auto-update
- `VaultPilot-win-x64-Portable.zip` — portable, no install needed

### First Run

1. Launch VaultPilot
2. Open **Settings** and configure your vault directory and API key
3. Import existing Markdown notes or start writing new ones
4. Ask questions in the chat

### Build from Source

See [docs/build.md](docs/build.md) for detailed build instructions.

```powershell
# Quick build (requires Rust + .NET 8 SDK + VS Build Tools)
dotnet build native/VaultPilot.WinUI/VaultPilot.WinUI.csproj -p:Platform=x64
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Frontend | WinUI 3 / .NET 8 |
| Backend | Rust (Tokio, Axum, Reqwest) |
| Storage | SQLite (FTS5) + Markdown files |
| AI | Anthropic Messages API with tool use |
| Packaging | Velopack (auto-update, x86/x64) |

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
| serde_yaml | MIT / Apache-2.0 |
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
- **全文搜索** — SQLite FTS5 索引，支持中文分词、同义词扩展和多信号排序
- **结构化笔记管理** — Markdown 文件 + 元数据（标签、关键词、平台、板卡、内核、状态）
- **图片智能检索** — OCR 文字提取、感知哈希去重、语义相似度匹配
- **多会话记忆** — 支持多个独立聊天会话，长对话自动压缩上下文
- **AI 工具调用** — AI 可以搜索笔记、读取文件、列出目录、执行命令、保存笔记
- **Markdown 批量导入** — 一键导入现有 `.md` 文件到知识库
- **离线可用** — 笔记管理和搜索完全离线，AI 功能仅需配置 API Key

## 快速开始

### 系统要求

- Windows 10 1809 及以上版本
- AI 功能需要配置 API Key（支持 Anthropic、OpenAI 兼容等）

### Linux CLI

Linux 版本只包含 CLI，不包含 WinUI 图形界面。

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

主要产物：

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### 安装

从 [Releases](https://github.com/ryanloee/VaultPilot/releases) 下载最新版本：

- `VaultPilot-win-x64-Setup.exe` — 安装版，支持自动更新
- `VaultPilot-win-x64-Portable.zip` — 便携版，解压即用

### 使用流程

1. 启动 VaultPilot
2. 在设置中配置知识库目录和 API Key
3. 导入现有 Markdown 笔记，或直接开始记录
4. 在聊天框中提问

### 从源码构建

详细说明请参考 [构建指南](docs/build.md)。

```powershell
dotnet build native/VaultPilot.WinUI/VaultPilot.WinUI.csproj -p:Platform=x64
```

## 技术栈

| 层级 | 技术 |
|------|------|
| 前端 | WinUI 3 / .NET 8 |
| 后端 | Rust (Tokio, Axum, Reqwest) |
| 存储 | SQLite (FTS5) + Markdown 文件 |
| AI | Anthropic Messages API (工具调用) |
| 打包 | Velopack (自动更新, x86/x64) |

## 许可证

本项目基于 **MIT 许可证** 开源，详见 [LICENSE](LICENSE)。
