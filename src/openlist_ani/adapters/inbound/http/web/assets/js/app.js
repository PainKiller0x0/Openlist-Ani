import { api } from './api.js';
import { escapeHtml, pageHeader, renderShell } from './components.js';
import { renderTracking } from './pages/tracking.js?v=20260824-card-size-2';
import { renderAdd, renderSelect } from './pages/add.js';
import { renderPreview } from './pages/preview.js';
import { renderDetails } from './pages/details.js';
import { renderDownloads } from './pages/downloads.js';
import { renderLogs } from './pages/logs.js';
import { renderSettings } from './pages/settings.js';

const flow = {};

function navigate(hash) { if (location.hash === hash) renderRoute(); else location.hash = hash; }

async function renderRoute() {
  const hash = location.hash || '#/';
  const ctx = {flow, navigate, reload: renderRoute};
  try {
    if (hash === '#/' || hash === '#') return await renderTracking(ctx);
    if (hash === '#/add') return await renderAdd(ctx);
    if (hash === '#/add/select') return await renderSelect(ctx);
    if (hash === '#/add/preview') return await renderPreview(ctx);
    if (hash === '#/downloads') return await renderDownloads(ctx);
    if (hash === '#/logs') return await renderLogs(ctx);
    if (hash === '#/settings') return await renderSettings(ctx);
    const match = hash.match(/^#\/subscription\/(.+)$/);
    if (match) return await renderDetails(ctx, match[1]);
    navigate('#/');
  } catch (error) {
    renderShell('/', `${pageHeader('页面加载失败', '后端接口返回了错误', '<button class="secondary-button" id="retryPage">重试</button>')}<div class="card notice">${escapeHtml(error.message || '未知错误')}</div>`, {});
    document.querySelector('#retryPage')?.addEventListener('click', renderRoute);
  }
}

window.addEventListener('hashchange', renderRoute);
renderRoute();
