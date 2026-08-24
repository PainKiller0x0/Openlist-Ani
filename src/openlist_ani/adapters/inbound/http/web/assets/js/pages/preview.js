import { api } from '../api.js';
import { escapeHtml, pageHeader, poster, renderShell, setLoading, toast } from '../components.js';

function steps() { return `<div class="stepper"><div class="step done"><span class="step-number">✓</span>搜索</div><div class="step done"><span class="step-number">✓</span>选择作品</div><div class="step active"><span class="step-number">3</span>选择字幕组</div><div class="step"><span class="step-number">4</span>确认追番</div></div>`; }

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
  const content = `${pageHeader('选择字幕组与下载预览', '确认后才会保存订阅并开始自动追踪', '<span class="muted">◔ RSS 自动追踪</span>')}${steps()}<div class="preview-layout"><section><div class="card selected-card"><div class="card-header"><div><h2 class="section-title">已选作品</h2><div class="muted">确认要追踪的番剧与来源</div></div><span class="badge blue">TV · ${escapeHtml((data.entries || [])[0]?.season ? `${(data.entries || [])[0].season} 季` : '待识别')}</span></div><div class="card-body preview-show">${poster(data.poster_url, data.name || ctx.flow.preferredName || '订阅')}<div><h3>${escapeHtml(data.name || ctx.flow.preferredName || '未命名订阅')}</h3><div class="muted">${selected ? escapeHtml(selected.name || '') : 'RSS 订阅'}</div><div class="muted small">${escapeHtml(ctx.flow.rssUrl)}</div></div></div></div><div class="card group-select-card"><div class="card-header"><div><h2 class="section-title">选择字幕组</h2><div class="muted">将使用选中的来源创建 RSS 追踪规则</div></div><span>♧</span></div><div class="card-body"><div class="selected-group"><span class="radio-dot"></span><strong>${escapeHtml(data.fansub || selected?.name || '当前 RSS 来源')}</strong></div></div></div></section><section class="preview-main"><div class="card"><div class="card-header"><div><h2 class="section-title">番剧详情</h2><div class="muted">根据 TMDB 自动识别</div></div><button class="ghost-button" id="refreshMetadata">✎ 修正识别信息</button></div><div class="card-body"><div class="metadata-summary"><div class="metadata-poster">${poster(data.poster_url, data.name || '番剧')}</div><dl><dt>番剧名称</dt><dd>${escapeHtml(data.name || ctx.flow.preferredName || '未命名')}</dd><dt>Season</dt><dd>${escapeHtml(data.season || (data.entries || [])[0]?.season || 'Season 1')}</dd><dt>TMDB ID</dt><dd>${escapeHtml(data.tmdb_id || '未记录')}</dd><dt>RSS 来源</dt><dd>${escapeHtml(ctx.flow.rssUrl)}</dd></dl></div></div></div><section class="card download-preview-card"><div class="card-header"><div><h2 class="section-title">下载预览</h2><div class="muted">将按规则规范化文件名并保存至 OpenList</div></div><span class="muted">预览</span></div><div class="card-body"><div class="form-grid"><label class="field"><span>下载用 anime_name（可选）</span><input id="previewAnimeName" value="${escapeHtml(ctx.flow.animeName || data.anime_name || '')}" placeholder="留空使用识别结果"></label><label class="field"><span>下载目录名（可选）</span><input id="previewDirectoryName" value="${escapeHtml(ctx.flow.directoryName || data.download_directory_name || '')}" placeholder="留空使用 anime_name"></label><label class="field wide"><span>这个 RSS 的排除规则（可选）</span><input id="previewExclude" value="${escapeHtml(ctx.flow.excludePatterns || data.exclude_patterns || '')}" placeholder="多个规则用 | 分隔"></label><label class="field wide"><span>订阅显示名称（可选）</span><input id="previewName" value="${escapeHtml(data.name || ctx.flow.preferredName || '')}" placeholder="留空使用识别结果"></label><div class="wide button-row"><button class="secondary-button" id="refreshPreview">重新识别预览</button><span class="muted small">修改字段后按 Enter 或点击按钮，重新获取真实改名和目录结果。</span></div></div><div class="preview-stats"><span class="badge blue">RSS 总资源 ${escapeHtml(data.total ?? 0)}</span><span class="badge danger">已排除 ${escapeHtml(data.excluded ?? 0)}</span><span class="badge good">将下载 ${escapeHtml(data.included ?? 0)}</span></div><div id="previewEntries">${previewEntries(data)}</div>${data.download_directory ? `<div class="path-bar">▱ 最终 OpenList 下载目录 <strong>${escapeHtml(data.download_directory)}</strong></div>` : ''}</div></section></section></div><div class="sticky-bar"><div><strong>将下载 ${escapeHtml(data.included ?? 0)} 集 · 已排除 ${escapeHtml(data.excluded ?? 0)} 集</strong></div><div class="button-row"><button class="secondary-button" id="cancelPreview">返回修改</button><button class="primary-button" id="confirmPreview">▶ 确认并开始追踪</button></div></div>`;
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
  document.querySelector('#refreshMetadata')?.addEventListener('click', async (event) => {
    setLoading(event.currentTarget, true, '识别中');
    try {
      const result = await api.post('/api/ui/rss/metadata', {url: ctx.flow.rssUrl, name: value('#previewName'), tmdb_id: data.tmdb_id || null});
      if (result.name) ctx.flow.preferredName = result.name;
      ctx.flow.preview = await api.post('/api/ui/rss/preview', {url: ctx.flow.rssUrl, name: ctx.flow.preferredName || value('#previewName'), anime_name: value('#previewAnimeName'), download_directory_name: value('#previewDirectoryName'), exclude_patterns: value('#previewExclude')});
      await renderPreview(ctx);
    } catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); }
  });
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
