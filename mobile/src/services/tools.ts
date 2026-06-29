/**
 * tools.ts — @tool 命令执行服务
 *
 * 处理 Chat 中的 @tool 命令（@vault、@web、@url、@youtube）。
 * 每个工具函数接收查询参数，返回格式化的结果文本。
 */

/* ── 工具结果 ── */

export interface ToolResult {
  summary: string;      // 给用户看到的简短结果描述
  content: string;      // 完整结果内容
  sourceUrl?: string;   // 来源链接（可选）
}

/* ── 工具调度 ── */

const TOOL_PATTERN = /^@(\w+):\s*(.+)/s;

/**
 * 判断输入是否为 @tool 命令。
 * 返回匹配的工具 id 和查询参数，或 null。
 */
export function parseToolCommand(input: string): { toolId: string; query: string } | null {
  const match = input.trim().match(TOOL_PATTERN);
  if (!match) return null;
  return { toolId: match[1].toLowerCase(), query: match[2].trim() };
}

/* ── @vault: 搜索笔记 ── */

/**
 * @vault: keyword — 搜索本地 vault 笔记
 * 使用现有的 DB searchNotes 功能。
 */
import { searchNotes, globalSearch } from '../db';

export async function executeVault(query: string): Promise<ToolResult> {
  if (!query) {
    return { summary: '请提供搜索关键词', content: '用法：@vault: 关键词' };
  }

  // Try globalSearch first (combines notes + messages), fallback to searchNotes
  let results: any[] = [];
  try {
    const global = await globalSearch(query);
    if (global && global.length > 0) {
      results = global;
    }
  } catch { /* fallthrough */ }

  if (results.length === 0) {
    try {
      results = await searchNotes(query);
    } catch { /* fallthrough */ }
  }

  if (!results || results.length === 0) {
    return {
      summary: `未找到与 "${query}" 相关的笔记`,
      content: `未找到与 "${query}" 相关的笔记。\n\n提示：可以尝试不同的关键词，或检查笔记是否已保存。`,
    };
  }

  const lines = results.slice(0, 8).map((r: any, i: number) => {
    const title = r.title || r.note_title || '无标题';
    const snippet = (r.content || r.snippet || '').slice(0, 200);
    return `${i + 1}. **${title}**\n   ${snippet}`;
  });

  return {
    summary: `找到 ${results.length} 条相关笔记（显示前 ${Math.min(8, results.length)} 条）`,
    content: lines.join('\n\n'),
  };
}

/* ── @web: Web 搜索 ── */

/**
 * @web: query — 实时 Web 搜索
 * 使用 DuckDuckGo Instant Answer API（免费，无需 API Key）。
 * 回退：使用 DuckDuckGo HTML 搜索。
 */
export async function executeWebSearch(query: string): Promise<ToolResult> {
  if (!query) {
    return { summary: '请提供搜索内容', content: '用法：@web: 搜索内容' };
  }

  try {
    const encoded = encodeURIComponent(query);
    const url = `https://api.duckduckgo.com/?q=${encoded}&format=json&no_html=1&skip_disambig=1`;
    const resp = await fetch(url, {
      // Short timeout to avoid hanging
      signal: AbortSignal.timeout(10000),
    });

    if (!resp.ok) {
      throw new Error(`DuckDuckGo API returned ${resp.status}`);
    }

    const data: any = await resp.json();

    // Build result from AbstractText + RelatedTopics
    const parts: string[] = [];

    if (data.AbstractText) {
      parts.push(`**摘要**\n${data.AbstractText}`);
    }

    if (data.AbstractURL) {
      parts.push(`来源：${data.AbstractURL}`);
    }

    if (data.RelatedTopics && Array.isArray(data.RelatedTopics)) {
      const topics = data.RelatedTopics
        .filter((t: any) => t.Text && !t.Name) // skip category headers
        .slice(0, 5);

      if (topics.length > 0) {
        const topicLines = topics.map((t: any, i: number) => {
          const link = t.FirstURL ? `\n  ${t.FirstURL}` : '';
          return `${i + 1}. ${t.Text}${link}`;
        });
        parts.push(`**相关结果**\n${topicLines.join('\n')}`);
      }
    }

    if (parts.length === 0) {
      // Fallback: try DuckDuckGo lite HTML endpoint
      return await webSearchFallbackHtml(query);
    }

    return {
      summary: `"${query}" 搜索结果`,
      content: parts.join('\n\n'),
      sourceUrl: data.AbstractURL || `https://duckduckgo.com/?q=${encoded}`,
    };
  } catch (err: any) {
    if (err.name === 'TimeoutError' || err.name === 'AbortError') {
      return {
        summary: `"${query}" 搜索超时`,
        content: `搜索 "${query}" 超时（10秒）。\n\n请检查网络连接后重试。`,
      };
    }
    // Fallback
    return await webSearchFallbackHtml(query);
  }
}

/** Fallback: parse DuckDuckGo lite HTML result page */
async function webSearchFallbackHtml(query: string): Promise<ToolResult> {
  try {
    const encoded = encodeURIComponent(query);
    const resp = await fetch(`https://lite.duckduckgo.com/lite/?q=${encoded}`, {
      signal: AbortSignal.timeout(8000),
    });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);

    const html = await resp.text();

    // Extract search result rows from the lite HTML
    const results: string[] = [];
    // Simple regex extraction of result links
    const linkRegex = /<a[^>]+href="([^"]+)"[^>]*>([^<]+)<\/a>/g;
    let match;
    let count = 0;
    while ((match = linkRegex.exec(html)) !== null && count < 5) {
      const href = match[1];
      const text = match[2].replace(/<[^>]+>/g, '').trim();
      if (href && text && !href.startsWith('#') && !href.startsWith('/')) {
        results.push(`${text}\n  ${href}`);
        count++;
      }
    }

    if (results.length > 0) {
      return {
        summary: `"${query}" 搜索结果（HTML 回退）`,
        content: results.join('\n\n'),
        sourceUrl: `https://duckduckgo.com/?q=${encoded}`,
      };
    }

    return {
      summary: `"${query}" 搜索无结果`,
      content: `未找到 "${query}" 的搜索结果。\n\n提示：请检查网络连接或尝试不同的关键词。`,
    };
  } catch {
    return {
      summary: `"${query}" 搜索失败`,
      content: `搜索 "${query}" 时出错。\n\n请检查网络连接后重试。\n\n如果问题持续，可以尝试 @url 工具直接查看相关网页。`,
    };
  }
}

/* ── @url: 网页内容提取 ── */

/**
 * @url: https://... — 提取网页内容并生成摘要
 * 使用 fetch 获取网页 HTML，提取文本内容。
 */
export async function executeUrlExtract(url: string): Promise<ToolResult> {
  if (!url) {
    return { summary: '请提供网页链接', content: '用法：@url: https://example.com/...' };
  }

  // Basic URL validation
  let normalizedUrl = url.trim();
  if (!normalizedUrl.startsWith('http://') && !normalizedUrl.startsWith('https://')) {
    normalizedUrl = 'https://' + normalizedUrl;
  }

  try {
    new URL(normalizedUrl);
  } catch {
    return {
      summary: '无效的链接',
      content: `"${url}" 不是一个有效的网页链接。\n\n请提供完整的 URL，例如：@url: https://example.com/article`,
    };
  }

  try {
    const resp = await fetch(normalizedUrl, {
      signal: AbortSignal.timeout(15000),
      headers: {
        'User-Agent': 'Mozilla/5.0 (compatible; VaultPilot/1.0; +https://github.com/ryanloee/VaultPilot)',
      },
    });

    if (!resp.ok) {
      throw new Error(`HTTP ${resp.status}: ${resp.statusText}`);
    }

    const html = await resp.text();

    // Extract text content from HTML
    const text = extractTextFromHtml(html);

    if (!text || text.length < 20) {
      return {
        summary: `无法提取 ${normalizedUrl} 的内容`,
        content: `已连接到 ${normalizedUrl}，但无法提取有意义的文字内容。\n\n该页面可能包含大量 JavaScript 动态内容（如单页应用），或需要登录才能访问。`,
        sourceUrl: normalizedUrl,
      };
    }

    // Get page title
    const titleMatch = html.match(/<title[^>]*>([^<]+)<\/title>/i);
    const pageTitle = titleMatch ? titleMatch[1].trim() : normalizedUrl;

    // Truncate to reasonable length
    const maxLen = 3000;
    const truncated = text.length > maxLen
      ? text.slice(0, maxLen) + `\n\n…[内容过长，仅显示前 ${maxLen} 字符]`
      : text;

    return {
      summary: `📄 ${pageTitle}`,
      content: `**${pageTitle}**\n\n${truncated}`,
      sourceUrl: normalizedUrl,
    };
  } catch (err: any) {
    if (err.name === 'TimeoutError' || err.name === 'AbortError') {
      return {
        summary: `获取 ${normalizedUrl} 超时`,
        content: `获取 ${normalizedUrl} 超时（15秒）。\n\n该网页可能加载较慢，或需要更复杂的浏览器环境才能渲染。`,
        sourceUrl: normalizedUrl,
      };
    }
    return {
      summary: `获取 ${normalizedUrl} 失败`,
      content: `无法获取 ${normalizedUrl} 的内容：${err.message}\n\n提示：该网页可能需要登录，或阻止了自动化访问。`,
      sourceUrl: normalizedUrl,
    };
  }
}

/** Strip HTML tags and extract readable text */
function extractTextFromHtml(html: string): string {
  // Remove script and style blocks
  let text = html.replace(/<script[^>]*>[\s\S]*?<\/script>/gi, ' ');
  text = text.replace(/<style[^>]*>[\s\S]*?<\/style>/gi, ' ');
  text = text.replace(/<nav[^>]*>[\s\S]*?<\/nav>/gi, ' ');
  text = text.replace(/<footer[^>]*>[\s\S]*?<\/footer>/gi, ' ');

  // Replace br and block elements with newlines
  text = text.replace(/<br\s*\/?>/gi, '\n');
  text = text.replace(/<\/(p|div|h[1-6]|li|tr|blockquote|section|article)>/gi, '\n');

  // Strip remaining tags
  text = text.replace(/<[^>]+>/g, '');

  // Decode common entities
  text = text.replace(/&amp;/g, '&');
  text = text.replace(/&lt;/g, '<');
  text = text.replace(/&gt;/g, '>');
  text = text.replace(/&quot;/g, '"');
  text = text.replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(parseInt(code, 10)));
  text = text.replace(/&#x([0-9a-f]+);/gi, (_, hex) => String.fromCodePoint(parseInt(hex, 16)));

  // Collapse whitespace
  text = text.replace(/\n\s*\n/g, '\n\n');
  text = text.replace(/[ \t]+/g, ' ');
  text = text.replace(/^\s+|\s+$/gm, '');

  return text.trim();
}

/* ── @youtube: YouTube 视频转录 ── */

/**
 * @youtube: https://youtube.com/watch?v=... — 提取 YouTube 视频转录文本
 * 使用 youtubetranscript.com 免费 API。
 */
export async function executeYoutube(url: string): Promise<ToolResult> {
  if (!url) {
    return { summary: '请提供 YouTube 视频链接', content: '用法：@youtube: https://youtube.com/watch?v=...' };
  }

  // Extract video ID
  const videoId = extractYoutubeId(url.trim());
  if (!videoId) {
    return {
      summary: '无法识别 YouTube 链接',
      content: `"${url}" 不是有效的 YouTube 视频链接。\n\n支持的格式：\n- https://youtube.com/watch?v=VIDEO_ID\n- https://youtu.be/VIDEO_ID\n- https://m.youtube.com/watch?v=VIDEO_ID`,
    };
  }

  try {
    // Get video page for title
    const pageResp = await fetch(`https://www.youtube.com/watch?v=${videoId}`, {
      signal: AbortSignal.timeout(8000),
      headers: { 'User-Agent': 'Mozilla/5.0' },
    });
    const pageHtml = await pageResp.text();
    const titleMatch = pageHtml.match(/<title[^>]*>([^<]+)<\/title>/i);
    const videoTitle = titleMatch
      ? titleMatch[1].trim().replace(' - YouTube', '')
      : `YouTube 视频 ${videoId}`;

    // Fetch transcript via youtubetranscript.com
    const transcriptUrl = `https://youtubetranscript.com/?v=${videoId}`;
    const resp = await fetch(transcriptUrl, {
      signal: AbortSignal.timeout(10000),
    });

    if (!resp.ok) {
      return {
        summary: `无法获取 "${videoTitle}" 的转录文本`,
        content: `视频：${videoTitle}\n链接：https://youtube.com/watch?v=${videoId}\n\n无法获取转录文本。\n\n可能的原因：\n- 视频没有字幕/CC\n- 视频语言不支持自动转录\n- 视频长度超过限制`,
        sourceUrl: `https://youtube.com/watch?v=${videoId}`,
      };
    }

    const text = await resp.text();

    // The API returns XML or JSON depending on format
    // Try to parse as JSON first (newer format)
    let transcript: string;
    try {
      const json = JSON.parse(text);
      if (Array.isArray(json)) {
        transcript = json.map((seg: any) => seg.text || '').join(' ');
      } else {
        transcript = text;
      }
    } catch {
      // If not JSON, treat as plain text (remove XML tags if any)
      transcript = text.replace(/<[^>]+>/g, ' ').replace(/\s+/g, ' ').trim();
    }

    // Decode HTML entities in transcript
    transcript = transcript
      .replace(/&amp;#39;/g, "'")
      .replace(/&amp;/g, '&')
      .replace(/&lt;/g, '<')
      .replace(/&gt;/g, '>')
      .replace(/&#39;/g, "'")
      .replace(/&quot;/g, '"');

    const maxLen = 3000;
    const truncated = transcript.length > maxLen
      ? transcript.slice(0, maxLen) + `\n\n…[转录过长，仅显示前 ${maxLen} 字符]`
      : transcript;

    return {
      summary: `🎬 ${videoTitle}`,
      content: `**${videoTitle}**\n\n${truncated}`,
      sourceUrl: `https://youtube.com/watch?v=${videoId}`,
    };
  } catch (err: any) {
    return {
      summary: `获取视频转录失败`,
      content: `无法获取视频转录：${err.message}\n\n提示：可以尝试 @url 工具手动提取视频页面内容。`,
      sourceUrl: `https://youtube.com/watch?v=${videoId}`,
    };
  }
}

/** Extract YouTube video ID from various URL formats */
function extractYoutubeId(url: string): string | null {
  const patterns = [
    /(?:youtube\.com\/watch\?v=|youtu\.be\/|youtube\.com\/embed\/|youtube\.com\/v\/)([a-zA-Z0-9_-]{11})/,
    /^([a-zA-Z0-9_-]{11})$/,
  ];

  for (const p of patterns) {
    const match = url.match(p);
    if (match) return match[1];
  }
  return null;
}

/* ── 统一调度 ── */

/**
 * 根据工具 ID 和查询参数执行对应的工具。
 */
export async function executeTool(toolId: string, query: string): Promise<ToolResult> {
  switch (toolId) {
    case 'vault':
      return executeVault(query);
    case 'web':
      return executeWebSearch(query);
    case 'url':
      return executeUrlExtract(query);
    case 'youtube':
      return executeYoutube(query);
    default:
      return {
        summary: `未知工具 @${toolId}`,
        content: `未知工具：@${toolId}\n\n可用工具：\n- @vault — 搜索 vault 笔记\n- @web — 实时 Web 搜索\n- @url — 提取网页内容并总结\n- @youtube — 提取 YouTube 视频内容`,
      };
  }
}

/** Check if a string matches any @tool command pattern */
export function isToolCommand(input: string): boolean {
  return parseToolCommand(input) !== null;
}
