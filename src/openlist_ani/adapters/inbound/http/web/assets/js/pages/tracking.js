import { api } from '../api.js';
import { confirmDialog, emptyState, escapeHtml, formatDate, pageHeader, poster, renderShell, setLoading, statusBadge, toast } from '../components.js';

const CARD_SIZE_KEY = 'op-ani.subscription-card-size';
const CARD_SIZES = ['large', 'medium', 'small'];

function readCardSize() {
  try {
    const saved = localStorage.getItem(CARD_SIZE_KEY);
    return CARD_SIZES.includes(saved) ? saved : 'large';
  } catch {
    return 'large';
  }
}

function saveCardSize(size) {
  try { localStorage.setItem(CARD_SIZE_KEY, size); } catch { /* private browsing may block storage */ }
}

function subscriptionCard(item, state) {
  const enabled = item.enabled !== false;
  const latest = item.latest_episode ?? item.last_episode ?? null;
  const latestText = latest == null ? '暂无集数' : `更新到第 ${latest} 集`;
  const url = encodeURIComponent(item.url || '');
  const title = item.name || item.anime_name || '未命名订阅';
  return `<article class="poster-card subscription-card" data-url="${escapeHtml(item.url)}">
    <a class="subscription-card-link" href="#/subscription/${url}">${poster(item.poster_url, title)}<div class="poster-copy"><div class="poster-title">${escapeHtml(title)}</div><div class="poster-meta"><span>${enabled ? '追踪中' : '已暂停'}</span><span>${escapeHtml(latestText)}</span></div></div></a>
    <div class="subscription-card-status">${statusBadge(enabled ? 'running' : 'paused', enabled ? '追踪中' : '已暂停')}</div>
    <div class="card-actions"><button class="ghost-button" data-action="toggle" title="${enabled ? '暂停追番' : '继续追番'}">${enabled ? '暂停' : '继续'}</button><button class="ghost-button" data-action="remove" title="删除订阅">删除</button></div>
  </article>`;
}

function taskActivity(tasks) {
  const recent = [...(tasks || [])].sort((a, b) => String(b.updated_at || '').localeCompare(String(a.updated_at || ''))).slice(0, 8);
  if (!recent.length) return emptyState('暂无最近活动');
  return `<ul class="activity-list">${recent.map((task) => `<li><div class="activity-icon ${task.state === 'failed' ? 'danger' : task.state === 'completed' ? 'good' : 'blue'}">${task.state === 'failed' ? '!' : task.state === 'completed' ? '✓' : '↻'}</div><div class="activity-copy"><div class="button-row"><strong>${escapeHtml(task.title || task.anime_name || '下载任务')}</strong>${statusBadge(task.state)}</div><div class="muted small">${escapeHtml(task.anime_name || '')}${task.episode != null ? ` · 第 ${task.episode} 集` : ''}</div><div class="muted small">${escapeHtml(formatDate(task.updated_at))}</div>${task.error_message ? `<div class="small activity-error">${escapeHtml(task.error_message)}</div>` : ''}</div></li>`).join('')}</ul>`;
}

export async function renderTracking(ctx) {
  const state = await api.get('/api/ui/state');
  const status = state.rss_status || {};
  const cardSize = readCardSize();
  const subscriptions = (state.rss_subscriptions || []).filter((item) => item.url);
  const failed = (state.tasks || []).filter((task) => task.state === 'failed').length;
  const running = status.running === true || status.status === 'running';
  const syncEnabled = status.enabled !== false;
  const activeCount = subscriptions.filter((item) => item.enabled !== false).length;
  const content = `${pageHeader('追番中心', '管理正在追番的番剧，保持更新同步。', `<span class="runtime-pill"><i></i> 系统${running ? '运行中' : '空闲'}</span><button class="secondary-button" id="scanNow">立即扫描</button><a class="primary-button" href="#/add">＋ 添加追番</a>`)}
    <section class="grid stats-grid dashboard-stats">
      <div class="card stat-card"><div class="stat-label">运行状态</div><div class="stat-value">${running ? '服务在线' : escapeHtml(status.status || '服务空闲')}</div><span class="stat-icon">⌁</span></div>
      <div class="card stat-card"><div class="stat-label">上次扫描</div><div class="stat-value">${escapeHtml(formatDate(status.last_scan_finished_at) || '尚未扫描')}</div><span class="stat-icon blue">↻</span></div>
      <div class="card stat-card"><div class="stat-label">下次扫描</div><div class="stat-value">${escapeHtml(formatDate(status.next_scan_at) || '等待安排')}</div><span class="stat-icon">◷</span></div>
      <div class="card stat-card"><div class="stat-label">下载错误</div><div class="stat-value">${failed} 个待处理</div><span class="stat-icon danger">△</span></div>
    </section>
    <div class="home-toolbar"><div class="toolbar-search"><span>⌕</span><input class="search-input" id="subscriptionSearch" placeholder="搜索番剧…"></div><select class="status-filter" id="subscriptionFilter"><option value="all">全部状态</option><option value="running">追踪中</option><option value="paused">已暂停</option></select><span class="total-count">▥ 共 ${subscriptions.length} 部追番</span></div>
    <div class="grid home-content-grid"><section><div class="section-heading"><div class="section-heading-title"><h2>我的追番</h2><div class="card-size-switch" role="group" aria-label="卡片大小"><button class="card-size-button ${cardSize === 'large' ? 'active' : ''}" type="button" data-card-size="large" title="大卡片">大</button><button class="card-size-button ${cardSize === 'medium' ? 'active' : ''}" type="button" data-card-size="medium" title="中卡片">中</button><button class="card-size-button ${cardSize === 'small' ? 'active' : ''}" type="button" data-card-size="small" title="小卡片">小</button></div></div><a href="#/add">添加新的追番</a></div><div class="poster-grid size-${cardSize}" id="subscriptionList">${subscriptions.length ? subscriptions.map((item) => subscriptionCard(item, state)).join('') : emptyState('还没有追番，去添加一部吧。')}</div></section><aside class="card activity-card"><div class="card-header"><div><h2 class="section-title">最近活动</h2><div class="muted small">查看全部</div></div></div><div class="card-body">${taskActivity(state.tasks)}</div><div class="sync-notice"><span>ϟ</span><div><strong>自动同步${syncEnabled ? '已开启' : '已暂停'}</strong><small>${status.next_scan_at ? `下次扫描：${escapeHtml(formatDate(status.next_scan_at))}` : '按轮询周期检查更新。'}</small></div></div></aside></div>`;
  renderShell('/', content, state);

  const list = document.querySelector('#subscriptionList');
  document.querySelectorAll('[data-card-size]').forEach((button) => button.addEventListener('click', () => {
    const size = button.dataset.cardSize;
    if (!CARD_SIZES.includes(size)) return;
    list.classList.remove(...CARD_SIZES.map((value) => `size-${value}`));
    list.classList.add(`size-${size}`);
    document.querySelectorAll('[data-card-size]').forEach((item) => item.classList.toggle('active', item.dataset.cardSize === size));
    saveCardSize(size);
  }));
  const filter = () => {
    const needle = document.querySelector('#subscriptionSearch').value.trim().toLowerCase();
    const mode = document.querySelector('#subscriptionFilter').value;
    list.querySelectorAll('.subscription-card').forEach((node) => {
      const item = subscriptions.find((entry) => entry.url === node.dataset.url);
      const matchesText = !needle || node.textContent.toLowerCase().includes(needle);
      const matchesState = mode === 'all' || (mode === 'running' ? item?.enabled !== false : item?.enabled === false);
      node.hidden = !(matchesText && matchesState);
    });
  };
  document.querySelector('#subscriptionSearch').addEventListener('input', filter);
  document.querySelector('#subscriptionFilter').addEventListener('change', filter);
  document.querySelector('#scanNow').addEventListener('click', async (event) => {
    setLoading(event.currentTarget, true, '扫描中');
    try { const result = await api.post('/api/ui/scan'); toast(result.message || '扫描已启动'); await ctx.reload(); }
    catch (error) { toast(error.message, 'error'); }
    finally { setLoading(event.currentTarget, false); }
  });
  list.querySelectorAll('[data-action]').forEach((button) => button.addEventListener('click', async (event) => {
    event.preventDefault();
    const card = event.currentTarget.closest('.subscription-card');
    const item = subscriptions.find((entry) => entry.url === card.dataset.url);
    if (!item) return;
    if (event.currentTarget.dataset.action === 'remove') {
      if (!await confirmDialog('删除订阅', `确定删除“${item.name || item.anime_name || item.url}”吗？删除后将停止自动追踪。`)) return;
      try { await api.post('/api/ui/rss/remove', {url: item.url}); toast('订阅已删除'); await ctx.reload(); } catch (error) { toast(error.message, 'error'); }
      return;
    }
    setLoading(event.currentTarget, true, '处理中');
    try { await api.post('/api/ui/rss/toggle', {url: item.url, enabled: item.enabled === false}); toast(item.enabled === false ? '已恢复追踪' : '已暂停追踪'); await ctx.reload(); }
    catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); }
  }));
}
