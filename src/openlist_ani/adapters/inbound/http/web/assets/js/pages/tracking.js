import { api } from '../api.js';
import { confirmDialog, emptyState, escapeHtml, formatDate, pageHeader, poster, renderShell, setLoading, statusBadge, toast } from '../components.js';

function subscriptionCard(item, state) {
  const enabled = item.enabled !== false;
  const latest = item.latest_episode ?? item.last_episode ?? null;
  const latestText = latest == null ? '尚未记录下载集数' : `更新到第 ${latest} 集`;
  const url = encodeURIComponent(item.url || '');
  return `<article class="subscription" data-url="${escapeHtml(item.url)}">
    ${poster(item.poster_url, item.name || item.anime_name || '订阅')}
    <div><div class="subscription-title"><a href="#/subscription/${url}">${escapeHtml(item.name || item.anime_name || '未命名订阅')}</a>${statusBadge(enabled ? 'running' : 'paused', enabled ? '追踪中' : '已暂停')}</div>
      <div class="subscription-url" title="${escapeHtml(item.url)}">${escapeHtml(item.url)}</div>
      <div class="muted small">${escapeHtml(latestText)} · ${escapeHtml(item.download_directory || state.download_path || '由配置决定')}</div>
    </div>
    <div class="subscription-actions"><button class="secondary-button" data-action="toggle">${enabled ? '暂停' : '继续'}</button><a class="secondary-button" href="#/subscription/${url}">详情</a><button class="danger-button" data-action="remove">删除</button></div>
  </article>`;
}

function taskActivity(tasks) {
  const recent = [...(tasks || [])].sort((a, b) => String(b.updated_at || '').localeCompare(String(a.updated_at || ''))).slice(0, 8);
  if (!recent.length) return emptyState('暂无进行中的下载任务');
  return `<ul class="activity-list">${recent.map((task) => `<li><div class="button-row"><strong>${escapeHtml(task.title)}</strong>${statusBadge(task.state)}</div><div class="muted small">${escapeHtml(task.anime_name || '')}${task.episode != null ? ` · 第 ${task.episode} 集` : ''}</div>${task.error_message ? `<div class="small" style="color:var(--danger)">${escapeHtml(task.error_message)}</div>` : ''}</li>`).join('')}</ul>`;
}

export async function renderTracking(ctx) {
  const state = await api.get('/api/ui/state');
  const status = state.rss_status || {};
  const subscriptions = (state.rss_subscriptions || []).filter((item) => item.url);
  const failed = (state.tasks || []).filter((task) => task.state === 'failed').length;
  const running = status.running === true || status.status === 'running';
  const content = `${pageHeader('追番中心', '管理 RSS 订阅、查看实际运行状态和最近任务', '<button class="primary-button" id="scanNow">立即扫描</button><button class="secondary-button" id="refreshState">刷新</button>')}
    <section class="grid stats-grid">
      <div class="card stat-card"><div class="stat-label">订阅</div><div class="stat-value">${subscriptions.length}</div><div class="muted small">${subscriptions.filter((item) => item.enabled !== false).length} 个追踪中</div></div>
      <div class="card stat-card"><div class="stat-label">RSS 状态</div><div class="stat-value">${running ? '运行中' : escapeHtml(status.status || '空闲')}</div><div class="muted small">下一次：${escapeHtml(formatDate(status.next_scan_at))}</div></div>
      <div class="card stat-card"><div class="stat-label">上次扫描</div><div class="stat-value">${escapeHtml(status.last_scan_new_count ?? 0)}</div><div class="muted small">新增条目 · ${escapeHtml(formatDate(status.last_scan_finished_at))}</div></div>
      <div class="card stat-card"><div class="stat-label">失败任务</div><div class="stat-value">${failed}</div><div class="muted small">前往下载任务查看重试</div></div>
    </section>
    <div class="grid content-grid" style="margin-top:16px"><section class="card"><div class="card-header"><div><h2 class="section-title">RSS 订阅</h2><div class="muted small">已保存的订阅和实际下载目录</div></div><input class="search-input" id="subscriptionSearch" style="max-width:240px" placeholder="搜索订阅"></div><div class="card-body"><div class="subscription-list" id="subscriptionList">${subscriptions.length ? subscriptions.map((item) => subscriptionCard(item, state)).join('') : emptyState('还没有订阅，去添加追番吧。')}</div></div></section><aside class="card"><div class="card-header"><div><h2 class="section-title">最近活动</h2><div class="muted small">只展示后端实际返回的任务</div></div></div><div class="card-body">${taskActivity(state.tasks)}</div></aside></div>`;
  renderShell('/', content, state);

  const list = document.querySelector('#subscriptionList');
  document.querySelector('#subscriptionSearch').addEventListener('input', (event) => {
    const needle = event.target.value.trim().toLowerCase();
    list.querySelectorAll('.subscription').forEach((node) => { node.hidden = needle && !node.textContent.toLowerCase().includes(needle); });
  });
  document.querySelector('#refreshState').addEventListener('click', () => ctx.reload());
  document.querySelector('#scanNow').addEventListener('click', async (event) => {
    setLoading(event.currentTarget, true, '扫描中');
    try { const result = await api.post('/api/ui/scan'); toast(result.message || '扫描已启动'); await ctx.reload(); }
    catch (error) { toast(error.message, 'error'); }
    finally { setLoading(event.currentTarget, false); }
  });
  list.querySelectorAll('[data-action]').forEach((button) => button.addEventListener('click', async (event) => {
    const card = event.currentTarget.closest('.subscription');
    const item = subscriptions.find((entry) => entry.url === card.dataset.url);
    if (!item) return;
    if (event.currentTarget.dataset.action === 'remove') {
      if (!await confirmDialog('删除订阅', `确定删除“${item.name || item.anime_name || item.url}”吗？`)) return;
      try { await api.post('/api/ui/rss/remove', {url: item.url}); toast('订阅已删除'); await ctx.reload(); }
      catch (error) { toast(error.message, 'error'); }
      return;
    }
    try { await api.post('/api/ui/rss/toggle', {url: item.url, enabled: item.enabled === false}); toast(item.enabled === false ? '已恢复追踪' : '已暂停追踪'); await ctx.reload(); }
    catch (error) { toast(error.message, 'error'); }
  }));
}
