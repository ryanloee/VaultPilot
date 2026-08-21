/**
 * VaultPilot Clipper — Background Service Worker
 *
 * Handles:
 * 1. Caching page content from content script
 * 2. Making API calls to VaultPilot HTTP Bridge
 * 3. Communicating with the popup UI
 */

// Module scope to avoid redeclaration conflicts
export {};

// ─── Types ─────────────────────────────────────────────────────────

interface PageContent {
  title: string;
  url: string;
  content: string;
  excerpt: string;
}

interface ClipResult {
  success: boolean;
  noteId?: string;
  title?: string;
  error?: string;
}

interface VaultPilotConfig {
  host: string;
  port: number;
  token: string;
}

// ─── State ─────────────────────────────────────────────────────────

let bgCachedContent: PageContent | null = null;
const DEFAULT_HOST = '127.0.0.1';
const DEFAULT_PORT = 8765;

// ─── Storage helpers ───────────────────────────────────────────────

async function getConfig(): Promise<VaultPilotConfig> {
  const result = await chrome.storage.sync.get(['vpHost', 'vpPort', 'vpToken']);
  return {
    host: result.vpHost || DEFAULT_HOST,
    port: result.vpPort || DEFAULT_PORT,
    token: result.vpToken || '',
  };
}

// ─── API Call ──────────────────────────────────────────────────────

async function clipToVault(content: PageContent): Promise<ClipResult> {
  const config = await getConfig();
  const baseUrl = `http://${config.host}:${config.port}`;

  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
  };
  if (config.token) {
    headers['Authorization'] = `Bearer ${config.token}`;
  }

  try {
    const response = await fetch(`${baseUrl}/api/notes`, {
      method: 'POST',
      headers,
      body: JSON.stringify({
        title: content.title,
        content: content.content,
        sourceUrl: content.url,
        tags: 'clipped',
      }),
    });

    if (!response.ok) {
      const errorBody = await response.text().catch(() => 'unknown error');
      let message: string;
      switch (response.status) {
        case 401:
          message = '认证失败：请检查 API Token 配置';
          break;
        case 413:
          message = '内容过大：页面内容超过 10MB 限制';
          break;
        case 429:
          message = '请求过于频繁：请稍后再试';
          break;
        default:
          message = `请求失败 (${response.status}): ${errorBody}`;
      }
      return { success: false, error: message };
    }

    const data = await response.json();
    return {
      success: true,
      noteId: data.id,
      title: data.title,
    };
  } catch (error) {
    const message = error instanceof TypeError && error.message === 'Failed to fetch'
      ? `无法连接到 VaultPilot (${baseUrl})，请确认 HTTP Bridge 已启动`
      : String(error);
    return { success: false, error: message };
  }
}

// ─── Message handling ──────────────────────────────────────────────

chrome.runtime.onMessage.addListener((
  message: { action: string; data?: unknown },
  _sender: chrome.runtime.MessageSender,
  sendResponse: (response: unknown) => void
) => {
  switch (message.action) {
    case 'cacheContent':
      bgCachedContent = message.data as PageContent;
      sendResponse({ ok: true });
      break;

    case 'getCachedContent':
      sendResponse(bgCachedContent);
      break;

    case 'clipToVault':
      if (bgCachedContent) {
        clipToVault(bgCachedContent).then(sendResponse);
        return true; // Keep channel open for async response
      }
      sendResponse({ success: false, error: '没有缓存的页面内容' });
      break;

    case 'getConfig':
      getConfig().then(sendResponse);
      return true;

    case 'saveConfig':
      if (message.data && typeof message.data === 'object') {
        const config = message.data as Partial<VaultPilotConfig>;
        chrome.storage.sync.set({
          vpHost: config.host || DEFAULT_HOST,
          vpPort: config.port || DEFAULT_PORT,
          vpToken: config.token || '',
        }).then(() => sendResponse({ ok: true }));
      } else {
        sendResponse({ ok: false, error: '无效配置' });
      }
      return true;

    default:
      sendResponse({ error: `unknown action: ${message.action}` });
  }
  return false;
});

// ─── Extension icon click ──────────────────────────────────────────

chrome.action.onClicked.addListener(async (tab: chrome.tabs.Tab) => {
  if (!tab.id) return;

  try {
    const response = await chrome.tabs.sendMessage<{ action: string }, PageContent>(
      tab.id,
      { action: 'getPageContent' }
    );
    if (response && 'title' in response && 'content' in response) {
      bgCachedContent = response;
      const result = await clipToVault(response);
      if (result.success) {
        chrome.notifications.create({
          type: 'basic',
          iconUrl: 'icons/icon128.png',
          title: '已剪藏到 VaultPilot',
          message: `「${result.title}」已保存为笔记`,
        });
      } else {
        chrome.notifications.create({
          type: 'basic',
          iconUrl: 'icons/icon128.png',
          title: '剪藏失败',
          message: result.error || '未知错误',
        });
      }
    }
  } catch (error) {
    chrome.notifications.create({
      type: 'basic',
      iconUrl: 'icons/icon128.png',
      title: '剪藏失败',
      message: `无法读取页面内容: ${error}`,
    });
  }
});

// Log install
chrome.runtime.onInstalled.addListener(() => {
  console.log('VaultPilot Clipper installed');
});
