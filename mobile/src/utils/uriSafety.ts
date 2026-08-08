/**
 * vaultpilot:// URI safety helpers — TypeScript mirror of the Rust backend's
 * classify_uri_action_risk (src/deep_link.rs) for issue #3964.
 *
 * ⚠️ MIRROR CONSTRAINT (#3964/#3995): this file must stay in sync with
 * `src/deep_link.rs` — `classify_uri_action_risk`, `parse_deep_link`
 * (route table, case-insensitivity #3734) and `TrustedAppRegistry::is_trusted`
 * (lowercase comparison). When the Rust side changes any of those, update the
 * corresponding TS logic here (and vice versa). See deep_link.rs
 * `classify_uri_action_risk` (~line 432) and `automation_tool_gate`
 * (~line 700) for the authoritative classification.
 *
 * The mobile app gates risky deep links behind an explicit Alert confirmation
 * before they are dispatched to React Navigation. These helpers are pure
 * functions (no react-native imports) so they can be unit-tested in Jest
 * without mocking.
 */

/** Risk level of a vaultpilot:// deep-link action. */
export type UriActionRisk = 'low' | 'medium' | 'high';

/** Result of evaluating a URI against the trusted-source list. */
export interface UriSafetyEvaluation {
  risk: UriActionRisk;
  /** True when the caller must show a confirmation prompt before executing. */
  needsConfirmation: boolean;
  /** Human-readable description of the action (shown in the Alert). */
  reason: string;
}

/** Known vaultpilot:// routes (mirrors the Rust DeepLinkAction routes). */
export type VaultUriRoute =
  | 'chat'
  | 'chat/new'
  | 'chat/sessions'
  | 'note'
  | 'note/new'
  | 'note/delete'
  | 'note/edit'
  | 'note/bulk'
  | 'note/:id'
  | 'search'
  | 'settings'
  | 'unknown';

export interface ParsedVaultUri {
  route: VaultUriRoute;
  /** noteId for route 'note/:id'. */
  noteId?: string;
  /**
   * overwrite query flag for route 'note/new'. true/false when the param is
   * present, undefined when absent.
   */
  overwrite?: boolean;
}

/**
 * Values treated as TRUTHY for the overwrite query flag — mirrors Rust's
 * `flag("overwrite")` in deep_link.rs: truthy = {1, true, yes, on}, anything
 * else is falsy (#3995; previously this was an inverted falsy-set which
 * disagreed on values like `overwrite=2`).
 */
const TRUTHY_FLAG_VALUES = new Set(['1', 'true', 'yes', 'on']);

/**
 * Split a raw URI into scheme, path and query params without throwing on
 * malformed input. Returns null for anything that is not a vaultpilot:// URI.
 */
function parseVaultUri(uri: string): { path: string; params: Record<string, string> } | null {
  if (typeof uri !== 'string') return null;
  const trimmed = uri.trim();
  const schemeEnd = trimmed.indexOf('://');
  if (schemeEnd <= 0) return null;
  if (trimmed.slice(0, schemeEnd).toLowerCase() !== 'vaultpilot') return null;

  let rest = trimmed.slice(schemeEnd + 3);
  const queryIdx = rest.indexOf('?');
  let query = '';
  if (queryIdx >= 0) {
    query = rest.slice(queryIdx + 1);
    rest = rest.slice(0, queryIdx);
  }
  const hashIdx = rest.indexOf('#');
  if (hashIdx >= 0) rest = rest.slice(0, hashIdx);
  // Normalize trailing slashes: vaultpilot://note/new/ === vaultpilot://note/new
  const path = rest.replace(/\/+$/, '');
  if (!path) return null;

  const params: Record<string, string> = {};
  if (query) {
    for (const pair of query.split('&')) {
      if (!pair) continue;
      const eqIdx = pair.indexOf('=');
      const rawKey = eqIdx >= 0 ? pair.slice(0, eqIdx) : pair;
      const rawValue = eqIdx >= 0 ? pair.slice(eqIdx + 1) : 'true';
      let key: string;
      let value: string;
      try {
        key = decodeURIComponent(rawKey);
        value = decodeURIComponent(rawValue);
      } catch {
        continue; // malformed percent-encoding — ignore this pair
      }
      if (key.trim()) params[key.trim()] = value;
    }
  }
  return { path, params };
}

/** True when the overwrite query flag value should be treated as truthy. */
function isTruthyFlag(value: string | undefined): boolean {
  if (value === undefined) return false;
  return TRUTHY_FLAG_VALUES.has(value.trim().toLowerCase());
}

/**
 * Parse a vaultpilot:// URI into a route descriptor. Unknown/malformed URIs
 * map to route 'unknown' (never throws).
 *
 * Mirrors `parse_deep_link` (deep_link.rs ~line 194): route keywords are
 * matched case-insensitively (#3734/#3995), note ids keep their original
 * case, and `note/open/<id>` is an alias for `note/<id>` (OpenNote).
 */
export function parseVaultPilotUri(uri: string): ParsedVaultUri {
  const parsed = parseVaultUri(uri);
  if (!parsed) return { route: 'unknown' };

  const segments = parsed.path.split('/');
  const lower = segments.map((s) => s.toLowerCase());
  const [head, second] = lower;

  switch (head) {
    case 'chat':
      if (second === 'new') return { route: 'chat/new' };
      if (second === 'sessions') return { route: 'chat/sessions' };
      if (segments.length === 1) return { route: 'chat' };
      break;
    case 'note':
      if (segments.length === 1) return { route: 'note' };
      if (second === 'new') {
        return {
          route: 'note/new',
          overwrite:
            parsed.params['overwrite'] !== undefined
              ? isTruthyFlag(parsed.params['overwrite'])
              : undefined,
        };
      }
      // Destructive / bulk routes — Rust classifies DeleteNote / EditNote /
      // BulkNoteOp as HIGH (#3964/#3995). Matched before the note/:id fallback
      // so `note/delete` is never treated as "open the note named delete".
      if (second === 'delete') return { route: 'note/delete' };
      if (second === 'edit') return { route: 'note/edit' };
      if (second === 'bulk') return { route: 'note/bulk' };
      // note/open/<id> is an explicit alias for note/<id> (Rust OpenNote).
      if (second === 'open' && segments.length === 3 && segments[2]) {
        return { route: 'note/:id', noteId: segments[2] };
      }
      // note/<id> → open existing note (id keeps original case).
      if (segments.length === 2 && segments[1]) {
        return { route: 'note/:id', noteId: segments[1] };
      }
      break;
    case 'search':
      if (segments.length === 1) return { route: 'search' };
      break;
    case 'settings':
      if (segments.length === 1) return { route: 'settings' };
      break;
    default:
      break;
  }
  return { route: 'unknown' };
}

/**
 * Classify the risk of a vaultpilot:// URI (mirror of Rust
 * classify_uri_action_risk, deep_link.rs ~line 432):
 *   - vaultpilot://chat/new            → HIGH (AI chat may trigger agent tools)
 *   - vaultpilot://note/new?overwrite  → HIGH (irreversible overwrite)
 *   - vaultpilot://note/delete         → HIGH (irreversible delete, #3964/#3995)
 *   - vaultpilot://note/edit           → HIGH (destructive rewrite, #3964/#3995)
 *   - vaultpilot://note/bulk           → HIGH (bulk destructive op, #3964/#3995)
 *   - vaultpilot://note/new            → MEDIUM (creates a note)
 *   - vaultpilot://note/:id            → LOW (opens existing note)
 *   - vaultpilot://search|settings     → LOW
 *   - unknown/unparseable routes      → MEDIUM (Rust: Unknown → Medium,
 *                                        "be conservative", #3995)
 */
export function classifyUriActionRisk(uri: string): UriActionRisk {
  const parsed = parseVaultPilotUri(uri);
  switch (parsed.route) {
    case 'chat/new':
    case 'note/delete':
    case 'note/edit':
    case 'note/bulk':
      return 'high';
    case 'note/new':
      return parsed.overwrite ? 'high' : 'medium';
    case 'chat':
    case 'chat/sessions':
    case 'note':
    case 'note/:id':
    case 'search':
    case 'settings':
      return 'low';
    default:
      // Unknown routes: conservative Medium — mirrors Rust's Unknown → Medium.
      return 'medium';
  }
}

/**
 * Extract the x-source query param — the package/app that opened the URI
 * (e.g. vaultpilot://chat/new?x-source=com.example.app). Returns '' when
 * absent or unparseable.
 */
export function extractSource(uri: string): string {
  const parsed = parseVaultUri(uri);
  if (!parsed) return '';
  const source = parsed.params['x-source'];
  return source ? source.trim() : '';
}

const REASONS: Record<UriActionRisk, string> = {
  high: '此链接将执行高风险操作：启动 AI 对话（可能触发代理工具执行）、删除/覆盖笔记或执行批量操作，且无法撤销。',
  medium: '此链接将执行中风险操作（例如创建新笔记或无法识别的路由）。',
  low: '低风险操作：打开已有内容或跳转页面。',
};

/**
 * Evaluate whether a vaultpilot:// URI needs a confirmation prompt:
 *   - HIGH  → always needsConfirmation, even for trusted sources
 *             (mirrors Obsidian's high-risk-always-confirm policy)
 *   - MEDIUM→ needsConfirmation unless the x-source is in trustedSources
 *   - LOW   → never needs confirmation
 *
 * Trusted-source comparison is case-insensitive, mirroring Rust
 * `TrustedAppRegistry::is_trusted` which lowercases both sides (#3995).
 */
export function evaluateUriSafety(uri: string, trustedSources: string[]): UriSafetyEvaluation {
  const risk = classifyUriActionRisk(uri);
  const source = extractSource(uri);

  if (risk === 'high') {
    return { risk, needsConfirmation: true, reason: REASONS.high };
  }
  if (risk === 'medium') {
    const sourceLower = source.toLowerCase();
    const trusted =
      source !== '' &&
      trustedSources.some((s) => s.trim().toLowerCase() === sourceLower);
    return { risk, needsConfirmation: !trusted, reason: REASONS.medium };
  }
  return { risk, needsConfirmation: false, reason: REASONS.low };
}
