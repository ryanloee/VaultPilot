/**
 * VaultPilot Clipper — Popup Script
 *
 * Manages the extension popup UI: displays page info, handles clip button,
 * manages settings panel.
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

// ─── DOM refs ──────────────────────────────────────────────────────

const statusDot = document.getElementById('statusDot') as HTMLSpanElement;
const pageTitle = document.getElementById('pageTitle') as HTMLDivElement;
const connectionStatus = document.getElementById('connectionStatus') as HTMLDivElement;
const clipBtn = document.getElementById('clipBtn') as HTMLButtonElement;
const resultDiv = document.getElementById('result') as HTMLDivElement;
const settingsToggle = document.getElementById('settingsToggle') as HTMLAnchorElement;
const settingsPanel = document.getElementById('settingsPanel') as HTMLDivElement;
const vpHost = document.getElementById('vpHost') as HTMLInputElement;
const vpPort = document.getElementById('vpPort') as HTMLInputElement;
const vpToken = document.getElementById('vpToken') as HTMLInputElement;
const saveConfigBtn = document.getElementById('saveConfigBtn') as HTMLButtonElement;

// ─── State ─────────────────────────────────────────────────────────

let popupCachedContent: PageContent | null = null;

// ─── UI helpers ────────────────────────────────────────────────────

function setStatus(connected: boolean | 'loading') {
  statusDot.className = 'status-dot';
  if (connected === 'loading') {
    statusDot.classList.add('loading');
    connectionStatus.textContent = '检查中…';
  } else if (connected) {
    statusDot.classList.add('connected');
    connectionStatus.textContent = '已连接';
    connectionStatus.style.color = '#4ade80';
  } else {
    statusDot.classList.add('disconnected');
    connectionStatus.textContent = '未连接（请启动 vaultpilot-cli serve）';
    connectionStatus.style.color = '#f87171';
  }
}

function showResult(success: boolean, message: string, _noteId?: string, noteTitle?: string) {
  resultDiv.className = 'result ' + (success ? 'success' : 'error');
  if (success && noteTitle) {
    resultDiv.innerHTML = `✅ 已剪藏「<strong>${escapeHtml(noteTitle)}</strong>」到 VaultPilot`;
  } else {
    resultDiv.textContent = message;
  }
  resultDiv.style.display = 'block';
}

function escapeHtml(text: string): string {
  const div = document.createElement('div');
  div.textContent = text;
  return div.innerHTML;
}

// ─── Connection check ──────────────────────────────────────────────

async function checkConnection(): Promise<boolean> {
  try {
    const config = await chrome.runtime.sendMessage<{ action: string }, VaultPilotConfig>({ action: 'getConfig' });
    if (!config || typeof config !== 'object' || !('host' in config)) return false;
    const baseUrl = `http://${config.host}:${config.port}`;
    const healthResp = await fetch(`${baseUrl}/health`);
    return healthResp.ok;
  } catch {
    return false;
  }
}

// ─── Initialization ────────────────────────────────────────────────

async function init() {
  setStatus('loading');

  // Load cached content from background
  try {
    const content = await chrome.runtime.sendMessage<{ action: string }, PageContent | null>({ action: 'getCachedContent' });
    if (content) {
      popupCachedContent = content;
      pageTitle.textContent = content.title || '(无标题)';
    } else {
      pageTitle.textContent = '(未缓存，请刷新页面)';
    }
  } catch {
    pageTitle.textContent = '(无法获取页面信息)';
  }

  // Check connection
  const connected = await checkConnection();
  setStatus(connected);

  // Load settings
  try {
    const config = await chrome.runtime.sendMessage<{ action: string }, VaultPilotConfig>({ action: 'getConfig' });
    if (config) {
      vpHost.value = config.host || '127.0.0.1';
      vpPort.value = String(config.port || 8765);
      vpToken.value = config.token || '';
    }
  } catch {
    // Defaults
    vpHost.value = '127.0.0.1';
    vpPort.value = '8765';
  }
}

// ─── Event handlers ────────────────────────────────────────────────

clipBtn.addEventListener('click', async () => {
  clipBtn.disabled = true;
  clipBtn.textContent = '⏳ 正在剪藏…';
  resultDiv.style.display = 'none';

  try {
    const result = await chrome.runtime.sendMessage<{ action: string }, ClipResult>({ action: 'clipToVault' });
    if (result) {
      showResult(result.success, result.error || '剪藏成功', result.noteId, result.title);
    } else {
      showResult(false, '未收到响应');
    }
  } catch (error) {
    showResult(false, `剪藏失败: ${error}`);
  } finally {
    clipBtn.disabled = false;
    clipBtn.textContent = '📎 剪藏到 VaultPilot';
  }
});

settingsToggle.addEventListener('click', () => {
  settingsPanel.classList.toggle('open');
});

saveConfigBtn.addEventListener('click', async () => {
  const config: VaultPilotConfig = {
    host: vpHost.value.trim() || '127.0.0.1',
    port: parseInt(vpPort.value) || 8765,
    token: vpToken.value.trim(),
  };

  try {
    await chrome.runtime.sendMessage({ action: 'saveConfig', data: config });
    saveConfigBtn.textContent = '✅ 已保存';
    setTimeout(() => { saveConfigBtn.textContent = '保存设置'; }, 2000);
    // Re-check connection
    setStatus('loading');
    const connected = await checkConnection();
    setStatus(connected);
  } catch {
    saveConfigBtn.textContent = '❌ 保存失败';
  }
});

// ─── Start ─────────────────────────────────────────────────────────

init();
