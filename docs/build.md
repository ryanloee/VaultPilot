# Build Guide

## English

This project currently has:

- Windows desktop app build
- Windows installer package build
- Linux CLI build

### Prerequisites

- Windows 10 or later
- Rust toolchain with `rustup`
- .NET 8 SDK
- Visual Studio 2022 Build Tools with Windows App SDK support
- PowerShell

### Build the desktop app for local development

This builds the WinUI frontend and the Rust binaries used by the frontend.

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe" `
  ".\native\VaultPilot.WinUI\VaultPilot.WinUI.csproj" `
  /restore `
  /t:Build `
  /p:Configuration=Debug `
  /p:Platform=x64
```

Main local output:

- `native/VaultPilot.WinUI/bin/x64/Debug/net8.0-windows10.0.19041.0/VaultPilot.WinUI.exe`

### Build a release package

This is the normal packaging flow for installers and release artifacts.

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-windows-installers.ps1 -Platforms x64 -Version 0.1.4
```

You can also build both `x86` and `x64`:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-windows-installers.ps1 -Platforms x86,x64 -Version 0.1.4
```

Main package outputs:

- `artifacts/velopack/packages/win-x64/VaultPilot-win-x64-Setup.exe`
- `artifacts/velopack/packages/win-x64/VaultPilot-win-x64-Portable.zip`
- `artifacts/velopack/packages/win-x64/VaultPilot-0.1.4-win-x64-full.nupkg`

Intermediate publish directory:

- `artifacts/velopack/publish/win-x64/`

### Build flow summary

1. `build-windows-installers.ps1` reads the version from `Cargo.toml`.
2. The WinUI project builds first.
3. During WinUI build, the project runs `cargo build` for:
   - `vaultpilot-agent`
   - `vaultpilot-cli`
4. The Rust binaries are copied into the WinUI output directory.
5. The publish directory is assembled under `artifacts/velopack/publish/...`.
6. Velopack packs the final installer and release assets under `artifacts/velopack/packages/...`.

### Important note

These directories are build outputs and should not be committed:

- `target/`
- `artifacts/`
- `native/**/bin/`
- `native/**/obj/`

### Build the Linux CLI

The Linux build is CLI-only. It does not include the WinUI frontend.

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

## 中文

当前项目现在包含三条构建路径：

- Windows 桌面应用构建
- Windows 安装包构建
- Linux CLI 构建

### 环境要求

- Windows 10 或更高版本
- 已安装 Rust 工具链和 `rustup`
- .NET 8 SDK
- 带 Windows App SDK 支持的 Visual Studio 2022 Build Tools
- PowerShell

### 本地开发构建

这个流程会同时构建 WinUI 前端和前端依赖的 Rust 可执行文件。

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe" `
  ".\native\VaultPilot.WinUI\VaultPilot.WinUI.csproj" `
  /restore `
  /t:Build `
  /p:Configuration=Debug `
  /p:Platform=x64
```

主要本地产物：

- `native/VaultPilot.WinUI/bin/x64/Debug/net8.0-windows10.0.19041.0/VaultPilot.WinUI.exe`

### 构建正式安装包

这是正常的打包流程，会输出安装包和 Release 产物。

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-windows-installers.ps1 -Platforms x64 -Version 0.1.4
```

如果要同时打 `x86` 和 `x64`：

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\build-windows-installers.ps1 -Platforms x86,x64 -Version 0.1.4
```

主要打包产物：

- `artifacts/velopack/packages/win-x64/VaultPilot-win-x64-Setup.exe`
- `artifacts/velopack/packages/win-x64/VaultPilot-win-x64-Portable.zip`
- `artifacts/velopack/packages/win-x64/VaultPilot-0.1.4-win-x64-full.nupkg`

打包前的发布目录：

- `artifacts/velopack/publish/win-x64/`

### 编译流程概览

1. `build-windows-installers.ps1` 从 `Cargo.toml` 读取版本号。
2. 先构建 WinUI 工程。
3. WinUI 工程在构建过程中会调用 `cargo build`，生成：
   - `vaultpilot-agent`
   - `vaultpilot-cli`
4. 这两个 Rust 可执行文件会被复制到 WinUI 输出目录。
5. 脚本把发布目录整理到 `artifacts/velopack/publish/...`。
6. 最后用 Velopack 生成安装包和 Release 产物到 `artifacts/velopack/packages/...`。

### 重要说明

以下目录都是构建产物，不应该提交到仓库：

- `target/`
- `artifacts/`
- `native/**/bin/`
- `native/**/obj/`

### 构建 Linux CLI

Linux 版本只包含 CLI，不包含 WinUI 图形界面。

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
