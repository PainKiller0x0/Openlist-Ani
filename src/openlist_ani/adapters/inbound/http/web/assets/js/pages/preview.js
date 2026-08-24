import { api } from '../api.js';
import { escapeHtml, pageHeader, poster, renderShell, setLoading, toast } from '../components.js';

function steps() { return `<div class="stepper"><div class="step"><span class="step-number">1</span>搜索</div><div class="step"><span class="step-number">2</span>选择字幕组</div><div class="step active"><span class="step-number">3</span>确认预览</div></div>`; }

function previewEntries(data) {
  const entries = data.entries || [];
  if (!entries.length) return '<div class="empty">没有可展示的 RSS 条目。</div>';
  return `<div class="preview-list">${entries.map((entry) => `<article class="preview-item ${entry.excluded ? 'excluded' : ''}"><div class="button-row"><span class="preview-item-title">${escapeHtml(entry.title)}</span><span class="badge ${entry.excluded ? 'danger' : 'good'}">${entry.excluded ? `排除${entry.matched_pattern ? `：${escapeHtml(entry.matched_pattern)}` : ''}` : '会下载'}</span></div><div class="muted small">${escapeHtml(entry.anime_name || '')}${entry.season != null ? ` · 第 ${entry.season} 季` : ''}${entry.episode != null ? ` · 第 ${entry.episode} 集` : ''}${entry.llm_parsed ? ' · 已识别' : ''}</div>${entry.rename_preview ? `<div class="preview-item-path">改名预览：${escapeHtml(entry.rename_preview)}</div>` : '<div class="preview-item-path">改名预览：后端未返回结果</div>'}${entry.download_directory ? `<div class="preview-item-path">下载目录：${escapeHtml(entry.download_directory)}</div>` : ''}</article>`).join('')}</div>`;
}

export async function renderPreview(ctx) {
  const initial = ctx.flow.preview;
  if (!initial || !ctx.flow.rssUrl) return ctx.navigate('#/add');
  const data = initial;
  const selected = ctx.flow.selectedBangumi;
  const content = `${pageHeader('识别与下载预览', '确认后才会保存订阅并立即触发一次新条目扫描', '<a class="secondary-button" href="#/add">返回修改</a>')}${steps()}<section class="card"><div class="preview-header">${poster(data.poster_url, data.name || ctx.flow.preferredName || '订阅')}<div><h2 class="section-title">${escapeHtml(data.name || ctx.flow.preferredName || '未命名订阅')}</h2><div class="muted">${selected ? `Mikan · ${escapeHtml(selected.name || '')}` : escapeHtml(ctx.flow.rssUrl)}</div><div class="muted small">TMDb #${escapeHtml(data.tmdb_id || '未识别')}</div></div></div><div class="card-body"><div class="form-grid"><label class="field"><span>显示名称（可选）</span><input id="previewName" value="${escapeHtml(data.name || ctx.flow.preferredName || '')}"></label><label class="field"><span>下载用 anime_name（可选）</span><input id="previewAnimeName" value="${escapeHtml(ctx.flow.animeName || data.anime_name || '')}" placeholder="留空使用识别结果"></label><label class="field"><span>下载目录名（可选）</span><input id="previewDirectoryName" value="${escapeHtml(ctx.flow.directoryName || data.download_directory_name || '')}" placeholder="留空使用 anime_name"></label><label class="field"><span>此 RSS 的排除规则（可选）</span><input id="previewExclude" value="${escapeHtml(ctx.flow.excludePatterns || data.exclude_patterns || '')}" placeholder="多个规则用 | 分隔"></label><div class="wide button-row"><button class="secondary-button" id="refreshPreview">重新识别预览</button><span class="muted small">修改字段后按 Enter 或点击按钮，重新获取真实改名和目录结果。</span></div></div><div class="preview-stats"><span class="badge blue">总计 ${escapeHtml(data.total ?? 0)}</span><span class="badge good">排除后会下载 ${escapeHtml(data.included ?? 0)}</span><span class="badge danger">排除 ${escapeHtml(data.excluded ?? 0)}</span></div><div id="previewEntries">${previewEntries(data)}</div></div></section><div class="sticky-bar"><div><strong>保存前确认</strong><div class="muted small">当前不会直接下载，确认后才保存 RSS。</div></div><div class="button-row"><button class="secondary-button" id="cancelPreview">返回修改</button><button class="primary-button" id="confirmPreview">确认并开始追踪</button></div></div>`;
  renderShell('#/add', content, {});

  const value = (id) => document.querySelector(id).value.trim();
  const refresh = async (button) => {
    ctx.flow.preferredName = value('#previewName');
    ctx.flow.animeName = value('#previewAnimeName');
    ctx.flow.directoryName = value('#previewDirectoryName');
    ctx.flow.excludePatterns = value('#previewExclude');
    setLoading(button, true, '⌛ 识别中');
    try { ctx.flow.preview = await api.post('/api/ui/rss/preview', {url: ctx.flow.rssUrl, name: ctx.flow.preferredName, anime_name: ctx.flow.animeName, download_directory_name: ctx.flow.directoryName, exclude_patterns: ctx.flow.excludePatterns}); await renderPreview(ctx); }
    catch (error) { toast(error.message, 'error'); setLoading(button, false); }
  };
  ['#previewName', '#previewAnimeName', '#previewDirectoryName', '#previewExclude'].forEach((selector) => document.querySelector(selector).addEventListener('keydown', (event) => { if (event.key === 'Enter') { event.preventDefault(); refresh(document.querySelector('#refreshPreview')); } }));
  document.querySelector('#refreshPreview').addEventListener('click', (event) => refresh(event.currentTarget));
  document.querySelector('#cancelPreview').addEventListener('click', () => ctx.navigate('#/add'));
  document.querySelector('#confirmPreview').addEventListener('click', async (event) => {
    const button = event.currentTarget;
    setLoading(button, true, '保存中');
    try {
      const firstIncluded = (data.entries || []).find((entry) => !entry.excluded);
      const result = await api.post('/api/ui/rss', {url: ctx.flow.rssUrl, name: value('#previewName'), anime_name: value('#previewAnimeName'), download_directory_name: value('#previewDirectoryName'), exclude_patterns: value('#previewExclude'), tmdb_id: data.tmdb_id || null, season: Number(firstIncluded?.season || 1), confirmed: true});
      toast(result.message || 'RSS 已保存'); ctx.flow = {}; ctx.navigate('#/');
    } catch (error) { toast(error.message, 'error'); setLoading(button, false); }
  });
}
