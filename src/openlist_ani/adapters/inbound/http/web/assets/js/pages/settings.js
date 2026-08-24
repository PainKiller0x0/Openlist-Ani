import { api } from '../api.js';
import { confirmDialog, escapeHtml, pageHeader, renderShell, setLoading, toast } from '../components.js';

function field(label, id, value, type = 'text', extra = '') { return `<label class="field"><span>${escapeHtml(label)}</span><input id="${id}" type="${type}" value="${escapeHtml(value ?? '')}" ${extra}></label>`; }

export async function renderSettings(ctx) {
  const settings = await api.get('/api/ui/settings');
  const llm = settings.llm || {};
  const content = `${pageHeader('设置', '修改后端实际使用的 OpenList、RSS、命名和识别配置', '<button class="primary-button" id="saveSettings">保存设置</button>')}
    <div class="settings-layout"><nav class="settings-nav"><button class="active" data-section="openlist">OpenList</button><button data-section="rss">RSS 与 Mikan</button><button data-section="rename">下载命名</button><button data-section="metadata">媒体识别</button><button data-section="llm">LLM</button><button data-section="system">系统</button></nav><div class="settings-content">
      <section class="card settings-section" data-section-panel="openlist"><div class="card-header"><div><h2 class="section-title">OpenList</h2><div class="muted">保存前会验证地址和下载目录；验证失败不会保存。</div></div></div><div class="card-body form-grid">${field('OpenList 地址', 'openlistUrl', settings.openlist_url, 'url', 'required')}${field('下载根目录', 'downloadPath', settings.download_path, 'text', 'required')}</div></section>
      <section class="card settings-section" data-section-panel="rss" hidden><div class="card-header"><div><h2 class="section-title">RSS 与 Mikan</h2><div class="muted">轮询间隔范围为 60–86400 秒。</div></div></div><div class="card-body form-grid">${field('RSS 轮询间隔（秒）', 'pollInterval', settings.poll_interval_seconds, 'number', 'min="60" max="86400" required')}${field('最大下载重试次数', 'maxRetries', settings.max_download_retries, 'number', 'min="0" max="100" required')}${field('Mikan 地址', 'mikanBaseUrl', settings.mikan_base_url, 'url', 'required')}<label class="field wide"><span>全局排除规则（多个用 | 分隔）</span><textarea id="globalExclude">${escapeHtml(settings.global_exclude_patterns || '')}</textarea></label></div></section>
      <section class="card settings-section" data-section-panel="rename" hidden><div class="card-header"><div><h2 class="section-title">下载命名</h2><div class="muted">使用后端支持的字段；保存时会执行现有格式校验。</div></div></div><div class="card-body form-grid">${field('重命名规则', 'renameFormat', settings.rename_format, 'text', 'required')}<div class="notice wide">可用字段由后端命名规划器决定；不要在这里填写虚构字段。</div></div></section>
      <section class="card settings-section" data-section-panel="metadata" hidden><div class="card-header"><div><h2 class="section-title">媒体识别</h2><div class="muted">空配置不会被前端伪装成已启用。</div></div></div><div class="card-body form-grid">${field('TMDB 语言', 'tmdbLanguage', llm.tmdb_language || 'zh-CN')}${field('元数据解析器', 'metadataProvider', llm.metadata_parser_provider || 'none')}<div class="notice wide">TMDB / 豆瓣 / LLM 的实际启用状态由后端配置和凭据决定；当前页面只保存后端已公开的配置项。</div></div></section>
      <section class="card settings-section" data-section-panel="llm" hidden><div class="card-header"><div><h2 class="section-title">LLM</h2><div class="muted">API Key 已配置时只显示状态，不回显密钥；密钥留空表示保持原值。</div></div></div><div class="card-body form-grid">${field('提供商类型', 'llmProvider', llm.provider_type || 'openai-compatible')}${field('Base URL', 'llmBaseUrl', llm.base_url || '', 'url')}${field('模型', 'llmModel', llm.model || '')}${field('API Key', 'llmApiKey', '', 'password', llm.api_key_configured ? 'placeholder="已配置，留空保持不变"' : 'placeholder="未配置"')}</div></section>
      <section class="card settings-section" data-section-panel="system" hidden><div class="card-header"><div><h2 class="section-title">系统</h2><div class="muted">保存设置不会自动重启服务。</div></div></div><div class="card-body"><div id="restartNotice" class="notice">如设置返回需要重启，请手动点击下面按钮。</div><div class="button-row" style="margin-top:14px"><button class="secondary-button" id="restartService">重启 op-ani</button></div></div></section>
    </div></div>`;
  renderShell('#/settings', content, settings);
  document.querySelectorAll('[data-section]').forEach((button) => button.addEventListener('click', () => { document.querySelectorAll('[data-section]').forEach((item) => item.classList.toggle('active', item === button)); document.querySelectorAll('[data-section-panel]').forEach((panel) => { panel.hidden = panel.dataset.sectionPanel !== button.dataset.section; }); }));

  document.querySelector('#saveSettings').addEventListener('click', async (event) => {
    const button = event.currentTarget;
    const payload = {
      openlist_url: document.querySelector('#openlistUrl').value.trim(),
      download_path: document.querySelector('#downloadPath').value.trim(),
      poll_interval_seconds: Number(document.querySelector('#pollInterval').value),
      max_download_retries: Number(document.querySelector('#maxRetries').value),
      mikan_base_url: document.querySelector('#mikanBaseUrl').value.trim(),
      global_exclude_patterns: document.querySelector('#globalExclude').value,
      rename_format: document.querySelector('#renameFormat').value.trim(),
      llm_provider_type: document.querySelector('#llmProvider').value.trim(),
      llm_base_url: document.querySelector('#llmBaseUrl').value.trim(),
      llm_model: document.querySelector('#llmModel').value.trim(),
      tmdb_language: document.querySelector('#tmdbLanguage').value.trim(),
      metadata_parser_provider: document.querySelector('#metadataProvider').value.trim(),
    };
    const key = document.querySelector('#llmApiKey').value;
    if (key) payload.llm_api_key = key;
    if (!Number.isInteger(payload.poll_interval_seconds) || payload.poll_interval_seconds < 60 || payload.poll_interval_seconds > 86400) return toast('RSS 轮询间隔必须在 60–86400 秒之间', 'error');
    setLoading(button, true, '保存中');
    try { const result = await api.post('/api/ui/settings', payload); toast(result.message || '设置已保存'); document.querySelector('#restartNotice').className = result.requires_restart ? 'notice' : 'notice good'; document.querySelector('#restartNotice').textContent = result.requires_restart ? '设置已保存，需要手动重启 op-ani 才会完全生效。' : '设置已保存并已生效。'; }
    catch (error) { toast(error.message, 'error'); }
    finally { setLoading(button, false); }
  });
  document.querySelector('#restartService').addEventListener('click', async (event) => { if (!await confirmDialog('重启 op-ani', '重启会短暂中断页面和当前扫描，确认继续吗？', '确认重启')) return; setLoading(event.currentTarget, true, '重启中'); try { const result = await api.post('/api/restart'); toast(result.message || '重启信号已发送'); } catch (error) { toast(error.message, 'error'); } finally { setLoading(event.currentTarget, false); } });
}
