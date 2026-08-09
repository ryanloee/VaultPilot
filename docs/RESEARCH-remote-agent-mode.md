# Research: Remote Server Mode for Mobile Agent Mode

## 1. Summary of Current Infrastructure

### Rust Backend (axum HTTP bridge)
- **Existing**: `src/bin/vaultpilot-cli/http_bridge.rs` contains a full axum v0.8 HTTP server
  - Routes: `/v1/chat/completions` (OpenAI-compatible SSE streaming), `/v1/models`, CRUD for notes, subscriptions, AI actions, health
  - SSE streaming via `tokio::sync::mpsc::channel` + `axum::response::sse::Sse<ReceiverStream>`
  - Auth via optional bearer token (`require_bridge_token` middleware)
  - Rate limiting, CORS (localhost-only), 180s timeout layer, 10MB body limit
  - `CancellationToken` for client-disconnect cancellation of upstream AI requests
- **Agent module** (`src/agent.rs`): `run_agent()` — async function taking `(settings, context, prompt, images, history, config, on_event, on_plan_decision) → AgentResult`. Emits typed `AgentEvent` variants via callback (Thinking, ToolCall, ToolResult, FinalAnswer, PlanProposed, etc.)
- **Engine layer** (`src/agent_engine.rs`): `AgentEngine` trait (`send_prompt`), `SubprocessEngine` for external CLI agents (Claude Code, Codex), `AgentEngineRegistry`
- **Types ready to reuse**: `AgentConfig`, `AgentResourceLimits`, `AgentPermission`, `AgentEvent`, `AgentResult`, `ExecutionPlan`, `EngineContext`, all with `serde::{Serialize, Deserialize}`

### Mobile App (React Native / Expo)
- **API client** (`mobile/src/api/client.ts`): connects to **LLM providers** (OpenAI-compatible, Anthropic) via SSE. Already has `chat()` returning `ReadableStream`, `chatWithReconnect()` with auto-reconnect, `checkApi()` health check
- **Secure storage**: `expo-secure-store` already used for API keys (`SecureStore.setItemAsync/getItemAsync`)
- **SSE parser** (`mobile/src/api/sse.ts`): `parseSSEStream()`, `parseSSEStreamWithReconnect()` — full reconnection logic with exponential backoff, abort support, parse error handling
- **Offline queue** (`mobile/src/utils/offlineSync.ts`): sync queue with retry, backoff, pending sync tracking. **Currently for note edits only — no agent-specific offline logic**
- **Network state**: `useNetworkState` hook for connectivity awareness
- **Store**: Zustand with `persist` middleware; `ProviderConfig[]` with `SecureStore`-backed keys

### Key gap
**No existing endpoint for remote agent execution.** The `/v1/chat/completions` endpoint is a simple LLM proxy — it does NOT use `run_agent()`, `ToolProxy`, sandboxing, or any of the agent infrastructure. Agent mode is currently only available via the CLI (`vaultpilot agent run`) and the desktop WinUI app (which uses named pipes).

---

## 2. Proposed REST API Surface

### 2.1 Agent Session Lifecycle

```
POST   /api/agent/sessions              # Create a new agent session
GET    /api/agent/sessions/{session_id}  # Get session status + audit log
DELETE /api/agent/sessions/{session_id}  # Cancel/kill a running session
POST   /api/agent/sessions/{session_id}/prompt  # Send a prompt (SSE streaming)
POST   /api/agent/sessions/{session_id}/plan/decision  # Submit plan decision (Approve/Reject/Edit)
GET    /api/agent/engines                # List available agent engines (builtin, claude-code, codex)
GET    /api/agent/config/defaults        # Get default AgentConfig
```

### 2.2 Detailed Endpoints

#### POST `/api/agent/sessions` — Create Session
```json
// Request
{
  "config": {
    "name": "my-agent",
    "permission": "read_only",           // "read_only" | "read_write"
    "limits": {
      "max_duration_secs": 300,          // defaults from AgentResourceLimits
      "max_tool_calls": 100,
      "max_tokens": 0
    },
    "allowed_tools": [],                 // empty = all read-only tools
    "write_patterns": ["inbox/*", "daily-notes/*"],
    "execution_mode": "direct"           // "direct" | "plan"
  },
  "vault": "default"                     // vault name; server resolves to path
}

// Response 201
{
  "session_id": "uuid-v4",
  "created_at": "2026-07-06T12:00:00Z",
  "status": "created",                   // "created" | "running" | "awaiting_plan_decision" | "completed" | "failed" | "cancelled"
  "config": { ... }                      // resolved config
}
```

**Implementation notes:**
- The server creates an `AgentSession` (from `agent.rs`), stores it in an `Arc<Mutex<HashMap<String, SessionState>>>` (in-memory; sessions are ephemeral).
- Session status transitions: `created → running → completed|failed`, or `created → awaiting_plan_decision → running → completed`.
- A background cancellation token is wired to client disconnect.

#### POST `/api/agent/sessions/{session_id}/prompt` — Run Agent (SSE Stream)
```json
// Request
{
  "prompt": "Search my notes about Rust async patterns...",
  "history": [                           // optional conversation history
    { "role": "user", "content": "..." },
    { "role": "assistant", "content": "..." }
  ],
  "images": []                           // optional base64-encoded images
}

// Response: SSE stream of AgentEvent JSON
// event: agent_event
// data: {"type":"thinking","step":1}
// 
// event: agent_event
// data: {"type":"tool_call","step":1,"tool":"search_notes","args":"..."}
//
// event: agent_event
// data: {"type":"tool_result","step":1,"tool":"search_notes","result_preview":"...","is_error":false}
//
// event: agent_event
// data: {"type":"final_answer","text":"..."}
//
// event: agent_event
// data: {"type":"plan_proposed","plan":{"task":"...","steps":[...]}}
// (session status becomes "awaiting_plan_decision" — client must POST /plan/decision)
//
// event: done
// data: [DONE]
```

**Implementation approach:**
- Reuse the existing SSE streaming pattern from `http_chat_completions` (channel-based, `CancellationToken`)
- The `on_event` callback in `run_agent()` sends events through a `tokio::sync::mpsc::channel`
- Forwarder task reads from the channel and writes SSE events
- On client disconnect (`sse_tx.send` fails), `CancellationToken.cancel()` aborts the agent loop
- `AgentEvent` already derives `Serialize` with `#[serde(tag = "type")]` — perfect for SSE

#### POST `/api/agent/sessions/{session_id}/plan/decision` — Submit Plan Decision
```json
// Request
{
  "decision": "approve"                   // "approve" | "reject" | "partial_approve"
  // OR
  "decision": "edit",
  "steps": [...]                          // revised steps for edit mode
}

// Response 200
{
  "status": "running",                    // or "completed" if decision was reject
  "message": "Plan accepted, executing..."
}
```

**Implementation:**
- The session's `on_plan_decision` callback blocks via a `tokio::sync::oneshot::channel`
- When client sends `plan/decision`, the server resolves the oneshot with the appropriate `PlanDecision`
- Session status transitions from `awaiting_plan_decision` back to `running`

#### GET `/api/agent/engines` — List Available Engines
```json
// Response
{
  "engines": [
    { "name": "builtin", "available": false, "description": "VaultPilot's self-built agent loop..." },
    { "name": "claude-code", "available": true, "description": "Anthropic Claude Code CLI..." },
    { "name": "codex", "available": true, "description": "OpenAI Codex CLI..." }
  ]
}
```

Wraps `AgentEngineRegistry::engine_infos()`.

#### GET `/api/agent/sessions/{session_id}` — Session Status
```json
// Response
{
  "session_id": "...",
  "status": "running",
  "config": { ... },
  "audit_log": [
    { "timestamp": "...", "tool": "search_notes", "args_summary": "...", "allowed": true, "reason": "ok" }
  ],
  "tool_call_count": 7,
  "elapsed_secs": 12.5
}
```

#### DELETE `/api/agent/sessions/{session_id}` — Cancel Session
```json
// Response 200
{ "status": "cancelled" }
```

Calls `cancellation_token.cancel()` on the running task, then cleans up the session map.

---

## 3. Reusing Existing AgentConfig, AgentResourceLimits, and Permission Model over HTTP

### What maps directly (no change needed)

| Rust Type | HTTP Representation | Reuse Plan |
|---|---|---|
| `AgentPermission` | `"read_only"` / `"read_write"` | `#[derive(Serialize, Deserialize)]` with `#[serde(rename_all="snake_case")]` — already works |
| `AgentResourceLimits` | `{ max_duration_secs, max_tool_calls, max_tokens }` | Add a `#[serde(deserialize_with)]` helper to convert `max_duration_secs: u64` → `Duration` (or make the API use secs and convert server-side) |
| `AgentConfig` | Full JSON object | `#[derive(Serialize, Deserialize)]` — already works |
| `ExecutionMode` | `"direct"` / `"plan"` | Already `#[serde(rename_all="snake_case")]` |
| `AgentAuditEntry` | As-is JSON | Already `Serialize + Deserialize` |
| `AgentEvent` | SSE data lines | Already `Serialize` with `#[serde(tag = "type")]` |
| `ExecutionPlan` | As-is JSON for plan_proposed event | Already ready |

### What needs adapting

1. **`max_duration`: Duration vs seconds**: `AgentResourceLimits.max_duration` is `std::time::Duration`. JSON doesn't natively serialize Duration. Options:
   - (Recommended) Custom `#[serde(deserialize_with)]` in the API layer that accepts `max_duration_secs: u64` and converts to `Duration`. Keep the internal type unchanged.
   - Or serialize Duration as seconds in the response (the `elapsed_secs` field in session status)

2. **Vault resolution**: The mobile client sends `vault: "default"` (or vault name); the server resolves it to a filesystem path via its config/settings. This avoids exposing filesystem paths to mobile.

3. **Session storage**: Sessions are ephemeral, in-memory (server restart = lost sessions). For a production server, consider:
   - SQLite-backed session persistence for crash recovery
   - Session timeout/GC for abandoned sessions

---

## 4. SSE/Streaming Considerations for Real-Time Agent Output

### Current SSE infrastructure already in place

| Component | Location | Ready For Agent Mode? |
|---|---|---|
| Rust: SSE response with `CancellationToken` | `http_bridge.rs` lines 2615-2673 (streaming branch ~2634-2673) | ✅ Yes, reusable pattern |
| Rust: `tokio::sync::mpsc` bounded channels | `http_bridge.rs` line 2616 (`sse_tx`, cap=16) and line 2624 (`chunk_tx`, cap=64) | ✅ Yes, just needs different event format |

> **Channel clarification:** the capacity-16 channel (`sse_tx`, line 2616) is the **SSE socket** boundary; the capacity-64 channel (`chunk_tx`, line 2624) is the **upstream→forwarder** backpressure boundary. Do not conflate the two — backpressure is enforced by `chunk_tx`, not by the SSE socket channel.
| Mobile: `parseSSEStreamWithReconnect` | `sse.ts` line 162-257 | ✅ Yes, auto-reconnect with backoff |
| Mobile: `StreamChunk` type | `sse.ts` lines 3-7 | ⚠️ Currently assumes OpenAI delta format — needs extension for agent events |

### What's different for agent vs chat streaming

| Aspect | Chat Completions | Agent Mode |
|---|---|---|
| Event types | `content` delta only | `thinking`, `tool_call`, `tool_result`, `final_answer`, `plan_proposed`, `error` |
| SSE event names | Default (no named event) | Named events like `event: agent_event`, `event: done` |
| Client-side parsing | Simple concatenation of content deltas | Must interpret typed events, update UI accordingly |
| Cancellation | Client disconnect | Client disconnect + explicit `DELETE /session` |
| Plan mode pause | N/A | Stream pauses at `plan_proposed`, waits for `/plan/decision` |
| Retry behavior | Safe to retry (idempotent LLM call) | NOT safe to retry — agent has already executed tools. Once content is delivered, `parseSSEStreamWithReconnect` correctly stops retrying. |

### Recommended SSE format for agent events

```json
event: agent_event
data: {"type":"thinking","step":1}

event: agent_event
data: {"type":"tool_call","step":1,"tool":"search_notes","args":"{\"query\":\"Rust async\"}"}

event: agent_event
data: {"type":"tool_result","step":1,"tool":"search_notes","result_preview":"Found 3 notes...","is_error":false}

event: agent_event
data: {"type":"final_answer","text":"Here's what I found..."}

event: agent_event
data: {"type":"plan_proposed","plan":{"task":"...","steps":[...]}}

event: done
data: [DONE]
```

**Mobile client changes needed:**
- Extend `StreamChunk` to include a `kind` field matching `AgentEvent` variants
- Add an `AgentEventCallback` type alongside `onChunk` for structured event handling
- The SSE parser already handles arbitrary `data:` lines — just parse and dispatch by `type` field

### Streaming backpressure

- The existing bounded channel (capacity 64) provides backpressure
- On mobile, the SSE reader naturally paces consumption via `ReadableStreamDefaultReader.read()`
- `CancellationToken` on client disconnect prevents resource leaks

---

## 5. Security

### 5.1 API Key Storage on Mobile

**Current**: `expo-secure-store` with keys stored encrypted per-provider (see `mobile/src/store.ts` lines 26-68). The `SecureStore` is backed by iOS Keychain / Android Keystore.

**Recommendation**: Continue using `SecureStore`. Add a new configuration key for the VaultPilot server URL + auth token:

```typescript
// In mobile/src/api/client.ts or a new agentClient.ts
const AGENT_KEYS = {
  serverUrl: 'cfg_agent_server_url',
  authToken: 'cfg_agent_auth_token',  // stored in SecureStore
} as const;
```

### 5.2 Auth Tokens

**Current**: The HTTP bridge uses an optional bearer token (`x-api-key` header, validated by `require_bridge_token`). This is suitable for local-only usage (same WiFi/LAN).

**For remote server mode**, two approaches:

| Approach | Pros | Cons |
|---|---|---|
| **Static bearer token** (current pattern) | Simple, no additional infrastructure | Token rotation requires manual update; token leaked = full access |
| **Short-lived JWT with refresh** | Revocable, time-limited, per-device | Needs auth endpoint, token refresh logic, JWT library on server |
| **API key per device/user** | Compartmentalized, revocable individually | Key management overhead |

**Recommendation** (Phase 1): Start with the existing static bearer token pattern (same as current HTTP bridge). Add JWT support in Phase 2 if multi-user scenarios arise.

**Implementation detail**: The server-side `require_bridge_token` middleware already exists. The mobile client already has `SecureStore` for storing the token. The `Authorization: Bearer <token>` header pattern is already used for LLM API keys.

### 5.3 Network Security

- **TLS**: Remote mode MUST use HTTPS in production. The current HTTP bridge is local-only (binds to `127.0.0.1`). For remote, bind to `0.0.0.0` + TLS termination via reverse proxy (nginx/caddy) or built-in `axum-server + rustls`.
- **CORS**: Currently restricted to `localhost` origins. For remote mobile, must allow the mobile app's origin or be permissive with `AllowOrigin::any()` (acceptable if token-authenticated).
- **Rate limiting**: Already implemented (60 req/min/IP). Should be fine-tuned for agent sessions (long-running SSE connections consume one "slot").

### 5.4 Sandbox Security Over HTTP

The existing `ToolProxy` sandbox (path confinement, permission model, write patterns) works identically over HTTP — the `AgentSession` is constructed the same way server-side. No HTTP-specific sandbox bypass is introduced because:
1. The mobile client never sends filesystem paths — it sends abstracted agent config
2. The server resolves vault paths internally
3. Tool execution happens server-side, subject to `ToolProxy::check_tool_call()`
4. The audit log is still collected and returned

---

## 6. Offline Fallback Strategy

### Current offline infrastructure
- `offlineSync.ts`: Flushes pending note edits when connectivity returns (queue + retry)
- `useNetworkState`: Connectivity detection
- `db.ts`: Local SQLite database for notes (PouchDB-like pattern)
- **No local agent execution capability**

### Offline approach for Agent Mode

**Reality**: Agent Mode inherently requires a backend LLM or external CLI agent binary. A mobile device cannot run:
- The builtin agent (`run_agent()` — needs LLM API access)
- Claude Code / Codex (not available on mobile)

**Therefore: "Offline" for Agent Mode means grace degradation, not local execution.**

| Scenario | Behavior |
|---|---|
| **Connected** | Normal remote agent mode as described above |
| **Connection lost mid-session (SSE disconnect)** | `parseSSEStreamWithReconnect` stops retrying after content delivery (correct behavior — agent state is server-side). Show "Connection lost — check session status" in UI. |
| **Connection lost before starting session** | Show "Agent Mode requires network connection" message. Queue the agent request as a "pending agent task" (new feature, analogous to `offlineSync` for notes). |
| **Reconnecting after offline queue** | On reconnection, attempt to run the queued prompt. Show result when available. |
| **Server unreachable (different network / server down)** | Surface clear error. Do NOT retry automatically (agent results are not idempotent). |

### Recommended offline extensions

1. **Add agent task queue** (similar to `offlineSync`):
   ```
   mobile/src/utils/agentQueue.ts
   ```
   - Queues `{ prompt, images?, config }` when offline
   - On reconnect: presents queued tasks to user for confirmation, then executes
   - Stores in local SQLite (`db.ts`) with status: `pending | running | completed | failed`

2. **Session recovery endpoint**:
   ```
   GET /api/agent/sessions/{session_id}/reconnect
   ```
   - If a session is still running server-side (client disconnected during SSE), returns buffered events since last known event
   - Requires client to send `last_event_index` or `last_event_timestamp`
   - This is **optional** — simple approach is to start a new session

3. **UI states**:
   - Agent Mode unavailable (no network) → show offline notice
   - Queued tasks → show in a "Pending Tasks" section
   - Disconnected mid-execution → show "Reconnecting..." with option to check status

---

## 7. Development Effort Estimate

### Approach A: Full Remote Agent Mode (described above)
**Estimated: 4-6 weeks (1 developer, moderate Rust + React Native experience)**

| Module | Effort | Details |
|---|---|---|
| **Rust: Agent API endpoints** | 1.5 weeks | ~400 lines of new routes in `http_bridge.rs`: session CRUD, prompt execution with SSE, plan decision. Most of this is wiring existing types (`run_agent`, `AgentEvent`, `ExecutionPlan`) into channel-based streaming. |
| **Rust: Session lifecycle management** | 0.5 week | `HashMap<String, SessionState>` with `CancellationToken`, timeout/GC for abandoned sessions, error handling for concurrent access |
| **Rust: Agent engine over HTTP** | 0.5 week | Wire `AgentEngineRegistry` → `GET /engines`, allow selecting engine per session (future: `engine_name` field in session creation) |
| **Rust: Auth & security hardening** | 0.5 week | Token validation, CORS for mobile, TLS guide, rate limit tuning for SSE sessions |
| **Mobile: Agent SSE parser** | 0.5 week | Extend `StreamChunk` → `AgentStreamChunk`, `parseAgentSSEStream()` that dispatches typed events. Reuse existing `parseSSEStreamWithReconnect`. |
| **Mobile: Agent API client** | 1 week | New `agentClient.ts` with `createSession()`, `runPrompt()`, `submitPlanDecision()`, `cancelSession()`, `getEngines()`. Uses existing fetch + SSE patterns. |
| **Mobile: Agent UI components** | 1.5 weeks | Agent chat view that renders Thinking/ ToolCall/ ToolResult/ FinalAnswer states, Plan approval UI (show steps, approve/reject/edit), session status indicator |
| **Mobile: Offline agent queue** | 0.5 week | Queue prompts offline, surface on reconnect, debounce retries (agent output is not idempotent) |
| **Integration testing** | 0.5 week | E2E test: create session → run prompt → observe SSE events on mobile → cancel → verify audit log |
| **Total** | **~6 weeks** | |

### Approach B: Minimal Proxy (reuse `/v1/chat/completions`)
**Estimated: 1-2 weeks**

Instead of creating dedicated agent endpoints, extend the existing `/v1/chat/completions` to accept an optional `agent_mode: true` flag. When set, the server:
1. Extracts the last user message as the prompt
2. Creates an implicit `AgentSession` with default config
3. Runs `run_agent()` and wraps its `AgentEvent` stream into OpenAI-compatible SSE chunks (content deltas only)
4. Throws away structured events (tool calls, audit log)

**Pros:** Minimal backend changes, minimal mobile changes (just `stream: true` with a new flag)
**Cons:** No audit log visibility on mobile, no Plan Mode, no session lifecycle, no cancellation, no engine selection — all the value of agent mode is lost

### Approach C: Hybrid (recommended baseline for Phase 1)
**Estimated: 3-4 weeks**

Build the full REST API surface (Approach A) but with simplified mobile integration:
1. Implement all agent endpoints on the Rust side ✅
2. Mobile: Create `agentClient.ts` with raw SSE parsing (no fancy UI components in Phase 1)
3. Mobile UI: Show agent responses as markdown (collapsing tool calls into expandable sections)
4. Plan Mode: Show plan as structured card, allow approve/reject (edit comes in Phase 2)
5. Offline: Basic "no network" guard (no queue yet)

This delivers the core value earlier and defers polish to Phase 2.

---

## 8. Pros, Cons, and Effort Summary

### Approach A (Full Remote Agent Mode) — 6 weeks

| Pros | Cons |
|---|---|
| Full agent lifecycle: session, config, audit, cancellation | More Rust code in http_bridge.rs |
| Structured SSE events enable rich mobile UI | Requires Plan Mode state machine on server |
| Plan Mode fully supported | More mobile-side work (dedicated UI components) |
| Complete security boundary (sandbox works as designed) | |
| Audit log accessible from mobile | |
| Multi-engine support (claude-code, codex) | |

### Approach B (Minimal Proxy) — 1-2 weeks

| Pros | Cons |
|---|---|
| Minimal backend changes | Flat output — no structured events visible on mobile |
| Mobile mostly unchanged | No audit log from mobile |
| Fastest path to "something" | No Plan Mode approval flow |
| | No session lifecycle or cancellation |
| | No engine selection |
| | Wastes existing agent infrastructure (`AgentEvent` types, ExecutionPlan) |

### Approach C (Hybrid / Phase 1) — 3-4 weeks ✅ **RECOMMENDED**

| Pros | Cons |
|---|---|
| Full backend API — no rework needed in Phase 2 | Mobile UI less polished initially |
| Mobile gets structured agent events from day 1 | No offline queue (add in Phase 2) |
| Plan Mode works | |
| Audit log available | |
| Can extend UI incrementally | |

### Architecture Decision: Why the agent endpoints should be NEW routes, not an extension of `/v1/chat/completions`

The existing `/v1/chat/completions` is designed as a pure LLM proxy (stateless, OpenAI-compatible). Agent Mode is fundamentally different:

1. **Stateful**: Sessions persist server-side (audit log, plan decision awaiting)
2. **Multi-message protocol**: Create session → send prompt → receive structured events → possibly submit plan decision → get final result
3. **Different output format**: Not content deltas, but typed `AgentEvent`s
4. **Different cancellation semantics**: `DELETE /session` vs abort signal on fetch
5. **Different resource model**: Each session holds a `ToolProxy`, tokio tasks, and channels

New routes under `/api/agent/` keep concerns cleanly separated and are easier to version, test, and maintain.

---

## 9. Integration Points Summary

| Integration | What's Needed | Where |
|---|---|---|
| Rust → Mobile (SSE streaming) | `axum::response::sse::Sse<ReceiverStream>` with `AgentEvent` JSON | New: `src/bin/vaultpilot-cli/http_bridge.rs` (add routes) |
| Rust (session management) | `Arc<Mutex<HashMap<String, SessionState>>>` | New: same file or new `agent_session.rs` module |
| Rust → Agent engine | `AgentEngineRegistry` → `GET /api/agent/engines` | Wire existing to new route |
| Rust → `run_agent()` | Pass `on_event` callback that sends to channel | Existing function, new wiring |
| Mobile → Rust (REST calls) | `agentClient.ts` with fetch + SecureStore | New: `mobile/src/api/agentClient.ts` |
| Mobile (SSE parsing) | Parse `AgentEvent` JSON from SSE stream | Extend: `mobile/src/api/sse.ts` |
| Mobile (UI) | Agent chat view, plan approval view | New components under `mobile/src/components/ai/` |
| Mobile (offline) | Agent task queue | New: `mobile/src/utils/agentQueue.ts` |
| Security | Bearer token in SecureStore | Minor addition to existing pattern |
