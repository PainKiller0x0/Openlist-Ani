import { api } from '../api.js';
import { confirmDialog, emptyState, escapeHtml, formatDate, pageHeader, poster, renderShell, setLoading, statusBadge, toast } from '../components.js';

const CARD_SIZE_KEY = 'op-ani.subscription-card-size';
const SUBSCRIPTION_FILTER_KEY = 'op-ani.subscription-filter';
const CARD_SIZES = ['large', 'medium', 'small'];
const SUBSCRIPTION_FILTERS = ['all', 'running', 'paused', 'completed'];
// Keep card positions through SPA re-renders; a full page reload resets this order.
let displayedSubscriptionOrder = null;
const ACTION_ICONS = {
  pause: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 5v14M15 5v14"/></svg>',
  play: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 5v14l11-7z"/></svg>',
  stop: '<svg viewBox="0 0 24 24" aria-hidden="true"><rect x="6" y="6" width="12" height="12" rx="1"/></svg>',
  delete: '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M4 7h16M10 11v6M14 11v6M5 7l1 14h12l1-14M9 7V4h6v3"/></svg>',
};

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

function readSubscriptionFilter() {
  try {
    const saved = localStorage.getItem(SUBSCRIPTION_FILTER_KEY);
    return SUBSCRIPTION_FILTERS.includes(saved) ? saved : 'all';
  } catch {
    return 'all';
  }
}

function saveSubscriptionFilter(filter) {
  try { localStorage.setItem(SUBSCRIPTION_FILTER_KEY, filter); } catch { /* private browsing may block storage */ }
}

function subscriptionCard(item, state) {
  const enabled = item.enabled !== false;
  const completed = item.completed === true;
  const latest = item.latest_episode ?? item.last_episode ?? null;
  const latestText = latest == null ? '暂无集数' : `更新到第 ${latest} 集`;
  const url = encodeURIComponent(item.url || '');
  const title = item.name || item.anime_name || '未命名订阅';
  const status = completed ? '已完结' : enabled ? '追踪中' : '已暂停';
  const statusType = completed ? 'completed' : enabled ? 'running' : 'paused';
  const continueAction = `<button class="ghost-button card-action-icon action-play" type="button" data-action="${completed ? 'reopen' : 'toggle'}" title="${completed ? '恢复追踪' : '继续追番'}" aria-label="${completed ? '恢复追踪' : '继续追番'}">${ACTION_ICONS.play}</button>`;
  const trackingAction = completed
    ? continueAction
    : `${enabled ? `<button class="ghost-button card-action-icon action-pause" type="button" data-action="toggle" title="暂停追番" aria-label="暂停追番">${ACTION_ICONS.pause}</button>` : continueAction}<button class="ghost-button card-action-icon action-stop" type="button" data-action="complete" title="标记已完结" aria-label="标记已完结">${ACTION_ICONS.stop}</button>`;
  return `<article class="poster-card subscription-card" data-url="${escapeHtml(item.url)}">
    <a class="subscription-card-link" href="#/subscription/${url}">${poster(item.poster_url, title)}<div class="poster-copy"><div class="poster-title" title="${escapeHtml(title)}">${escapeHtml(title)}</div><div class="poster-meta"><span>${status}</span><span>${escapeHtml(latestText)}</span></div></div></a>
    <div class="subscription-card-status">${statusBadge(statusType, status)}</div>
    <div class="card-actions">${trackingAction}<button class="ghost-button card-action-icon action-delete" type="button" data-action="remove" title="删除订阅" aria-label="删除订阅">${ACTION_ICONS.delete}</button></div>
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
  const subscriptionFilter = readSubscriptionFilter();
  const statusOrder = (item) => item.completed === true ? 2 : item.enabled === false ? 1 : 0;
  const tmdbId = (item) => {
    const id = Number(item.tmdb_id);
    return item.tmdb_id == null || !Number.isFinite(id) ? -1 : id;
  };
  const subscriptions = (state.rss_subscriptions || []).filter((item) => item.url);
  const orderByStatusAndTmdb = (a, b) => statusOrder(a) - statusOrder(b) || tmdbId(b) - tmdbId(a);
  if (displayedSubscriptionOrder) {
    const positions = new Map(displayedSubscriptionOrder.map((url, index) => [url, index]));
    subscriptions.sort((a, b) => {
      const aPosition = positions.get(a.url);
      const bPosition = positions.get(b.url);
      if (aPosition !== undefined && bPosition !== undefined) return aPosition - bPosition;
      if (aPosition !== undefined) return -1;
      if (bPosition !== undefined) return 1;
      return orderByStatusAndTmdb(a, b);
    });
  } else {
    subscriptions.sort(orderByStatusAndTmdb);
  }
  displayedSubscriptionOrder = subscriptions.map((item) => item.url);
  const failed = (state.tasks || []).filter((task) => task.state === 'failed').length;
  const running = status.running === true || status.status === 'running';
  const syncEnabled = status.enabled !== false;
  const activeCount = subscriptions.filter((item) => item.enabled !== false && item.completed !== true).length;
  const content = `${pageHeader('追番中心', '管理正在追番的番剧，保持更新同步。', `<span class="runtime-pill"><i></i> 系统${running ? '运行中' : '空闲'}</span><button class="secondary-button" id="scanNow">立即扫描</button><a class="primary-button" href="#/add">＋ 添加追番</a>`)}
    <section class="grid stats-grid dashboard-stats">
      <div class="card stat-card"><div class="stat-label">运行状态</div><div class="stat-value">${running ? '服务在线' : escapeHtml(status.status || '服务空闲')}</div><span class="stat-icon">⌁</span></div>
      <div class="card stat-card"><div class="stat-label">上次扫描</div><div class="stat-value">${escapeHtml(formatDate(status.last_scan_finished_at) || '尚未扫描')}</div><span class="stat-icon blue">↻</span></div>
      <div class="card stat-card"><div class="stat-label">下次扫描</div><div class="stat-value">${escapeHtml(formatDate(status.next_scan_at) || '等待安排')}</div><span class="stat-icon">◷</span></div>
      <div class="card stat-card"><div class="stat-label">下载错误</div><div class="stat-value">${failed} 个待处理</div><span class="stat-icon danger">△</span></div>
    </section>
    <div class="home-toolbar"><div class="toolbar-search"><span>⌕</span><input class="search-input" id="subscriptionSearch" placeholder="搜索番剧…"></div><span class="total-count">▥ 追踪中 ${activeCount} / 共 ${subscriptions.length} 部</span></div>
    <div class="grid home-content-grid"><section><div class="section-heading"><div class="section-heading-title"><h2>我的追番</h2><select class="status-filter subscription-filter" id="subscriptionFilter" aria-label="筛选追番"><option value="all" ${subscriptionFilter === 'all' ? 'selected' : ''}>全部</option><option value="running" ${subscriptionFilter === 'running' ? 'selected' : ''}>追踪中</option><option value="paused" ${subscriptionFilter === 'paused' ? 'selected' : ''}>已暂停</option><option value="completed" ${subscriptionFilter === 'completed' ? 'selected' : ''}>已完结</option></select><div class="card-size-switch" role="group" aria-label="卡片大小"><button class="card-size-button ${cardSize === 'large' ? 'active' : ''}" type="button" data-card-size="large" title="大卡片">大</button><button class="card-size-button ${cardSize === 'medium' ? 'active' : ''}" type="button" data-card-size="medium" title="中卡片">中</button><button class="card-size-button ${cardSize === 'small' ? 'active' : ''}" type="button" data-card-size="small" title="小卡片">小</button></div></div><a href="#/add">添加新的追番</a></div><div class="poster-grid size-${cardSize}" id="subscriptionList">${subscriptions.length ? subscriptions.map((item) => subscriptionCard(item, state)).join('') : emptyState('还没有追番，去添加一部吧。')}</div></section><aside class="card activity-card"><div class="card-header"><div><h2 class="section-title">最近活动</h2><div class="muted small">查看全部</div></div></div><div class="card-body">${taskActivity(state.tasks)}</div><div class="sync-notice"><span>ϟ</span><div><strong>自动同步${syncEnabled ? '已开启' : '已暂停'}</strong><small>${status.next_scan_at ? `下次扫描：${escapeHtml(formatDate(status.next_scan_at))}` : '按轮询周期检查更新。'}</small></div></div></aside></div>`;
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
      const completed = item?.completed === true;
      const matchesState = mode === 'all'
        || (mode === 'completed' ? completed
          : mode === 'running' ? !completed && item?.enabled !== false
            : !completed && item?.enabled === false);
      node.hidden = !(matchesText && matchesState);
    });
  };
  document.querySelector('#subscriptionSearch').addEventListener('input', filter);
  document.querySelector('#subscriptionFilter').addEventListener('change', (event) => {
    saveSubscriptionFilter(event.currentTarget.value);
    filter();
  });
  filter();
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
    if (event.currentTarget.dataset.action === 'complete' || event.currentTarget.dataset.action === 'reopen') {
      const completed = event.currentTarget.dataset.action === 'complete';
      if (completed && !await confirmDialog('标记已完结', `标记“${item.name || item.anime_name || item.url}”为已完结后，自动 RSS 扫描将跳过它；已排队下载不受影响。`)) return;
      setLoading(event.currentTarget, true, '处理中');
      try {
        await api.post('/api/ui/rss/completed', {url: item.url, completed});
        toast(completed ? '已完结，后续 RSS 扫描将跳过' : '已恢复追踪');
        await ctx.reload();
      } catch (error) {
        toast(error.message, 'error');
        setLoading(event.currentTarget, false);
      }
      return;
    }
    setLoading(event.currentTarget, true, '处理中');
    try { await api.post('/api/ui/rss/toggle', {url: item.url, enabled: item.enabled === false}); toast(item.enabled === false ? '已恢复追踪' : '已暂停追踪'); await ctx.reload(); }
    catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); }
  }));
}
