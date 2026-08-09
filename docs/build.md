# Build Guide

## English

This project currently has:

- Rust backend (`vaultpilot-cli` / `vaultpilot-agent` binaries + `vaultpilot_lib`)
- Linux CLI build (binary + `.deb`)
- Android mobile app (`mobile/`, Expo / React Native)
- Windows desktop frontend under development (`desktop/`, Tauri v2 + React)

The legacy WinUI client (`native/`) has been removed; Windows now uses the new
Tauri desktop UI once its sources are published to the repository.

### Prerequisites

- Rust toolchain with `rustup`
- For `.deb` packaging: `dpkg-deb` from `dpkg-dev`

### Build the Rust backend

```bash
cargo build --release --bins
```

Outputs:

- `target/release/vaultpilot-cli`
- `target/release/vaultpilot-agent`

### Build the Linux CLI

The Linux build is CLI-only. It does not include a desktop frontend.

Prerequisites:

- Linux with `bash`
- Rust toolchain with `rustup`
- for `x86` builds: either Zig + `cargo-zigbuild`, or a 32-bit GNU linker toolchain such as `gcc-multilib`
- for `.deb` packaging: `dpkg-deb` from `dpkg-dev`

Build an `x64` Linux executable and `.deb` package:

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

If your machine does not have `gcc-multilib`, install Zig and `cargo-zigbuild` instead.  
When both `zig` and `cargo-zigbuild` are present in `PATH`, the script uses Zig automatically.

Build only the executable:

```bash
./scripts/build-linux-cli.sh --platforms x64 --format bin
```

Optional `x86` build:

```bash
./scripts/build-linux-cli.sh --platforms x86 --format all
```

Main outputs:

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### Important note

These directories are build outputs and should not be committed:

- `target/`
- `artifacts/`

## 中文

当前项目包含：

- Rust 后端（`vaultpilot-cli` / `vaultpilot-agent` 可执行文件 + `vaultpilot_lib`）
- Linux CLI 构建（可执行文件 + `.deb`）
- Android 移动端（`mobile/`，Expo / React Native）
- Windows 桌面前端开发中（`desktop/`，Tauri v2 + React）

旧版 WinUI 客户端（`native/`）已删除，Windows 端将使用新的 Tauri 桌面 UI
（源码发布到仓库后）。

### 环境要求

- Rust 工具链和 `rustup`
- 若输出 `.deb`：需要 `dpkg-deb`，通常来自 `dpkg-dev`

### 构建 Rust 后端

```bash
cargo build --release --bins
```

产物：

- `target/release/vaultpilot-cli`
- `target/release/vaultpilot-agent`

### 构建 Linux CLI

Linux 版本只包含 CLI，不包含桌面图形界面。

环境要求：

- Linux + `bash`
- Rust 工具链和 `rustup`
- 若构建 `x86`：可使用 Zig + `cargo-zigbuild`，或者安装 32 位 GNU 链接工具链（如 `gcc-multilib`）
- 若输出 `.deb`：需要 `dpkg-deb`，通常来自 `dpkg-dev`

构建 `x64 Linux` 可执行文件和 `.deb` 包：

```bash
chmod +x ./scripts/build-linux-cli.sh
./scripts/build-linux-cli.sh --platforms x64 --format all
```

如果本机没有 `gcc-multilib`，也可以安装 Zig 和 `cargo-zigbuild`。  
当 `PATH` 中同时存在 `zig` 和 `cargo-zigbuild` 时，脚本会自动走 Zig 交叉编译。

只构建可执行文件：

```bash
./scripts/build-linux-cli.sh --platforms x64 --format bin
```

可选的 `x86` 构建：

```bash
./scripts/build-linux-cli.sh --platforms x86 --format all
```

主要产物：

- `artifacts/linux-cli/bin/linux-x64/vaultpilot-cli`
- `artifacts/linux-cli/packages/linux-x64/vaultpilot-cli_<version>_amd64.deb`

### 重要说明

以下目录都是构建产物，不应该提交到仓库：

- `target/`
- `artifacts/`
