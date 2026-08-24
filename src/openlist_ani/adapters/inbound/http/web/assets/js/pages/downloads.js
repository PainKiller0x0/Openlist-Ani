import { api } from '../api.js';
import { confirmDialog, emptyState, escapeHtml, formatDate, normalizePercent, pageHeader, renderShell, setLoading, statusBadge, toast } from '../components.js';

function taskRow(task) {
  const percent = normalizePercent(task.progress);
  return `<article class="card" style="padding:16px"><div class="button-row"><strong>${escapeHtml(task.title)}</strong>${statusBadge(task.state)}${task.retry_count ? `<span class="muted small">重试 ${task.retry_count} 次</span>` : ''}</div><div class="muted small" style="margin-top:6px">${escapeHtml(task.anime_name || '')}${task.season != null ? ` · 第 ${task.season} 季` : ''}${task.episode != null ? ` · 第 ${task.episode} 集` : ''}</div>${percent != null ? `<div style="margin-top:12px"><div class="muted small">进度 ${percent.toFixed(1)}%</div><progress value="${percent}" max="100" style="width:100%"></progress></div>` : ''}${task.save_path ? `<div class="muted small" style="margin-top:7px;word-break:break-all">保存路径：${escapeHtml(task.save_path)}</div>` : ''}${task.error_message ? `<div class="small" style="margin-top:7px;color:var(--danger);word-break:break-word">${escapeHtml(task.error_message)}</div>` : ''}<div class="button-row" style="margin-top:12px">${task.state === 'failed' ? `<button class="secondary-button" data-retry="${escapeHtml(task.id)}">手动重试</button>` : ''}<span class="muted small">更新时间：${escapeHtml(formatDate(task.updated_at))}</span></div></article>`;
}

export async function renderDownloads(ctx) {
  const response = await api.get('/api/downloads');
  const tasks = response.tasks || [];
  const failed = tasks.filter((task) => task.state === 'failed');
  const active = tasks.filter((task) => task.state !== 'failed');
  const content = `${pageHeader('下载任务', '查看真实下载状态，失败任务可以手动重试', '<a class="primary-button" href="#/add">上传种子</a><button class="secondary-button" id="refreshDownloads">刷新</button>')}
    <div class="tabs"><button class="tab active" data-tab="active">进行中 (${active.length})</button><button class="tab" data-tab="failed">失败 (${failed.length})</button><button class="tab" data-tab="completed">已完成（由后端归档）</button></div>
    <section id="downloadList" class="grid">${active.length ? active.map(taskRow).join('') : emptyState('当前没有进行中的下载任务。')}</section>`;
  renderShell('#/downloads', content, {});
  const list = document.querySelector('#downloadList');
  const renderTab = (tab) => { list.innerHTML = tab === 'failed' ? (failed.length ? failed.map(taskRow).join('') : emptyState('没有失败任务。')) : tab === 'completed' ? emptyState('当前 API 不返回已归档任务，没有虚构的完成列表。') : (active.length ? active.map(taskRow).join('') : emptyState('当前没有进行中的下载任务。')); bindRetry(); document.querySelectorAll('[data-tab]').forEach((button) => button.classList.toggle('active', button.dataset.tab === tab)); };
  const bindRetry = () => list.querySelectorAll('[data-retry]').forEach((button) => button.addEventListener('click', async (event) => { setLoading(event.currentTarget, true, '重试中'); try { const result = await api.post(`/api/ui/downloads/${encodeURIComponent(button.dataset.retry)}/retry`); toast(result.message || '已重新加入任务'); await ctx.reload(); } catch (error) { toast(error.message, 'error'); setLoading(event.currentTarget, false); } }));
  document.querySelectorAll('[data-tab]').forEach((button) => button.addEventListener('click', () => renderTab(button.dataset.tab)));
  document.querySelector('#refreshDownloads').addEventListener('click', () => ctx.reload());
  bindRetry();
}
