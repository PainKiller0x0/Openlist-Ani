import { api } from '../api.js';
import { escapeHtml, pageHeader, renderShell, toast } from '../components.js';

export async function renderLogs(ctx) {
  const response = await api.get('/api/ui/logs?limit=300');
  const lines = response.lines || [];
  const content = `${pageHeader('系统日志', '查看后端实际日志，便于定位 RSS、OpenList 和下载链路问题', '<button class="secondary-button" id="refreshLogs">刷新日志</button>')}
    <section class="card"><div class="toolbar"><input class="search-input" id="logSearch" placeholder="筛选日志关键词"><label class="button-row muted small"><input type="checkbox" id="logAuto"> 自动刷新（30 秒）</label></div><div class="card-body"><pre class="log-viewer" id="logViewer">${escapeHtml(lines.join('\n'))}</pre></div></section>`;
  renderShell('#/logs', content, {});
  const viewer = document.querySelector('#logViewer');
  const raw = lines.join('\n');
  document.querySelector('#logSearch').addEventListener('input', (event) => { const needle = event.target.value.trim().toLowerCase(); viewer.textContent = needle ? raw.split('\n').filter((line) => line.toLowerCase().includes(needle)).join('\n') : raw; });
  document.querySelector('#refreshLogs').addEventListener('click', () => ctx.reload());
  let timer = null;
  document.querySelector('#logAuto').addEventListener('change', (event) => { if (event.target.checked) timer = setInterval(() => ctx.reload(), 30000); else clearInterval(timer); });
}
