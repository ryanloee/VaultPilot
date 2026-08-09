# Usage Guide

## English

VaultPilot includes a Rust backend (`vaultpilot-cli` / `vaultpilot-agent`), a
cross-platform CLI, an Android mobile app, and a Windows desktop frontend
under development (`desktop/`, Tauri). The legacy WinUI client has been
removed.

### Main usage modes

- CLI usage
- local chat bridge usage (HTTP/SSE, MCP)
- desktop app usage (Tauri, once published)

### First-time setup (CLI)

Configure the CLI with:

- knowledge vault directory
- API key
- base URL
- model name
- request timeout

### Basic workflow

1. Set your vault directory.
2. Import Markdown notes or start recording notes directly.
3. Rebuild the index if needed.
4. Ask questions in chat.
5. Attach images when needed.
6. The app searches local notes and generates grounded answers.

### CLI and local bridge

The project includes:

- `vaultpilot-agent`
- `vaultpilot-cli`

The CLI can be used for local integration scenarios, and the backend can
expose a local chat bridge (HTTP/SSE) and an MCP server for agent-style access.

For Linux CLI builds, the main binary is:

- `vaultpilot-cli`

## 中文

VaultPilot 包含 Rust 后端（`vaultpilot-cli` / `vaultpilot-agent`）、跨平台 CLI、
Android 移动端，以及开发中的 Windows 桌面前端（`desktop/`，Tauri）。
旧版 WinUI 客户端已删除。

### 主要使用方式

- CLI 使用
- 本地对话桥接使用（HTTP/SSE、MCP）
- 桌面软件使用（Tauri，发布后）

### 首次配置（CLI）

需要配置：

- 知识库目录
- API Key
- 接口地址
- 模型名称
- 请求超时

### 基本使用流程

1. 设置知识库目录。
2. 导入 Markdown，或者直接开始记录笔记。
3. 必要时重建索引。
4. 在聊天框里提问。
5. 需要时附加图片。
6. 软件会先检索本地笔记，再生成带依据的回答。

### CLI 与本地桥

项目包含：

- `vaultpilot-agent`
- `vaultpilot-cli`

其中 CLI 可用于本地集成场景，后端也可提供本地对话桥（HTTP/SSE）和 MCP
服务，供外部 Agent 接入。

对于 Linux CLI 构建，主要可执行文件是：

- `vaultpilot-cli`
