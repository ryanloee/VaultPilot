# src/ai — Provider & Agent Pipeline

## OVERVIEW
Provider abstraction (Anthropic/OpenAI-compatible), grounded Q&A pipeline, tool loop, conversation compression.

## WHERE TO LOOK
| Task | Location | Notes |
|------|----------|-------|
| Answer parsing | `parsing.rs:parse_model_answer` | empty content = error, no fallback |
| Provider client | `client.rs` | `reqwest` rustls-tls, streaming |
| Tool definitions | `actions.rs` (2k lines) | search_notes/read_file/save_note etc. |
| Grounded ask | `orchestration/ask.rs` | retrieval → prompt → answer → citation |
| Agent loop | `src/agent.rs` (3.4k lines) | multi-step tool calling |
| Compress | `parsing.rs:compress_conversation` | strict, same policy as answer path |

## CONVENTIONS
- Provider responses validated strictly — `parse_model_answer` never fabricates text.
- Only ingest-side tolerances allowed: `parse_or_fallback_note` (heuristic draft), `fallback_record_reply` (templated ack).
- `Email` feature (IMAP) optional — guarded by `#[cfg(feature="email")]`.

## ANTI-PATTERNS
- No answer-path fallback text — user-mandated; removed `fallback_answer` intentionally.
- No `unwrap` on model JSON — use `parse_model_answer` error path.
- No new provider without updating `contracts/vaultpilot-agent.v1.json` if exposed to agents.

## NOTES
- Token usage (`usageInputTokens/usageOutputTokens`) surfaced in `GroundedAnswer` → frontend.
- `save_temp_attachment` (#4074/#4083): chat images → `attachments/chat/` (persistent), audio → temp TTL.
