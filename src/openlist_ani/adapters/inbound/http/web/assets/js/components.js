export function escapeHtml(value) {
  return String(value ?? '').replace(/[&<>"']/g, (char) => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
}

export function formatDate(value) {
  if (!value) return '—';
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? String(value) : date.toLocaleString('zh-CN', {hour12: false});
}

export function formatTime(value) {
  if (!value) return '—';
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? String(value) : date.toLocaleTimeString('zh-CN', {hour12: false});
}

export function statusBadge(state, label = state) {
  const normalized = String(state || '').toLowerCase();
  const kind = ['completed', 'downloaded', 'renamed', 'notifying'].includes(normalized) ? 'good'
    : ['failed', 'cancelled'].includes(normalized) ? 'danger'
    : ['pending', 'downloading', 'running'].includes(normalized) ? 'blue' : '';
  return `<span class="badge ${kind}">${escapeHtml(label || '未知')}</span>`;
}

export function poster(url, title, className = 'poster') {
  return url ? `<div class="${className}"><img loading="lazy" src="${escapeHtml(url)}" alt="${escapeHtml(title)}"></div>`
    : `<div class="${className}">${escapeHtml((title || '番').slice(0, 8))}</div>`;
}

export function normalizePercent(value) {
  if (value == null || value === '') return null;
  const number = Number(value);
  if (!Number.isFinite(number)) return null;
  return Math.max(0, Math.min(100, number <= 1 ? number * 100 : number));
}

export function setLoading(button, loading, label = '处理中') {
  if (!button) return;
  if (loading) {
    button.dataset.originalLabel = button.innerHTML;
    button.disabled = true;
    button.innerHTML = `<span class="spinner"></span> ${escapeHtml(label)}`;
  } else {
    button.disabled = false;
    button.innerHTML = button.dataset.originalLabel || button.innerHTML;
  }
}

export function toast(message, type = 'success') {
  const root = document.querySelector('#toast-root');
  if (!root) return;
  const node = document.createElement('div');
  node.className = `toast ${type}`;
  node.textContent = message;
  root.appendChild(node);
  setTimeout(() => node.remove(), 4200);
}

export function confirmDialog(title, message, confirmLabel = '确认') {
  return new Promise((resolve) => {
    const root = document.querySelector('#modal-root');
    root.innerHTML = `<div class="modal-backdrop"><div class="modal" role="dialog" aria-modal="true"><div class="modal-head"><h3>${escapeHtml(title)}</h3><button class="ghost-button" data-close>×</button></div><div class="modal-body"><p class="muted">${escapeHtml(message)}</p><div class="button-row"><button class="danger-button" data-confirm>${escapeHtml(confirmLabel)}</button><button class="secondary-button" data-close>取消</button></div></div></div></div>`;
    const close = (value) => { root.innerHTML = ''; resolve(value); };
    root.querySelectorAll('[data-close]').forEach((button) => button.addEventListener('click', () => close(false)));
    root.querySelector('[data-confirm]').addEventListener('click', () => close(true));
  });
}

export function renderShell(active, content, state = {}) {
  const links = [
    ['/', '⌂', '追番中心'], ['#/add', '+', '添加追番'], ['#/downloads', '⇩', '下载任务'],
    ['#/logs', '≡', '系统日志'], ['#/settings', '⚙', '设置'],
  ];
  document.querySelector('#app').innerHTML = `<div class="shell"><aside class="sidebar"><a class="brand" href="#/"><span class="brand-mark">OA</span><span>op-ani</span></a><nav class="nav">${links.map(([href, icon, label]) => `<a class="${active === href ? 'active' : ''}" href="${href}"><span class="nav-icon">${icon}</span><span>${label}</span></a>`).join('')}</nav><div class="sidebar-foot"><span>OpenList：${escapeHtml(state.openlist_url || '未配置')}</span><a href="/login" data-logout>退出登录</a></div></aside><main class="main">${content}</main></div>`;
  document.querySelector('[data-logout]')?.addEventListener('click', async (event) => {
    event.preventDefault();
    await fetch('/api/auth/logout', {method: 'POST'});
    location.href = '/login';
  });
}

export function pageHeader(title, subtitle = '', actions = '') {
  return `<header class="topbar"><div><h1>${escapeHtml(title)}</h1>${subtitle ? `<p>${escapeHtml(subtitle)}</p>` : ''}</div><div class="page-actions">${actions}</div></header>`;
}

export function emptyState(message) { return `<div class="empty">${escapeHtml(message)}</div>`; }

export function queryParam(value) { return encodeURIComponent(value || ''); }
