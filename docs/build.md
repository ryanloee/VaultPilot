# Build Guide

## English

This project currently has:

- Windows desktop UI build (Tauri v2 + React)
- Linux desktop UI build (Tauri v2 + React)
- Android UI build (Tauri v2 + React, same frontend)
- Linux CLI build

### Prerequisites

- Node.js 20+ with pnpm (or run `corepack enable`)
- Rust toolchain with `rustup`
- Windows: WebView2 runtime (preinstalled on Windows 10/11)
- Linux UI: `libwebkit2gtk-4.1-dev`, GTK3, `libayatana-appindicator3-dev`, `librsvg2-dev`
- Android: JDK 17, Android SDK + NDK, Rust targets (`aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`)

### Desktop UI (Windows / Linux) — local development

```powershell
cd desktop
pnpm install
pnpm tauri dev
```

`pnpm tauri dev` starts the Vite dev server, compiles the Tauri Rust shell
(which depends directly on `vaultpilot_lib`), and opens the app window.

### Desktop UI — production build

```powershell
cd desktop
pnpm tauri build
```

Windows installer (NSIS) output:

- `desktop/src-tauri/target/<target>/release/bundle/nsis/*-setup.exe`

Linux package (deb / AppImage) output:

- `desktop/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/*.deb`
- `desktop/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/*.AppImage`

### Android

One-time native project generation:

```bash
cd desktop
pnpm tauri android init
```

Build APK for a target:

```bash
cd desktop
pnpm tauri android build --target aarch64-linux-android --apk
```

APK output:

- `desktop/src-tauri/gen/android/app/build/outputs/apk/*/release/*.apk`

### Linux CLI

The CLI remains a standalone Rust binary (no UI dependency).

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

Main outputs:

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### Important note

These directories are build outputs and should not be committed:

- `target/`
- `artifacts/`
- `desktop/node_modules/`
- `desktop/dist/`
- `desktop/src-tauri/target/`

## 中文

当前项目包含以下构建路径：

- Windows 桌面 UI（Tauri v2 + React）
- Linux 桌面 UI（Tauri v2 + React）
- Android UI（Tauri v2 + React，同一套前端）
- Linux CLI

### 环境要求

- Node.js 20+，使用 pnpm（或执行 `corepack enable`）
- Rust 工具链和 `rustup`
- Windows：WebView2 运行时（Windows 10/11 自带）
- Linux UI：`libwebkit2gtk-4.1-dev`、GTK3、`libayatana-appindicator3-dev`、`librsvg2-dev`
- Android：JDK 17、Android SDK + NDK、Rust targets（`aarch64-linux-android`、`armv7-linux-androideabi`、`x86_64-linux-android`）

### 桌面 UI（Windows / Linux）本地开发

```powershell
cd desktop
pnpm install
pnpm tauri dev
```

`pnpm tauri dev` 会启动 Vite 开发服务器、编译 Tauri Rust 壳（直接依赖
`vaultpilot_lib`），并打开应用窗口。

### 桌面 UI 生产构建

```powershell
cd desktop
pnpm tauri build
```

Windows 安装包（NSIS）产物：

- `desktop/src-tauri/target/<target>/release/bundle/nsis/*-setup.exe`

Linux 包（deb / AppImage）产物：

- `desktop/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/deb/*.deb`
- `desktop/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/*.AppImage`

### Android

一次性生成原生工程：

```bash
cd desktop
pnpm tauri android init
```

构建 APK：

```bash
cd desktop
pnpm tauri android build --target aarch64-linux-android --apk
```

APK 产物：

- `desktop/src-tauri/gen/android/app/build/outputs/apk/*/release/*.apk`

### Linux CLI

CLI 仍是独立的 Rust 可执行文件（不依赖 UI）。

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

主要产物：

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### 重要说明

以下目录都是构建产物，不应该提交到仓库：

- `target/`
- `artifacts/`
- `desktop/node_modules/`
- `desktop/dist/`
- `desktop/src-tauri/target/`
