# Clean Guide

## English

Remove local build outputs:

- `target/`
- `artifacts/`
- `desktop/node_modules/`
- `desktop/dist/`
- `desktop/src-tauri/target/`

Quick PowerShell cleanup:

```powershell
.\scripts\clean.ps1
```

Use `.\scripts\clean.ps1 -IncludeReleaseAssets` to also remove `release-assets/`.

## 中文

清理本地构建产物：

- `target/`
- `artifacts/`
- `desktop/node_modules/`
- `desktop/dist/`
- `desktop/src-tauri/target/`

快速清理：

```powershell
.\scripts\clean.ps1
```

如需一并删除 `release-assets/`，使用 `.\scripts\clean.ps1 -IncludeReleaseAssets`。
