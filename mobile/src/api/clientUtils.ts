/**
 * Pure utility functions extracted from client.ts for testability.
 *
 * Zero React/RN dependencies — can be unit-tested in plain Jest.
 */

/** Default API settings */
export const DEFAULTS = {
  apiBase: 'https://opencode.ai/zen/v1',
  model: 'deepseek-v4-flash-free',
} as const;

/**
 * Check if an HTTP status code is retryable (transient server/rate-limit errors).
 */
export function isRetryable(status: number): boolean {
  return status === 408 || status === 429 || status === 502 || status === 503 || status === 504;
}

/** Friendly error messages for common HTTP status codes */
const STATUS_MESSAGES: Record<number, string> = {
  400: '请求格式错误',
  401: 'API Key 无效或已过期',
  403: '访问被拒绝，请检查权限',
  404: '请求的资源不存在',
  408: '请求超时，请稍后重试',
  429: '请求过于频繁，请稍后重试',
  500: '服务器内部错误',
  502: '服务暂时不可用',
  503: '服务暂时不可用，请稍后重试',
  504: '服务响应超时，请稍后重试',
};

/**
 * Convert an HTTP status code and raw body into a user-friendly error message.
 * Sanitizes the raw body to prevent leaking API keys or sensitive data.
 */
export function sanitizeApiError(status: number, _rawBody?: string): string {
  const friendly = STATUS_MESSAGES[status];
  if (friendly) return `API 错误 (${status}): ${friendly}`;
  if (status >= 500) return `API 错误 (${status}): 服务端异常，请稍后重试`;
  if (status >= 400) return `API 错误 (${status}): 请求有误，请检查参数`;
  return `API 错误 (${status})`;
}

/**
 * Normalize an API base URL:
 * - Trim whitespace and trailing slashes
 * - Return default if empty
 * - Preserve existing versioned paths (e.g. /v2, /v1-beta)
 * - Append /v1 otherwise
 */
export function normalizeApiBase(raw: string): string {
  const trimmed = raw.trim().replace(/\/+$/, '');
  if (!trimmed) return DEFAULTS.apiBase;
  if (/\/v\d+[\w-]*($|\/)/.test(trimmed)) return trimmed;
  return trimmed + '/v1';
}
