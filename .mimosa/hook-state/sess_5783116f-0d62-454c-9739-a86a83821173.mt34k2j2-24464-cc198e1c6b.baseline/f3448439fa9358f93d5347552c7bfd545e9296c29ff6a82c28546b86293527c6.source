// VaultPilot Clipper — Popup Script

const DEFAULTS = {
  apiUrl: 'http://127.0.0.1:10101',
  apiToken: '',
  defaultTags: '',
  defaultCollection: '',
  saveFormat: 'clean',
  autoSaveOnClick: false
};

document.addEventListener('DOMContentLoaded', async () => {
  const saveBtn = document.getElementById('saveBtn');
  const apiStatus = document.getElementById('apiStatus');
  const spinner = document.getElementById('spinner');
  const successMsg = document.getElementById('successMsg');
  const errorMsg = document.getElementById('errorMsg');

  // Check API connectivity
  const config = await chrome.storage.sync.get(Object.keys(DEFAULTS));
  const apiUrl = config.apiUrl.replace(/\/+$/, '');

  try {
    const headers = {};
    if (config.apiToken) {
      headers['Authorization'] = 'Bearer ' + config.apiToken;
    }
    const resp = await fetch(apiUrl + '/health', { headers, signal: AbortSignal.timeout(3000) });
    if (resp.ok) {
      apiStatus.textContent = '已连接 (' + apiUrl + ')';
      apiStatus.className = 'value ok';
      saveBtn.disabled = false;
    } else {
      apiStatus.textContent = '连接失败 (HTTP ' + resp.status + ')';
      apiStatus.className = 'value error';
    }
  } catch (e) {
    apiStatus.textContent = '无法连接: ' + e.message;
    apiStatus.className = 'value error';
  }

  // Save button
  saveBtn.addEventListener('click', async () => {
    saveBtn.disabled = true;
    spinner.style.display = 'block';
    successMsg.style.display = 'none';
    errorMsg.style.display = 'none';

    try {
      // Get current tab
      const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
      if (!tab?.id) {
        showError('无法获取当前标签页');
        return;
      }

      // Execute content extraction via scripting API
      const results = await chrome.scripting.executeScript({
        target: { tabId: tab.id },
        func: extractPageContent
      });

      const content = results[0]?.result;
      if (!content || !content.bodyText) {
        showError('无法提取页面内容');
        return;
      }

      // Save to Vault
      await saveToVault(content, config);
      spinner.style.display = 'none';
      successMsg.style.display = 'block';
      setTimeout(() => window.close(), 1500);
    } catch (e) {
      console.error('Save failed:', e);
      showError('保存失败: ' + e.message);
      saveBtn.disabled = false;
    }
  });

  // Options buttons
  document.getElementById('optionsBtn').addEventListener('click', () => {
    chrome.runtime.openOptionsPage();
  });

  document.getElementById('openOptions').addEventListener('click', (e) => {
    e.preventDefault();
    chrome.runtime.openOptionsPage();
  });

  function showError(msg) {
    spinner.style.display = 'none';
    errorMsg.textContent = msg;
    errorMsg.style.display = 'block';
  }
});

/**
 * Extract page content (injected into the tab).
 */
function extractPageContent() {
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

  // Clone body and clean
  const clone = document.body.cloneNode(true);
  const removals = clone.querySelectorAll(
    'script, style, noscript, iframe, svg, nav, footer, header, aside, ' +
    '.sidebar, .nav, .footer, .header, .menu, .ad, .advertisement, .ads'
  );
  removals.forEach(el => el.remove());

  // Prefer article element
  let content = clone;
  const article = document.querySelector('article');
  if (article) {
    content = article.cloneNode(true);
  }

  const innerRemovals = content.querySelectorAll('script, style, noscript, iframe, svg, button, .ad, .ads');
  innerRemovals.forEach(el => el.remove());

  const bodyText = content.textContent
    .replace(/\s+/g, ' ')
    .replace(/\n{3,}/g, '\n\n')
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

/**
 * Send page content to VaultPilot API.
 */
async function saveToVault(content, config) {
  const apiUrl = config.apiUrl.replace(/\/+$/, '');

  // Build tags
  const tags = ['clipper'];
  if (config.defaultTags) {
    config.defaultTags.split(',').map(t => t.trim()).filter(Boolean).forEach(t => {
      if (!tags.includes(t)) tags.push(t);
    });
  }

  // Build note body
  const noteBody = [
    `> **来源**: [${content.title}](${content.url})`,
    content.description ? `> **描述**: ${content.description}` : '',
    content.author ? `> **作者**: ${content.author}` : '',
    `> **提取时间**: ${content.extractedAt}`,
    '',
    '---',
    '',
    content.bodyText
  ].filter(Boolean).join('\n');

  const payload = {
    title: content.title,
    body: noteBody,
    source: content.siteName || new URL(content.url).hostname,
    sourceUrl: content.url,
    tags,
    collectionId: config.defaultCollection || ''
  };

  const headers = { 'Content-Type': 'application/json' };
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
    throw new Error('HTTP ' + response.status + ': ' + errorText);
  }

  return response.json();
}
