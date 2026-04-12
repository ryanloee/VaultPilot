# Cleanup Guide

## English

This project generates local build outputs during development and packaging.  
You can safely remove them before packaging again or before checking git status.

### Common generated directories

- `target/`
- `artifacts/`
- `native/VaultPilot.WinUI/bin/`
- `native/VaultPilot.WinUI/obj/`
- `packaging/windows/Output/`
- temporary folders such as `tmp-icons/`

### Manual cleanup

```powershell
Remove-Item -LiteralPath .\target -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\artifacts -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\native\VaultPilot.WinUI\bin -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\native\VaultPilot.WinUI\obj -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\packaging\windows\Output -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\tmp-icons -Recurse -Force -ErrorAction SilentlyContinue
```

### What should remain after cleanup

After cleanup, your git status should mainly show only source changes such as:

- `src/...`
- `native/VaultPilot.WinUI/...`
- `docs/...`
- `scripts/...`

### Important note

Do not delete tracked source files unless you really want to remove them from the project.  
For example, `scripts/generate-icons.py` is a tracked script, not a build artifact.

## 中文

项目在本地开发和打包时会生成很多中间产物。  
重新打包前，或者准备检查 `git status` 时，可以安全清理它们。

### 常见可清理目录

- `target/`
- `artifacts/`
- `native/VaultPilot.WinUI/bin/`
- `native/VaultPilot.WinUI/obj/`
- `packaging/windows/Output/`
- `tmp-icons/` 这种临时目录

### 手动清理命令

```powershell
Remove-Item -LiteralPath .\target -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\artifacts -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\native\VaultPilot.WinUI\bin -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\native\VaultPilot.WinUI\obj -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\packaging\windows\Output -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item -LiteralPath .\tmp-icons -Recurse -Force -ErrorAction SilentlyContinue
```

### 清理后应该保留什么

清理完成后，`git status` 理论上主要只会看到源码改动，例如：

- `src/...`
- `native/VaultPilot.WinUI/...`
- `docs/...`
- `scripts/...`

### 重要说明

不要把“源码脚本”和“构建产物”混在一起删。  
例如 `scripts/generate-icons.py` 是已跟踪的源码脚本，不是中间产物。
