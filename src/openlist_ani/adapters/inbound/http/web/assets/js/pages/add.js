import { api } from '../api.js';
import { escapeHtml, pageHeader, renderShell, setLoading, toast } from '../components.js';

function stepper(active) {
  return `<div class="stepper">${[['1', '搜索'], ['2', '选择作品'], ['3', '选择字幕组'], ['4', '确认追番']].map(([number, label], index) => { const step = index + 1; return `<div class="step ${active === step ? 'active' : step < active ? 'done' : ''}"><span class="step-number">${step < active ? '✓' : number}</span>${label}</div>`; }).join('')}</div>`;
}

function renderSearchResults(results) {
  if (!results.length) return '<div class="empty">没有找到匹配的番剧。</div>';
  return `<div class="result-list">${results.map((item, index) => `<div class="result-item"><div class="result-copy"><div class="result-title">${escapeHtml(item.name || '未命名')}</div><a class="mikan-bangumi-link" href="${escapeHtml(item.url || '#')}" target="_blank" rel="noopener noreferrer">Mikan Bangumi #${escapeHtml(item.bangumi_id)}</a></div><button class="result-select" data-result-index="${index}">选择 →</button></div>`).join('')}</div>`;
}

export async function renderAdd(ctx) {
  const shellState = await api.get('/api/ui/settings');
  const content = `${pageHeader('添加追番', '选择一种方式，将新的番组加入你的本地媒体库。', '<span class="method-count">▱ 3 种添加方式</span>')}
    <section class="flow-method"><div class="method-intro"><span class="flow-number active">1</span><div><h2>通过 Mikan 搜索</h2><p>搜索番组并自动获取可用的 RSS 订阅源。</p></div><span class="recommended">推荐</span></div><div id="mikanHome" class="card featured-method"><div class="card-body"><div class="search-line"><span>⌕</span><input class="search-input" id="mikanKeyword" placeholder="输入番组名称，例如：葬送的芙莉莲"><button class="primary-button" id="mikanSearch">搜索番组</button></div><div class="method-help">✧ 支持中文、日文及罗马音关键词 · 由 Mikan Project 提供索引</div><div id="mikanResults" class="search-results"></div></div></div></section>
    <section class="flow-method"><div class="method-intro"><span class="flow-number">2</span><div><h2>直接添加 RSS</h2><p>粘贴订阅地址，识别后确认番组信息。</p></div></div><div class="card direct-method"><div class="card-body"><form id="directRssForm" class="method-form"><div class="input-with-icon"><span>◔</span><input id="directRssUrl" type="url" required placeholder="https://mikanani.me/RSS/..."></div><button class="secondary-button" type="submit">识别并预览</button></form></div></div></section>
    <details class="flow-method torrent-method"><summary><span class="flow-number">3</span><span><strong>上传 Torrent 手动补番</strong><small>适用于没有 RSS 来源的番组。</small></span><span class="summary-arrow">⌄</span></summary><div class="card"><div class="card-body"><form id="torrentForm" class="form-grid"><label class="field"><span>.torrent 文件</span><input id="torrentFile" type="file" accept=".torrent" required></label><label class="field"><span>任务名称（可选）</span><input id="torrentTitle" placeholder="留空使用种子标题"></label><div class="wide button-row"><button class="primary-button" type="submit">上传并创建任务</button></div></form></div></div></details>`;
  renderShell('#/add', content, shellState);

  const resultRoot = document.querySelector('#mikanResults');
  document.querySelector('#mikanSearch').addEventListener('click', async (event) => {
    const keyword = document.querySelector('#mikanKeyword').value.trim();
    if (!keyword) return toast('请输入番剧名称', 'error');
    setLoading(event.currentTarget, true, '搜索中');
    try {
      const response = await api.post('/api/ui/mikan/search', {keyword});
      ctx.flow.searchResults = response.results || [];
      resultRoot.innerHTML = renderSearchResults(ctx.flow.searchResults);
      resultRoot.querySelectorAll('[data-result-index]').forEach((button) => button.addEventListener('click', () => {
        ctx.flow.selectedBangumi = ctx.flow.searchResults[Number(button.dataset.resultIndex)];
        ctx.navigate('#/add/select');
      }));
    } catch (error) { resultRoot.innerHTML = `<div class="notice">${escapeHtml(error.message)}</div>`; }
    finally { setLoading(event.currentTarget, false); }
  });

  document.querySelector('#directRssForm').addEventListener('submit', async (event) => {
    event.preventDefault();
    const button = event.currentTarget.querySelector('button[type="submit"]');
    setLoading(button, true, '⌛ 识别中');
    try {
      ctx.flow.rssUrl = document.querySelector('#directRssUrl').value.trim();
      ctx.flow.preferredName = document.querySelector('#directRssName')?.value.trim() || '';
      ctx.flow.preview = await api.post('/api/ui/rss/preview', {url: ctx.flow.rssUrl, name: ctx.flow.preferredName});
      ctx.navigate('#/add/preview');
    } catch (error) { toast(error.message, 'error'); }
    finally { setLoading(button, false); }
  });

  document.querySelector('#torrentForm').addEventListener('submit', async (event) => {
    event.preventDefault();
    const form = event.currentTarget;
    const file = form.querySelector('#torrentFile').files[0];
    if (!file) return;
    const button = form.querySelector('button[type="submit"]');
    setLoading(button, true, '上传中');
    try { const result = await api.uploadTorrent(file, form.querySelector('#torrentTitle').value.trim()); toast(result.message || '任务已创建'); ctx.navigate('#/downloads'); }
    catch (error) { toast(error.message, 'error'); }
    finally { setLoading(button, false); }
  });
}

export async function renderSelect(ctx) {
  const selected = ctx.flow.selectedBangumi;
  if (!selected) return ctx.navigate('#/add');
  const shellState = await api.get('/api/ui/settings');
  let groups = ctx.flow.groups;
  if (!groups) {
    renderShell('#/add', `${pageHeader('选择字幕组', '正在读取可用字幕组')}${stepper(2)}<div class="card"><div class="card-body"><div class="loading-state">⌛ 正在读取 Mikan 字幕组，请稍候…</div></div></div>`, shellState);
    try { const response = await api.post('/api/ui/mikan/groups', {bangumi_id: selected.bangumi_id}); groups = response.groups || []; ctx.flow.groups = groups; ctx.flow.allRssUrl = response.all_rss_url || ''; }
    catch (error) { renderShell('#/add', `${pageHeader('选择字幕组')}<div class="notice">${escapeHtml(error.message)}</div>`, shellState); return; }
  }
  const content = `${pageHeader('选择字幕组', '从已选作品中选择要追踪的字幕来源', '<span class="muted">媒体库 · 本地部署</span>')}${stepper(3)}<div class="card"><div class="card-header"><div><h2 class="section-title">${escapeHtml(selected.name || '未命名番剧')}</h2><div class="muted">Mikan Bangumi #${escapeHtml(selected.bangumi_id)}</div></div></div><div class="card-body"><div class="group-list">${groups.length ? groups.map((group, index) => `<button class="group-item" data-group-index="${index}"><span><strong>${escapeHtml(group.name || '未命名字幕组')}</strong><br><span class="result-meta">${escapeHtml(group.release_count ?? 0)} 个条目 · 选择后识别 RSS</span></span><span>选择 →</span></button>`).join('') : '<div class="empty">没有可用字幕组。</div>'}</div>${ctx.flow.allRssUrl ? `<div class="button-row" style="margin-top:16px"><button class="secondary-button" id="useAllGroups">使用全部字幕组</button></div>` : ''}</div></div>`;
  renderShell('#/add', content, shellState);
  const choose = async (group) => {
    const button = document.querySelector(`[data-group-index="${group.index}"]`);
    if (button) setLoading(button, true, '识别预览中');
    try {
      const rssResponse = await api.post('/api/ui/mikan/rss', {bangumi_id: selected.bangumi_id, subgroup_id: group.id});
      ctx.flow.rssUrl = rssResponse.rss_url;
      ctx.flow.preferredName = selected.name || '';
      ctx.flow.preview = await api.post('/api/ui/rss/preview', {url: ctx.flow.rssUrl, name: ctx.flow.preferredName});
      ctx.navigate('#/add/preview');
    } catch (error) { toast(error.message, 'error'); if (button) setLoading(button, false); }
  };
  document.querySelectorAll('[data-group-index]').forEach((button, index) => button.addEventListener('click', () => choose({...groups[index], index})));
  document.querySelector('#useAllGroups')?.addEventListener('click', async (event) => {
    setLoading(event.currentTarget, true, '识别预览中');
    try { ctx.flow.rssUrl = ctx.flow.allRssUrl; ctx.flow.preferredName = selected.name || ''; ctx.flow.preview = await api.post('/api/ui/rss/preview', {url: ctx.flow.rssUrl, name: ctx.flow.preferredName}); ctx.navigate('#/add/preview'); }
    catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); }
  });
}
