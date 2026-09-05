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
      "args": ["--vault-dir", "/path/to/your/vault"],
      "env": { "VAULTPILOT_MCP_TOKEN": "<your-token>" }
    }
  }
}
```

</details>

<details>
<summary>Token authentication (recommended)</summary>

Without a token, any local process can launch `vaultpilot-mcp` and read your vault. Configure one to change that:

1. **Server side (expected value)** — pass `--token <value>` on the command line, or put `{"token": "<value>"}` in the `mcp-config.json` at your vault root.
2. **Client side (proof)** — inject the same value as the `VAULTPILOT_MCP_TOKEN` environment variable in the client's `mcpServers` entry (as in the Claude Desktop snippet above). Clients that can't inject env vars may instead send it in the `initialize` request: `params._meta.vaultpilotToken`.

Mismatch behavior:
- Wrong or missing `VAULTPILOT_MCP_TOKEN` at launch → the server exits immediately with `unauthorized` on stderr.
- A valid env proof is accepted at launch; the `_meta` proof is checked per `initialize`. Failing `initialize` leaves the session uninitialized — every later `tools/call` is rejected.

Generate a token locally, e.g. `python -c "import secrets; print(secrets.token_hex(32))"` — never commit it to source control.

</details>

<details>
<summary>Auto-discovery</summary>

Or place a `mcp-config.json` file in your vault root directory to auto-discover:

```json
{ "vault_dir": "/path/to/your/vault", "token": "<optional>" }
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
      "args": ["--vault-dir", "/path/to/your/vault"],
      "env": { "VAULTPILOT_MCP_TOKEN": "<your-token>" }
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
| `vault_search` | Full-text search across vault |
| `vault_read` | Read full note content by ID |
| `vault_write` | Write a new note (returns the created note ID) |
| `vault_list` | List notes, optionally filtered by collection |
| `vault_related` | Find notes related to a given note ID |
| `github_list_issues` | List GitHub issues (requires `GITHUB_TOKEN` env var or `--github-token`) |
| `github_get_issue` | Get a GitHub issue by number (same token requirement) |

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