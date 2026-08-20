# src/bin/vaultpilot-cli — CLI Binary

## OVERVIEW
`vaultpilot-cli` binary — 50+ subcommands in `main.rs` (12k lines). Thin dispatch into `vaultpilot_lib`.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Command dispatch | `main.rs` `handle_*` functions | `handle_agent_engine` etc., VaultPilot_lib calls |
| Agent daemon | `src/bin/vaultpilot-agent.rs` (2.3k lines) | standalone daemon, `RunAgentParams` etc. |
| HTTP bridge | `http_bridge.rs` (4.3k lines) | OpenAI-compatible `serve` endpoint |
| MCP server | `mcp_server.rs` (3.7k lines) | `mcp` / `mcp-http` commands |
| Completions | `main.rs:completions` | bash/zsh/fish/powershell |

## CONVENTIONS
- Every subcommand ultimately calls `vaultpilot_lib::*_with_context` — no duplicated logic.
- CLI trigger `fire-now` shares `fire_due_rules_with_dispatch` with desktop executor.

## ANTI-PATTERNS
- No business logic duplicated from `vaultpilot_lib` — CLI is dispatch only.
- No blocking work on async runtime without `spawn_blocking` for storage calls.
