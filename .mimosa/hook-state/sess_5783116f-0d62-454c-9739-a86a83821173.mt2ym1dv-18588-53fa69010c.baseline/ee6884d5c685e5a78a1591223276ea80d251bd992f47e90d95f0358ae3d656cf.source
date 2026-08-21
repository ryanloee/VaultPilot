// VaultPilot Clipper — Options Page Script

const DEFAULTS = {
  apiUrl: 'http://127.0.0.1:10101',
  apiToken: '',
  defaultTags: '',
  defaultCollection: '',
  saveFormat: 'clean',
  autoSaveOnClick: false
};

document.addEventListener('DOMContentLoaded', async () => {
  // Load current settings
  const config = await chrome.storage.sync.get(Object.keys(DEFAULTS));
  document.getElementById('apiUrl').value = config.apiUrl || DEFAULTS.apiUrl;
  document.getElementById('apiToken').value = config.apiToken || '';
  document.getElementById('defaultTags').value = config.defaultTags || '';
  document.getElementById('defaultCollection').value = config.defaultCollection || '';
  document.getElementById('saveFormat').value = config.saveFormat || 'clean';
  document.getElementById('autoSaveOnClick').checked = config.autoSaveOnClick || false;

  // Test connection
  document.getElementById('testBtn').addEventListener('click', async () => {
    const btn = document.getElementById('testBtn');
    const result = document.getElementById('testResult');
    btn.disabled = true;
    result.textContent = '检测中...';
    result.className = 'status-badge pending';

    const apiUrl = document.getElementById('apiUrl').value.replace(/\/+$/, '');
    const apiToken = document.getElementById('apiToken').value;

    try {
      const headers = {};
      if (apiToken) headers['Authorization'] = 'Bearer ' + apiToken;
      const resp = await fetch(apiUrl + '/health', { headers, signal: AbortSignal.timeout(5000) });
      if (resp.ok) {
        result.textContent = '✓ 已连接';
        result.className = 'status-badge ok';
      } else {
        result.textContent = '✗ HTTP ' + resp.status;
        result.className = 'status-badge error';
      }
    } catch (e) {
      result.textContent = '✗ ' + e.message;
      result.className = 'status-badge error';
    } finally {
      btn.disabled = false;
    }
  });

  // Save settings
  document.getElementById('saveBtn').addEventListener('click', async () => {
    const config = {
      apiUrl: document.getElementById('apiUrl').value.trim() || DEFAULTS.apiUrl,
      apiToken: document.getElementById('apiToken').value.trim(),
      defaultTags: document.getElementById('defaultTags').value.trim(),
      defaultCollection: document.getElementById('defaultCollection').value.trim(),
      saveFormat: document.getElementById('saveFormat').value,
      autoSaveOnClick: document.getElementById('autoSaveOnClick').checked
    };

    await chrome.storage.sync.set(config);
    showToast('设置已保存 ✓');
  });

  // Reset to defaults
  document.getElementById('resetBtn').addEventListener('click', async () => {
    await chrome.storage.sync.set(DEFAULTS);
    document.getElementById('apiUrl').value = DEFAULTS.apiUrl;
    document.getElementById('apiToken').value = '';
    document.getElementById('defaultTags').value = '';
    document.getElementById('defaultCollection').value = '';
    document.getElementById('saveFormat').value = DEFAULTS.saveFormat;
    document.getElementById('autoSaveOnClick').checked = false;
    showToast('已恢复默认设置');
  });
});

function showToast(message) {
  const toast = document.getElementById('toast');
  toast.textContent = message;
  toast.classList.add('show');
  setTimeout(() => toast.classList.remove('show'), 3000);
}
