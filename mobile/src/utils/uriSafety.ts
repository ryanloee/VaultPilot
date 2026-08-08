/**
 * vaultpilot:// URI safety helpers — TypeScript mirror of the Rust backend's
 * classify_uri_action_risk (src/deep_link.rs) for issue #3964.
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

/** Known vaultpilot:// routes (mirrors the React Navigation linking config). */
export type VaultUriRoute =
  | 'chat'
  | 'chat/new'
  | 'chat/sessions'
  | 'note'
  | 'note/new'
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

/** Values treated as falsy for the overwrite query flag. */
const FALSY_FLAG_VALUES = new Set(['', 'false', '0', 'no', 'off']);

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
  return !FALSY_FLAG_VALUES.has(value.trim().toLowerCase());
}

/**
 * Parse a vaultpilot:// URI into a route descriptor. Unknown/malformed URIs
 * map to route 'unknown' (never throws).
 */
export function parseVaultPilotUri(uri: string): ParsedVaultUri {
  const parsed = parseVaultUri(uri);
  if (!parsed) return { route: 'unknown' };
  switch (parsed.path) {
    case 'chat':
    case 'chat/new':
    case 'chat/sessions':
    case 'note':
    case 'note/new':
    case 'search':
    case 'settings':
      return {
        route: parsed.path,
        overwrite:
          parsed.path === 'note/new' && parsed.params['overwrite'] !== undefined
            ? isTruthyFlag(parsed.params['overwrite'])
            : undefined,
      };
    default:
      break;
  }
  // note/:id → open existing note
  if (parsed.path.startsWith('note/')) {
    const id = parsed.path.slice('note/'.length);
    if (id && !id.includes('/')) return { route: 'note/:id', noteId: id };
  }
  return { route: 'unknown' };
}

/**
 * Classify the risk of a vaultpilot:// URI (mirror of Rust
 * classify_uri_action_risk):
 *   - vaultpilot://chat/new            → HIGH (AI chat may trigger agent tools)
 *   - vaultpilot://note/new?overwrite  → HIGH (irreversible overwrite)
 *   - vaultpilot://note/new            → MEDIUM (creates a note)
 *   - vaultpilot://note/:id            → LOW (opens existing note)
 *   - vaultpilot://search|settings     → LOW
 *   - unknown/unparseable routes      → LOW (they simply fail navigation)
 */
export function classifyUriActionRisk(uri: string): UriActionRisk {
  const parsed = parseVaultPilotUri(uri);
  switch (parsed.route) {
    case 'chat/new':
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
      return 'low';
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
  high: '此链接将执行高风险操作：启动 AI 对话（可能触发代理工具执行）或覆盖笔记内容，且无法撤销。',
  medium: '此链接将创建一个新笔记。',
  low: '低风险操作：打开已有内容或跳转页面。',
};

/**
 * Evaluate whether a vaultpilot:// URI needs a confirmation prompt:
 *   - HIGH  → always needsConfirmation, even for trusted sources
 *             (mirrors Obsidian's high-risk-always-confirm policy)
 *   - MEDIUM→ needsConfirmation unless the x-source is in trustedSources
 *   - LOW   → never needs confirmation
 */
export function evaluateUriSafety(uri: string, trustedSources: string[]): UriSafetyEvaluation {
  const risk = classifyUriActionRisk(uri);
  const source = extractSource(uri);

  if (risk === 'high') {
    return { risk, needsConfirmation: true, reason: REASONS.high };
  }
  if (risk === 'medium') {
    const trusted = source !== '' && trustedSources.includes(source);
    return { risk, needsConfirmation: !trusted, reason: REASONS.medium };
  }
  return { risk, needsConfirmation: false, reason: REASONS.low };
}
