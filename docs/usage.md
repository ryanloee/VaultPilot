# Usage Guide

## English

VaultPilot includes a Windows desktop app and a cross-platform CLI.  
The Linux build is CLI-only and does not include the WinUI frontend.

### Main usage modes

- desktop app usage
- installer package usage
- local chat bridge usage

### Start the desktop app from a local build

For local testing, run:

- `native/VaultPilot.WinUI/bin/x64/Debug/net8.0-windows10.0.19041.0/VaultPilot.WinUI.exe`

If you built Release instead, use the equivalent Release path.

### Start the installed app

After installing the package, the app usually runs from:

- `C:\Users\<YourUser>\AppData\Local\VaultPilot\current\VaultPilot.WinUI.exe`

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

If the app is installed through the Velopack installer, it can check GitHub releases for updates.  
If the app is launched only from a local build directory, automatic update behavior is not available in the same way.

### CLI and local bridge

This project also includes:

- `vaultpilot-agent.exe`
- `vaultpilot-cli.exe`

The CLI can be used for local integration scenarios, and the app can expose a local chat bridge for agent-style access.

For Linux CLI builds, the main binary is:

- `vaultpilot-cli`

## 中文

VaultPilot 包含 Windows 桌面应用和跨平台 CLI。  
Linux 版本只包含 CLI，不包含 WinUI 图形界面。

### 主要使用方式

- 桌面软件使用
- 安装包使用
- 本地对话桥接使用

### 从本地构建结果启动桌面程序

本地测试时，通常启动：

- `native/VaultPilot.WinUI/bin/x64/Debug/net8.0-windows10.0.19041.0/VaultPilot.WinUI.exe`

如果你构建的是 Release 版本，也可以直接运行 Release 输出。

### 启动安装版程序

安装完成后，程序通常位于：

- `C:\Users\<你的用户名>\AppData\Local\VaultPilot\current\VaultPilot.WinUI.exe`

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

如果软件是通过 Velopack 安装包安装的，它可以从 GitHub Release 检查更新。  
如果只是从本地构建目录直接运行，则不会以同样方式支持自动更新。

### CLI 与本地桥

项目还包含：

- `vaultpilot-agent.exe`
- `vaultpilot-cli.exe`

其中 CLI 可用于本地集成场景，软件也可以提供本地对话桥，供外部 Agent 通过对话方式接入。

对于 Linux CLI 构建，主要可执行文件是：

- `vaultpilot-cli`
