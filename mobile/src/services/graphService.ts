/**
 * 后端知识图谱拉取 —— #3832 mobile 部分。
 *
 * 复用 services/sync.ts 的后端连接模式：从 AsyncStorage 读取
 * `cfg_backend_url` / `cfg_backend_token`（与 sync 相同的键名），
 * 通过 fetch 请求 GET /api/graph（见 http_bridge.rs `http_graph`）。
 *
 * 优雅降级：未配置后端 / 请求失败 / 返回非预期结构时一律返回 null，
 * 由调用方展示"未连接后端"空态，绝不抛出异常。
 */

import AsyncStorage from '@react-native-async-storage/async-storage';
import type { KnowledgeGraph } from '../utils/relatedNotes';

const SERVER_URL_KEY = 'cfg_backend_url';
const SERVER_TOKEN_KEY = 'cfg_backend_token';

const GRAPH_TIMEOUT_MS = 8000;

/**
 * 从后端拉取完整知识图谱 JSON。
 * 返回 null 表示不可用（未配置后端 / 网络失败 / 非 2xx / JSON 结构不符）。
 */
export async function fetchKnowledgeGraph(timeoutMs: number = GRAPH_TIMEOUT_MS): Promise<KnowledgeGraph | null> {
  let url: string | null = null;
  let token: string | null = null;
  try {
    [url, token] = await Promise.all([
      AsyncStorage.getItem(SERVER_URL_KEY),
      AsyncStorage.getItem(SERVER_TOKEN_KEY),
    ]);
  } catch (e) {
    console.warn('[Graph] failed to read server config:', e);
    return null;
  }
  if (!url) return null; // 未配置后端

  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort('graph-timeout'), timeoutMs);
  try {
    const headers: Record<string, string> = { Accept: 'application/json' };
    if (token) headers.Authorization = `Bearer ${token}`;
    const res = await fetch(`${url}/api/graph`, { headers, signal: controller.signal });
    if (!res.ok) return null;
    const data: unknown = await res.json();
    if (!data || typeof data !== 'object') return null;
    const graph = data as Partial<KnowledgeGraph>;
    if (!Array.isArray(graph.nodes) || !Array.isArray(graph.edges)) return null;
    return graph as KnowledgeGraph;
  } catch (e) {
    console.warn('[Graph] fetchKnowledgeGraph failed:', e);
    return null;
  } finally {
    clearTimeout(timer);
  }
}
