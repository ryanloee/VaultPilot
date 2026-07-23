# VaultPilot MCP Connector

**Standalone MCP Stdio server** — lets Claude Desktop, Cursor, Codex, and any MCP-compatible AI agent read & write your VaultPilot vault.

## Quick Start

### 1. Build

```bash
cargo build --release -p vaultpilot-mcp
```

### 2. Configure your client

<details>
<summary>Claude Desktop</summary>

Edit `claude_desktop_config.json` (or the equivalent `mcp.json` in Claude Desktop):

```json
{
  "mcpServers": {
    "vaultpilot": {
      "command": "/path/to/target/release/vaultpilot-mcp",
      "args": ["--vault-dir", "/path/to/your/vault"]
    }
  }
}
```

Or place a `mcp-config.json` file in your vault root directory to auto-discover:

```json
{ "vault_dir": "/path/to/your/vault" }
```

</details>

<details>
<summary>Codex / Cursor</summary>

Add to `.cursor/mcp.json` or codex configuration:

```json
{
  "mcpServers": {
    "vaultpilot": {
      "command": "/path/to/target/release/vaultpilot-mcp",
      "args": ["--vault-dir", "/path/to/your/vault"]
    }
  }
}
```

</details>

<details>
<summary>Generic MCP Client</summary>

The server speaks MCP protocol version `2025-06-18` (fallback `2024-11-05`) over stdio JSON-RPC 2.0. No network port required.

```bash
vaultpilot-mcp --vault-dir /path/to/your/vault
```

</details>

## Available Tools

| Tool | Description |
|------|-------------|
| `list_collections` | List all collections in the vault |
| `list_notes` | List notes within a collection (with optional search) |
| `read_note` | Read full note content by path or ID |
| `create_note` | Create a new note (returns path + ID) |
| `save_note` | Save / overwrite a note |
| `search_notes` | Full-text search across vault |

## Protocol

- **Transport**: Stdio (JSON-RPC 2.0, newline-delimited)
- **Protocol version**: `2025-06-18` (fallback `2024-11-05`)
- **Max line size**: 10 MiB

## Philosophy

- **Read-only by default** — tools that modify data are clearly marked as write operations
- **Safe concurrent access** — uses VaultPilot storage layer (same as CLI/Desktop/Mobile), so vault integrity is preserved across concurrent access
- **Transparent errors** — all errors are returned as structured JSON-RPC error responses, not panics

## MCP Registry

This server is registered at [registry.modelcontextprotocol.io](https://registry.modelcontextprotocol.io).

Package name: `vaultpilot-mcp` (reverse-DNS: `com.vaultpilot.mcp`)

## License

MIT. Part of the [VaultPilot](https://github.com/ryanloee/VaultPilot) project.