import { api } from '../api.js';
import { escapeHtml, pageHeader, renderShell, setLoading, toast } from '../components.js';

function stepper(active) {
  return `<div class="stepper">${[['1', '搜索'], ['2', '选择字幕组'], ['3', '确认预览']].map(([number, label], index) => `<div class="step ${active === index + 1 ? 'active' : ''}"><span class="step-number">${number}</span>${label}</div>`).join('')}</div>`;
}

function renderSearchResults(results) {
  if (!results.length) return '<div class="empty">没有找到匹配的番剧。</div>';
  return `<div class="result-list">${results.map((item, index) => `<button class="result-item" data-result-index="${index}"><span><span class="result-title">${escapeHtml(item.name || '未命名')}</span><br><span class="result-meta">Mikan Bangumi #${escapeHtml(item.bangumi_id)}</span></span><span>选择 →</span></button>`).join('')}</div>`;
}

export async function renderAdd(ctx) {
  const content = `${pageHeader('添加追番', '从 Mikan 搜索并添加 RSS，或直接粘贴 RSS / 上传种子')}
    <div id="mikanHome" class="card"><div class="card-header"><div><h2 class="section-title">从 Mikan 搜索并添加 RSS</h2><div class="muted">搜索番剧后选择字幕组，系统会自动生成 RSS 并进入识别预览。</div></div></div><div class="card-body"><div class="button-row"><input class="search-input" id="mikanKeyword" placeholder="例如：尼古喵喵、幼女战记"><button class="primary-button" id="mikanSearch">搜索 Mikan</button></div><div id="mikanResults" style="margin-top:16px"></div></div></div>
    <section class="card" style="margin-top:16px"><div class="card-header"><div><h2 class="section-title">直接添加 RSS</h2><div class="muted">输入链接后进入识别与下载预览，确认后才会保存订阅。</div></div></div><div class="card-body"><form id="directRssForm" class="form-grid"><label class="field wide"><span>RSS 链接</span><input id="directRssUrl" type="url" required placeholder="https://example.com/anime.xml"></label><label class="field"><span>显示名称（可选）</span><input id="directRssName" placeholder="留空使用识别结果"></label><div class="field" style="align-self:end"><button class="primary-button" type="submit">识别并预览</button></div></form></div></section>
    <section class="card" style="margin-top:16px"><div class="card-header"><div><h2 class="section-title">上传种子</h2><div class="muted">最大 50 MB，上传后创建真实下载任务。</div></div></div><div class="card-body"><form id="torrentForm" class="form-grid"><label class="field"><span>.torrent 文件</span><input id="torrentFile" type="file" accept=".torrent" required></label><label class="field"><span>任务名称（可选）</span><input id="torrentTitle" placeholder="留空使用种子标题"></label><div class="wide button-row"><button class="primary-button" type="submit">上传并创建任务</button></div></form></div></section>`;
  renderShell('#/add', content, {});

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
      ctx.flow.preferredName = document.querySelector('#directRssName').value.trim();
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
  let groups = ctx.flow.groups;
  if (!groups) {
    try { const response = await api.post('/api/ui/mikan/groups', {bangumi_id: selected.bangumi_id}); groups = response.groups || []; ctx.flow.groups = groups; ctx.flow.allRssUrl = response.all_rss_url || ''; }
    catch (error) { renderShell('#/add', `${pageHeader('选择字幕组')}<div class="notice">${escapeHtml(error.message)}</div>`, {}); return; }
  }
  const content = `${pageHeader('添加追番', '选择字幕组后进入识别与下载预览', '<a class="secondary-button" href="#/add">返回搜索</a>')}${stepper(2)}<div class="card"><div class="card-header"><div><h2 class="section-title">${escapeHtml(selected.name || '未命名番剧')}</h2><div class="muted">Mikan Bangumi #${escapeHtml(selected.bangumi_id)}</div></div></div><div class="card-body"><div class="group-list">${groups.length ? groups.map((group, index) => `<button class="group-item" data-group-index="${index}"><span><strong>${escapeHtml(group.name || '未命名字幕组')}</strong><br><span class="result-meta">${escapeHtml(group.release_count ?? 0)} 个条目</span></span><span>使用此组 →</span></button>`).join('') : '<div class="empty">没有可用字幕组。</div>'}</div>${ctx.flow.allRssUrl ? `<div class="button-row" style="margin-top:16px"><button class="secondary-button" id="useAllGroups">使用全部字幕组</button></div>` : ''}</div></div>`;
  renderShell('#/add', content, {});
  const choose = async (group) => {
    const button = document.querySelector(`[data-group-index="${group.index}"]`);
    if (button) setLoading(button, true, '读取中');
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
    setLoading(event.currentTarget, true, '读取中');
    try { ctx.flow.rssUrl = ctx.flow.allRssUrl; ctx.flow.preferredName = selected.name || ''; ctx.flow.preview = await api.post('/api/ui/rss/preview', {url: ctx.flow.rssUrl, name: ctx.flow.preferredName}); ctx.navigate('#/add/preview'); }
    catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); }
  });
}
