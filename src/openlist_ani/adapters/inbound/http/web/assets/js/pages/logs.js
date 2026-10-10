import { api } from '../api.js';
import { escapeHtml, pageHeader, renderShell, toast } from '../components.js';

const LOG_LEVELS = ['DEBUG', 'INFO', 'WARNING', 'ERROR', 'FATAL'];

export async function renderLogs(ctx) {
  const [response, shellState] = await Promise.all([api.get('/api/ui/logs?limit=300'), api.get('/api/ui/settings')]);
  const lines = [...(response.lines || [])].reverse();
  const levelOptions = ['<option value="">全部级别</option>', ...LOG_LEVELS.map((level) => `<option value="${level}">${level}</option>`)].join('');
  const content = `${pageHeader('系统日志', '查看后端实际日志，便于定位 RSS、OpenList 和下载链路问题', '<button class="secondary-button" id="refreshLogs">刷新日志</button>')}
    <section class="card"><div class="toolbar"><input class="search-input" id="logSearch" placeholder="筛选日志关键词"><select class="search-input" id="logLevel" aria-label="按日志级别筛选">${levelOptions}</select><label class="button-row muted small"><input type="checkbox" id="logAuto"> 自动刷新（30 秒）</label></div><div class="card-body"><pre class="log-viewer" id="logViewer">${escapeHtml(lines.join('\n'))}</pre></div></section>`;
  renderShell('#/logs', content, shellState);
  const viewer = document.querySelector('#logViewer');
  const applyFilters = () => {
    const needle = document.querySelector('#logSearch').value.trim().toLowerCase();
    const level = document.querySelector('#logLevel').value;
    viewer.textContent = lines.filter((line) => {
      const lineLevel = line.match(/\|\s*([A-Z]+)\s+\|/)?.[1] || '';
      return (!level || lineLevel === level) && (!needle || line.toLowerCase().includes(needle));
    }).join('\n');
  };
  document.querySelector('#logSearch').addEventListener('input', applyFilters);
  document.querySelector('#logLevel').addEventListener('change', applyFilters);
  document.querySelector('#refreshLogs').addEventListener('click', () => ctx.reload());
  let timer = null;
  document.querySelector('#logAuto').addEventListener('change', (event) => { if (event.target.checked) timer = setInterval(() => ctx.reload(), 30000); else clearInterval(timer); });
}
