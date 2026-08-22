// VaultPilot Clipper — Background Service Worker
// Handles extension icon click, context menu, and API communication.

// Default configuration
const DEFAULTS = {
  apiUrl: 'http://127.0.0.1:10101',
  apiToken: '',
  defaultTags: '',
  defaultCollection: '',
  saveFormat: 'clean', // 'clean' | 'full'
  autoSaveOnClick: false
};

// ─── Initialization ────────────────────────────────────────────

chrome.runtime.onInstalled.addListener(async () => {
  // Set default options if not already set
  const existing = await chrome.storage.sync.get(Object.keys(DEFAULTS));
  const toSet = {};
  for (const [key, value] of Object.entries(DEFAULTS)) {
    if (existing[key] === undefined) {
      toSet[key] = value;
    }
  }
  if (Object.keys(toSet).length > 0) {
    await chrome.storage.sync.set(toSet);
  }

  // Create context menu
  chrome.contextMenus.create({
    id: 'save-to-vaultpilot',
    title: '保存页面到 VaultPilot',
    contexts: ['page', 'link']
  });

  chrome.contextMenus.create({
    id: 'save-selection-to-vaultpilot',
    title: '保存选中内容到 VaultPilot',
    contexts: ['selection']
  });
});

// ─── Context Menu Handlers ─────────────────────────────────────

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  if (info.menuItemId === 'save-to-vaultpilot') {
    await saveCurrentPage(tab.id);
  } else if (info.menuItemId === 'save-selection-to-vaultpilot') {
    await saveSelection(tab.id, info.selectionText);
  }
});

// ─── Extension Icon Click ──────────────────────────────────────

chrome.action.onClicked.addListener(async (tab) => {
  const config = await chrome.storage.sync.get(Object.keys(DEFAULTS));
  if (config.autoSaveOnClick) {
    await saveCurrentPage(tab.id);
  }
  // If auto-save is off, the popup handles it
});

// ─── Core: Save Current Page ───────────────────────────────────

async function saveCurrentPage(tabId) {
  try {
    // Inject content script (if not already) and request content extraction
    const results = await chrome.scripting.executeScript({
      target: { tabId },
      func: extractPageContentFromTab
    });

    const content = results[0]?.result;
    if (!content) {
      notifyUser('无法提取页面内容');
      return;
    }

    await saveToVault(content);
    notifyUser('已保存到 VaultPilot ✓');
  } catch (error) {
    console.error('saveCurrentPage failed:', error);
    notifyUser('保存失败: ' + error.message);
  }
}

// ─── Core: Save Selection ──────────────────────────────────────

async function saveSelection(tabId, selectionText) {
  try {
    const results = await chrome.scripting.executeScript({
      target: { tabId },
      func: () => ({
        title: document.title,
        url: window.location.href
      })
    });

    const pageInfo = results[0]?.result || { title: '', url: '' };

    const content = {
      title: `选中: ${pageInfo.title}`,
      url: pageInfo.url,
      description: '',
      author: '',
      siteName: new URL(pageInfo.url).hostname,
      bodyText: selectionText,
      textLength: selectionText.length,
      extractedAt: new Date().toISOString()
    };

    await saveToVault(content);
    notifyUser('已保存到 VaultPilot ✓');
  } catch (error) {
    console.error('saveSelection failed:', error);
    notifyUser('保存失败: ' + error.message);
  }
}

// ─── API Call ──────────────────────────────────────────────────

async function saveToVault(content) {
  const config = await chrome.storage.sync.get(Object.keys(DEFAULTS));
  const apiUrl = config.apiUrl.replace(/\/+$/, '');

  // Build tags from config + auto-generated
  const tags = [];
  if (config.defaultTags) {
    config.defaultTags.split(',').map(t => t.trim()).filter(Boolean).forEach(t => tags.push(t));
  }
  tags.push('clipper');

  // Build the note body
  const noteBody = [
    `> **来源**: [${content.title}](${content.url})`,
    content.description ? `> **描述**: ${content.description}` : '',
    content.author ? `> **作者**: ${content.author}` : '',
    `> **提取时间**: ${content.extractedAt}`,
    '',
    '---',
    '',
    content.bodyText
  ].filter(Boolean).join('\\n');

  const payload = {
    title: content.title,
    body: noteBody,
    source: content.siteName || new URL(content.url).hostname,
    sourceUrl: content.url,
    tags,
    collectionId: config.defaultCollection || ''
  };

  const headers = {
    'Content-Type': 'application/json'
  };

  if (config.apiToken) {
    headers['Authorization'] = 'Bearer ' + config.apiToken;
  }

  const response = await fetch(apiUrl + '/api/notes', {
    method: 'POST',
    headers,
    body: JSON.stringify(payload)
  });

  if (!response.ok) {
    const errorText = await response.text();
    throw new Error(`HTTP ${response.status}: ${errorText}`);
  }

  return response.json();
}

// ─── Helpers ───────────────────────────────────────────────────

/**
 * This function is injected into the tab via executeScript.
 * It returns the extracted page content to the background script.
 */
function extractPageContentFromTab() {
  const url = window.location.href;
  const title = document.title;

  function getMeta(name) {
    const selectors = [
      `meta[name="${name}"]`,
      `meta[property="${name}"]`,
      `meta[name="og:${name}"]`,
      `meta[property="og:${name}"]`
    ];
    for (const sel of selectors) {
      const el = document.querySelector(sel);
      if (el && el.content) return el.content;
    }
    return '';
  }

  const description = getMeta('description');
  const author = getMeta('author');
  const siteName = getMeta('og:site_name') || getMeta('twitter:site') || new URL(url).hostname;

  // Clone and clean
  const clone = document.body.cloneNode(true);
  const removals = clone.querySelectorAll(
    'script, style, noscript, iframe, svg, nav, footer, header, aside, ' +
    '.sidebar, .nav, .footer, .header, .menu, .ad, .advertisement, .ads'
  );
  removals.forEach(el => el.remove());

  // Find main content
  let content = clone;
  const article = document.querySelector('article');
  if (article) {
    content = article.cloneNode(true);
  }

  const innerRemovals = content.querySelectorAll('script, style, noscript, iframe, svg, button, .ad, .ads');
  innerRemovals.forEach(el => el.remove());

  const bodyText = content.textContent
    .replace(/\\s+/g, ' ')
    .replace(/\\n{3,}/g, '\\n\\n')
    .trim()
    .substring(0, 50000);

  return {
    title,
    url,
    description,
    author,
    siteName,
    bodyText,
    textLength: bodyText.length,
    extractedAt: new Date().toISOString()
  };
}

function notifyUser(message) {
  // Use chrome.notifications API if available
  if (chrome.notifications) {
    chrome.notifications.create({
      type: 'basic',
      iconUrl: 'icons/icon128.png',
      title: 'VaultPilot Clipper',
      message: message
    });
  }
}
