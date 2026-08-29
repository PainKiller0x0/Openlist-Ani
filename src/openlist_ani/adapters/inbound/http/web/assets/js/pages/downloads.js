import { api } from '../api.js';
import { confirmDialog, emptyState, escapeHtml, formatDate, normalizePercent, pageHeader, renderShell, setLoading, statusBadge, toast } from '../components.js';

function taskRow(task) {
  const percent = normalizePercent(task.progress);
  const state = String(task.state || '').toLowerCase();
  return `<article class="card task-card"><div class="task-top"><span class="task-icon">▦</span><div class="task-copy"><div class="task-name">${escapeHtml(task.anime_name || task.title || '下载任务')}${task.episode != null ? ` <span class="badge blue">第 ${escapeHtml(task.episode)} 集</span>` : ''}</div><div class="task-source">${escapeHtml(task.title || '')}</div><div class="task-meta"><span>♧ ${escapeHtml(task.fansub || '字幕组未记录')}</span><span>◉ ${escapeHtml(task.quality || '清晰度未记录')}</span><span>◷ ${escapeHtml(formatDate(task.created_at || task.updated_at))}</span></div></div><span class="task-state">${statusBadge(task.state, state === 'failed' ? '失败' : state === 'pending' ? '等待中' : state === 'completed' ? '已完成' : '进行中')}</span></div>${percent != null ? `<div class="task-progress"><div class="progress-track"><i style="width:${percent}%"></i></div><strong>${percent.toFixed(0)}%</strong></div>` : '<div class="task-progress"><div class="progress-track"><i style="width:0%"></i></div><strong>—</strong></div>'}${task.save_path ? `<div class="task-path">保存目录：${escapeHtml(task.save_path)}</div>` : ''}${task.error_message ? `<div class="task-error">${escapeHtml(task.error_message)}${task.retry_count ? ` · 已重试 ${escapeHtml(task.retry_count)} 次` : ''}</div>` : ''}<div class="task-footer">${state === 'failed' ? `<div class="button-row"><button class="secondary-button" data-retry="${escapeHtml(task.id)}">↻ 手动重试</button><button class="ghost-button" data-archive="${escapeHtml(task.id)}">存档</button></div>` : ''}<span class="muted small">更新时间：${escapeHtml(formatDate(task.updated_at))}</span></div></article>`;
}

export async function renderDownloads(ctx) {
  const [response, shellState] = await Promise.all([api.get('/api/downloads'), api.get('/api/ui/settings')]);
  const tasks = response.tasks || [];
  const failed = tasks.filter((task) => String(task.state).toLowerCase() === 'failed');
  const completed = tasks.filter((task) => ['completed', 'downloaded', 'renamed', 'notifying'].includes(String(task.state).toLowerCase()));
  const active = tasks.filter((task) => !failed.includes(task) && !completed.includes(task));
  const content = `${pageHeader('下载任务', '查看由追番规则创建的真实下载任务', '<a class="secondary-button" href="#/add">⇧ 上传 Torrent</a>')}
    <div class="download-summary"><button class="tab active" data-tab="active">进行中 <b>${active.length}</b></button><button class="tab" data-tab="failed">失败 <b>${failed.length}</b></button><button class="tab" data-tab="completed">最近完成 <b>${completed.length}</b></button><span class="muted small">↻ 最近同步于 ${escapeHtml(formatDate(new Date()))}</span></div>
    <section id="downloadList" class="download-list">${active.length ? active.map(taskRow).join('') : emptyState('当前没有进行中的下载任务。')}</section>`;
  renderShell('#/downloads', content, shellState);
  const list = document.querySelector('#downloadList');
  const bindRetry = () => list.querySelectorAll('[data-retry]').forEach((button) => button.addEventListener('click', async (event) => { setLoading(event.currentTarget, true, '重试中'); try { const result = await api.post(`/api/ui/downloads/${encodeURIComponent(button.dataset.retry)}/retry`); toast(result.message || '已重新加入任务'); await ctx.reload(); } catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); } }));
  const bindArchive = () => list.querySelectorAll('[data-archive]').forEach((button) => button.addEventListener('click', async (event) => { if (!await confirmDialog('存档失败任务', '只会隐藏这条失败记录，不会删除网盘文件或 RSS 订阅。')) return; setLoading(event.currentTarget, true, '存档中'); try { const result = await api.post(`/api/ui/downloads/${encodeURIComponent(button.dataset.archive)}/archive`); toast(result.message || '失败任务已存档'); await ctx.reload(); } catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); } }));
  const renderTab = (tab) => {
    const selected = tab === 'failed' ? failed : tab === 'completed' ? completed : active;
    list.innerHTML = selected.length ? selected.map(taskRow).join('') : emptyState(tab === 'failed' ? '没有失败任务。' : tab === 'completed' ? '暂无已完成任务。' : '当前没有进行中的下载任务。');
    document.querySelectorAll('[data-tab]').forEach((button) => button.classList.toggle('active', button.dataset.tab === tab));
    bindRetry();
    bindArchive();
  };
  document.querySelectorAll('[data-tab]').forEach((button) => button.addEventListener('click', () => renderTab(button.dataset.tab)));
  bindRetry();
  bindArchive();
}
