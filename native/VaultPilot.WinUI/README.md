# VaultPilot.WinUI

Native Windows frontend for VaultPilot.

This is the initial WinUI 3 shell for the migration. It is intentionally thin until the Rust backend is extracted into `vaultpilot-agent.exe`.

## Expected Backend

The frontend will start a Rust sidecar process and communicate with it through line-delimited JSON RPC. The current contract lives at:

```text
../../contracts/vaultpilot-agent.v1.json
```

## Build

Requires .NET SDK and Visual Studio Windows App SDK components:

```powershell
dotnet restore
dotnet build
```
