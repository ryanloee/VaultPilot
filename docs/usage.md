# Usage Guide

## English

VaultPilot includes a Tauri desktop app (Windows / Linux), an Android app, and a cross-platform CLI.  
Windows, Linux, and Android share the same React UI and Rust backend (`vaultpilot_lib`).

### Main usage modes

- desktop app usage
- installer package usage
- local chat bridge usage

### Start the desktop app from a local build

For local testing, run the Tauri dev server from `desktop/`:

```powershell
cd desktop
pnpm tauri dev
```

For a production build, run `pnpm tauri build` (see `docs/build.md`).

### Start the installed app

After installing the package, launch VaultPilot from the Start Menu (Windows),
the installed .deb / AppImage (Linux), or the APK (Android).

### First-time setup

Open Settings and configure:

- knowledge vault directory
- API key
- base URL
- model name
- request timeout
- optional context window tokens

You can also choose whether the app should check for updates at startup.

### Basic workflow

1. Set your vault directory.
2. Import Markdown notes or start recording notes directly.
3. Rebuild the index if needed.
4. Ask questions in chat.
5. Attach images when needed.
6. Let the app search local notes and generate grounded answers.

### Update behavior

Update behavior is delivered through the release channel (installer re-download /
app store). Local dev builds do not auto-update.

### CLI and local bridge

This project also includes:

- `vaultpilot-agent.exe`
- `vaultpilot-cli.exe`

The CLI can be used for local integration scenarios, and the app can expose a local chat bridge for agent-style access.

For Linux CLI builds, the main binary is:

- `vaultpilot-cli`

## 中文

VaultPilot 包含 Tauri 桌面应用（Windows / Linux）、Android 应用和跨平台 CLI。  
Windows、Linux、Android 共用同一套 React UI 和 Rust 后端（`vaultpilot_lib`）。

### 主要使用方式

- 桌面软件使用
- 安装包使用
- 本地对话桥接使用

### 从本地构建结果启动桌面程序

本地测试时，在 `desktop/` 下运行 Tauri 开发模式：

```powershell
cd desktop
pnpm tauri dev
```

生产构建请运行 `pnpm tauri build`（参见 `docs/build.md`）。

### 启动安装版程序

安装完成后，从开始菜单启动（Windows）、安装的 .deb / AppImage（Linux）或 APK（Android）。

### 首次配置

打开“设置”后需要配置：

- 知识库目录
- API Key
- 接口地址
- 模型名称
- 请求超时
- 可选的上下文窗口 Token 数

你也可以在设置里决定是否在启动时自动检查更新。

### 基本使用流程

1. 设置知识库目录。
2. 导入 Markdown，或者直接开始记录笔记。
3. 必要时重建索引。
4. 在聊天框里提问。
5. 需要时附加图片。
6. 软件会先检索本地笔记，再生成带依据的回答。

### 自动更新说明

更新通过发布渠道提供（重新下载安装包 / 应用商店）。本地开发构建不自动更新。

### CLI 与本地桥

项目还包含：

- `vaultpilot-agent.exe`
- `vaultpilot-cli.exe`

其中 CLI 可用于本地集成场景，软件也可以提供本地对话桥，供外部 Agent 通过对话方式接入。

对于 Linux CLI 构建，主要可执行文件是：

- `vaultpilot-cli`
