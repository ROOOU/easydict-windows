// ==================== Tauri API Bridge ====================
if (!window.__TAURI__) {
  document.addEventListener('DOMContentLoaded', () => {
    document.body.innerHTML = '<div style="padding:40px;color:#f87171;font-family:sans-serif;"><h2>⚠️ Tauri API 未加载</h2><p>请确保通过 <code>npx tauri dev</code> 运行应用</p></div>';
  });
  throw new Error('Tauri API not available');
}
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const { getCurrentWindow } = window.__TAURI__.window;

// ==================== DOM Elements ====================
const $ = (sel) => document.querySelector(sel);
const inputText = $('#inputText');
const sourceLang = $('#sourceLang');
const targetLang = $('#targetLang');
const resultsSection = $('#resultsSection');
const loadingBar = $('#loadingBar');
const mainView = $('#mainView');
const settingsView = $('#settingsView');

// ==================== State ====================
let config = null;
let isPinned = false;
let isTranslating = false;
let isOcrInProgress = false;

// ==================== Init ====================
async function init() {
  config = await invoke('get_config');
  const languages = await invoke('get_languages');

  populateLanguageSelects(languages);
  applyConfig(config);
  applyTheme(config.general.theme);
  showEmptyState();
  setupEventListeners();
  setupTauriListeners();

  inputText.focus();
}

function populateLanguageSelects(languages) {
  sourceLang.innerHTML = '';
  targetLang.innerHTML = '';
  for (const lang of languages) {
    const opt1 = new Option(lang.name, lang.code);
    sourceLang.add(opt1);
    if (lang.code !== 'auto') {
      const opt2 = new Option(lang.name, lang.code);
      targetLang.add(opt2);
    }
  }
}

function applyConfig(cfg) {
  sourceLang.value = cfg.general.source_lang || 'auto';
  targetLang.value = cfg.general.target_lang || 'zh-CN';

  // Settings page
  $('#themeSelect').value = cfg.general.theme || 'auto';
  $('#defaultTargetLang').value = cfg.general.target_lang || 'zh-CN';
  $('#googleEnabled').checked = cfg.services.google.enabled;
  $('#bingEnabled').checked = cfg.services.bing.enabled;
  $('#deeplEnabled').checked = cfg.services.deepl.enabled;
  $('#deeplApiKey').value = cfg.services.deepl.api_key || '';
  $('#baiduEnabled').checked = cfg.services.baidu.enabled;
  $('#baiduAppId').value = cfg.services.baidu.app_id || '';
  $('#baiduSecretKey').value = cfg.services.baidu.secret_key || '';
  $('#openaiEnabled').checked = cfg.services.openai.enabled;
  $('#openaiApiKey').value = cfg.services.openai.api_key || '';
  $('#openaiApiUrl').value = cfg.services.openai.api_url || 'https://api.openai.com/v1/chat/completions';
  $('#openaiModel').value = cfg.services.openai.model || 'gpt-4o-mini';

  // Select-translate settings
  if (cfg.select_translate) {
    $('#selectTranslateEnabled').checked = cfg.select_translate.enabled !== false;
    $('#selectTranslateMode').value = cfg.select_translate.mode || 'icon';
  }

  // Hotkey settings
  if (cfg.hotkeys) {
    $('#hotkeyInputEnabled').checked = cfg.hotkeys.input_translate.enabled !== false;
    $('#hotkeyInputShortcut').value = cfg.hotkeys.input_translate.shortcut || '';
    $('#hotkeySelectEnabled').checked = cfg.hotkeys.select_translate.enabled !== false;
    $('#hotkeySelectShortcut').value = cfg.hotkeys.select_translate.shortcut || '';
    $('#hotkeyScreenshotEnabled').checked = cfg.hotkeys.screenshot_translate.enabled !== false;
    $('#hotkeyScreenshotShortcut').value = cfg.hotkeys.screenshot_translate.shortcut || '';
  }
}

function applyTheme(theme) {
  if (theme === 'auto') {
    const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    document.documentElement.setAttribute('data-theme', prefersDark ? 'dark' : 'light');
  } else {
    document.documentElement.setAttribute('data-theme', theme);
  }
}

// ==================== Event Listeners ====================
function setupEventListeners() {
  // Translate
  $('#translateBtn').addEventListener('click', doTranslate);
  inputText.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      doTranslate();
    }
  });

  // Clear
  $('#clearBtn').addEventListener('click', () => {
    inputText.value = '';
    resultsSection.innerHTML = '';
    showEmptyState();
    inputText.focus();
  });

  // Speak input
  $('#speakInputBtn').addEventListener('click', () => {
    const text = inputText.value.trim();
    if (text) invoke('speak', { text });
  });

  // OCR
  $('#ocrBtn').addEventListener('click', doOCR);

  // Swap languages
  $('#swapLangBtn').addEventListener('click', () => {
    const s = sourceLang.value;
    const t = targetLang.value;
    if (s !== 'auto') {
      sourceLang.value = t;
      targetLang.value = s;
    }
  });

  // Pin window
  $('#pinBtn').addEventListener('click', togglePin);

  // Window controls
  $('#minimizeBtn').addEventListener('click', () => getCurrentWindow().minimize());
  $('#closeBtn').addEventListener('click', () => getCurrentWindow().hide());

  // Settings
  $('#settingsBtn').addEventListener('click', () => {
    mainView.classList.add('hidden');
    settingsView.classList.remove('hidden');
  });

  $('#backBtn').addEventListener('click', () => {
    settingsView.classList.add('hidden');
    mainView.classList.remove('hidden');
  });

  // Save settings
  $('#saveSettingsBtn').addEventListener('click', saveSettings);

  // Theme
  $('#themeSelect').addEventListener('change', (e) => {
    applyTheme(e.target.value);
  });

  // System theme changes
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
    if (config && config.general.theme === 'auto') {
      applyTheme('auto');
    }
  });

  // Hotkey recorder
  document.querySelectorAll('.hotkey-input').forEach(input => {
    input.addEventListener('keydown', (e) => {
      e.preventDefault();
      e.stopPropagation();

      // Ignore modifier-only presses
      if (['Control', 'Shift', 'Alt', 'Meta'].includes(e.key)) return;

      const parts = [];
      if (e.ctrlKey) parts.push('Ctrl');
      if (e.altKey) parts.push('Alt');
      if (e.shiftKey) parts.push('Shift');
      if (e.metaKey) parts.push('Super');

      // Need at least one modifier
      if (parts.length === 0) return;

      // Map key name to Tauri format
      let key = e.key;
      if (key.length === 1) {
        key = key.toUpperCase();
      } else {
        // Map common key names
        const keyMap = {
          'ArrowUp': 'Up', 'ArrowDown': 'Down', 'ArrowLeft': 'Left', 'ArrowRight': 'Right',
          'Escape': 'Escape', 'Enter': 'Enter', 'Backspace': 'Backspace', 'Delete': 'Delete',
          'Tab': 'Tab', 'Space': 'Space', ' ': 'Space',
          'F1': 'F1', 'F2': 'F2', 'F3': 'F3', 'F4': 'F4', 'F5': 'F5', 'F6': 'F6',
          'F7': 'F7', 'F8': 'F8', 'F9': 'F9', 'F10': 'F10', 'F11': 'F11', 'F12': 'F12',
        };
        key = keyMap[key] || key;
      }

      parts.push(key);
      input.value = parts.join('+');
      input.blur();
    });

    // Visual focus state
    input.addEventListener('focus', () => input.classList.add('recording'));
    input.addEventListener('blur', () => input.classList.remove('recording'));
  });
}

function setupTauriListeners() {
  listen('focus-input', () => {
    inputText.value = '';
    resultsSection.innerHTML = '';
    showEmptyState();
    inputText.focus();
    settingsView.classList.add('hidden');
    mainView.classList.remove('hidden');
  });

  listen('select-translate', async () => {
    settingsView.classList.add('hidden');
    mainView.classList.remove('hidden');
    try {
      const text = await invoke('get_clipboard_text');
      if (text && text.trim()) {
        inputText.value = text.trim();
        doTranslate();
      }
    } catch (e) {
      console.error('Select translate error:', e);
    }
  });

  // trigger-screenshot event (from Alt+S hotkey or tray menu)
  listen('trigger-screenshot', async () => {
    doOCR();
  });

  // OCR region selection results
  listen('ocr-result', async (event) => {
    settingsView.classList.add('hidden');
    mainView.classList.remove('hidden');
    const text = event.payload;
    if (text && text.trim()) {
      inputText.value = text.trim();
      doTranslate();
    } else {
      resultsSection.innerHTML = '<div class="result-card"><div class="result-body"><span class="result-error">未识别到文字</span></div></div>';
    }
  });

  listen('ocr-error', async (event) => {
    settingsView.classList.add('hidden');
    mainView.classList.remove('hidden');
    resultsSection.innerHTML = '<div class="result-card"><div class="result-body"><span class="result-error"></span></div></div>';
    resultsSection.querySelector('.result-error').textContent = 'OCR 失败: ' + event.payload;
  });

  // Clipboard monitoring: auto-translate mode sends text directly
  listen('clipboard-translate', async (event) => {
    settingsView.classList.add('hidden');
    mainView.classList.remove('hidden');
    const text = event.payload;
    if (text && text.trim()) {
      inputText.value = text.trim();
      doTranslate();
    }
  });
}

// ==================== Translation ====================
async function doTranslate() {
  const text = inputText.value.trim();
  if (!text || isTranslating) return;

  isTranslating = true;
  loadingBar.classList.add('active');
  $('#translateBtn').disabled = true;
  resultsSection.innerHTML = '';

  try {
    const results = await invoke('translate_text', {
      text,
      source: sourceLang.value,
      target: targetLang.value,
    });

    renderResults(results);
  } catch (e) {
    resultsSection.innerHTML = `<div class="result-card"><div class="result-body"><span class="result-error">翻译出错: ${e}</span></div></div>`;
  } finally {
    isTranslating = false;
    loadingBar.classList.remove('active');
    $('#translateBtn').disabled = false;
  }
}

function renderResults(results) {
  resultsSection.innerHTML = '';

  if (!results || results.length === 0) {
    resultsSection.innerHTML = '<div class="result-card"><div class="result-body"><span class="result-error">没有翻译结果</span></div></div>';
    return;
  }

  for (const r of results) {
    const iconClass = getServiceIconClass(r.service);
    const iconLabel = getServiceIconLabel(r.service);

    const card = document.createElement('div');
    card.className = 'result-card';
    card.innerHTML = `
      <div class="result-header">
        <div class="result-service">
          <span class="result-service-icon ${iconClass}">${iconLabel}</span>
          <span class="result-service-name">${r.service}</span>
        </div>
        <div class="result-actions">
          <button class="result-action-btn copy-btn" title="复制">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <rect x="9" y="9" width="13" height="13" rx="2" ry="2"/><path d="M5 15H4a2 2 0 01-2-2V4a2 2 0 012-2h9a2 2 0 012 2v1"/>
            </svg>
          </button>
          <button class="result-action-btn speak-result-btn" title="朗读">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M15.54 8.46a5 5 0 010 7.07"/>
            </svg>
          </button>
        </div>
      </div>
      <div class="result-body">
        ${r.error
        ? `<span class="result-error">${r.error}</span>`
        : `<div class="result-text">${escapeHtml(r.translated)}</div>`
      }
        <div class="result-lang-info">${r.source_lang} → ${r.target_lang}</div>
      </div>
    `;

    // Copy button
    card.querySelector('.copy-btn').addEventListener('click', () => {
      navigator.clipboard.writeText(r.translated).then(() => showToast('已复制'));
    });

    // Speak button
    card.querySelector('.speak-result-btn').addEventListener('click', () => {
      if (r.translated) invoke('speak', { text: r.translated });
    });

    resultsSection.appendChild(card);
  }
}

function getServiceIconClass(service) {
  const s = service.toLowerCase();
  if (s.includes('google')) return 'google';
  if (s.includes('bing')) return 'bing';
  if (s.includes('deepl')) return 'deepl';
  if (s.includes('baidu') || s.includes('百度')) return 'baidu';
  if (s.includes('ai')) return 'ai';
  return 'google';
}

function getServiceIconLabel(service) {
  const s = service.toLowerCase();
  if (s.includes('google')) return 'G';
  if (s.includes('bing')) return 'B';
  if (s.includes('deepl')) return 'D';
  if (s.includes('baidu') || s.includes('百度')) return '百';
  if (s.includes('ai')) return 'AI';
  return '?';
}

// ==================== OCR ====================
async function doOCR() {
  if (isOcrInProgress) return;
  isOcrInProgress = true;
  try {
    await invoke('start_screenshot_ocr');
  } catch (e) {
    await getCurrentWindow().show().catch(() => { });
    await getCurrentWindow().setFocus().catch(() => { });
    const errMsg = typeof e === 'string' ? e : (e.message || JSON.stringify(e));
    resultsSection.innerHTML = '<div class="result-card"><div class="result-body"><span class="result-error"></span></div></div>';
    resultsSection.querySelector('.result-error').textContent = '截图失败: ' + errMsg;
    loadingBar.classList.remove('active');
  } finally {
    isOcrInProgress = false;
  }
}

// ==================== Settings ====================
async function saveSettings() {
  config.general.theme = $('#themeSelect').value;
  config.general.target_lang = $('#defaultTargetLang').value;

  config.services.google.enabled = $('#googleEnabled').checked;
  config.services.bing.enabled = $('#bingEnabled').checked;

  config.services.deepl.enabled = $('#deeplEnabled').checked;
  config.services.deepl.api_key = $('#deeplApiKey').value;

  config.services.baidu.enabled = $('#baiduEnabled').checked;
  config.services.baidu.app_id = $('#baiduAppId').value;
  config.services.baidu.secret_key = $('#baiduSecretKey').value;

  config.services.openai.enabled = $('#openaiEnabled').checked;
  config.services.openai.api_key = $('#openaiApiKey').value;
  config.services.openai.api_url = $('#openaiApiUrl').value;
  config.services.openai.model = $('#openaiModel').value;

  // Select-translate settings
  if (!config.select_translate) {
    config.select_translate = { enabled: true, mode: 'icon', monitor_clipboard: true };
  }
  config.select_translate.enabled = $('#selectTranslateEnabled').checked;
  config.select_translate.mode = $('#selectTranslateMode').value;
  config.select_translate.monitor_clipboard = $('#selectTranslateEnabled').checked;

  // Hotkey settings
  config.hotkeys.input_translate = {
    enabled: $('#hotkeyInputEnabled').checked,
    shortcut: $('#hotkeyInputShortcut').value,
  };
  config.hotkeys.select_translate = {
    enabled: $('#hotkeySelectEnabled').checked,
    shortcut: $('#hotkeySelectShortcut').value,
  };
  config.hotkeys.screenshot_translate = {
    enabled: $('#hotkeyScreenshotEnabled').checked,
    shortcut: $('#hotkeyScreenshotShortcut').value,
  };

  try {
    await invoke('update_config', { config });
    await invoke('update_shortcuts');
    targetLang.value = config.general.target_lang;
    showToast('设置已保存');
  } catch (e) {
    showToast('保存失败: ' + e);
  }
}

// ==================== Window Pin ====================
async function togglePin() {
  isPinned = !isPinned;
  await getCurrentWindow().setAlwaysOnTop(isPinned);
  $('#pinBtn').classList.toggle('active', isPinned);
}

// ==================== Helpers ====================
function showEmptyState() {
  resultsSection.innerHTML = `
    <div class="empty-state">
      <svg width="48" height="48" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1">
        <path d="M12.87 15.07l-2.54-2.51.03-.03A17.52 17.52 0 0014.07 6H17V4h-7V2H8v2H1v1.99h11.17C11.5 7.92 10.44 9.75 9 11.35 8.07 10.32 7.3 9.19 6.69 8h-2c.73 1.63 1.73 3.17 2.98 4.56l-5.09 5.02L4 19l5-5 3.11 3.11.76-2.04zM18.5 10h-2L12 22h2l1.12-3h4.75L21 22h2l-4.5-12zm-2.62 7l1.62-4.33L19.12 17h-3.24z"/>
      </svg>
      <p>输入文本后按 <span class="hotkey">Enter</span> 翻译<br/>
      <span class="hotkey">Alt+A</span> 输入翻译 &nbsp; <span class="hotkey">Alt+D</span> 划词翻译<br/>
      <span class="hotkey">Alt+S</span> 截图翻译</p>
    </div>
  `;
}

function escapeHtml(text) {
  const el = document.createElement('span');
  el.textContent = text;
  return el.innerHTML.replace(/\n/g, '<br>');
}

function showToast(message) {
  let toast = document.querySelector('.toast');
  if (!toast) {
    toast = document.createElement('div');
    toast.className = 'toast';
    document.body.appendChild(toast);
  }
  toast.textContent = message;
  toast.classList.add('show');
  setTimeout(() => toast.classList.remove('show'), 2000);
}

// ==================== Start ====================
document.addEventListener('DOMContentLoaded', init);
