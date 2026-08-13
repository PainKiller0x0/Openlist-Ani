"""The small, built-in web entry point for RSS subscriptions and torrents."""

INDEX_HTML = r"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>OpenList-Ani 追番入口</title>
  <style>
    :root{color-scheme:dark;--bg:#0b1020;--card:#151d31;--line:#2b3957;--text:#edf3ff;--muted:#9aa9c7;--accent:#58a6ff;--ok:#39d98a;--bad:#ff7b86}
    *{box-sizing:border-box}body{margin:0;min-height:100vh;font:15px/1.55 system-ui,-apple-system,"Microsoft YaHei",sans-serif;color:var(--text);background:radial-gradient(circle at top,#192640 0,#0b1020 52%)}main{width:min(980px,calc(100% - 32px));margin:34px auto 60px}header{display:flex;justify-content:space-between;align-items:flex-start;gap:20px;margin-bottom:22px}h1{margin:0 0 4px;font-size:27px}h2{margin:0 0 6px;font-size:19px}p{margin:6px 0}.muted{color:var(--muted)}.small{font-size:13px}.card{background:rgba(21,29,49,.94);border:1px solid var(--line);border-radius:15px;padding:22px;margin:16px 0;box-shadow:0 12px 35px rgba(0,0,0,.17)}.grid{display:grid;grid-template-columns:1fr 1fr;gap:16px}@media(max-width:760px){.grid{grid-template-columns:1fr}header{display:block}}label{display:block;color:var(--muted);font-size:13px;margin:14px 0 6px}input[type=url],input[type=text],input[type=file]{width:100%;padding:11px 12px;border:1px solid var(--line);border-radius:9px;background:#0c1426;color:var(--text);outline:none}input:focus{border-color:var(--accent);box-shadow:0 0 0 3px rgba(88,166,255,.14)}button{border:0;border-radius:9px;padding:10px 15px;color:#07111f;background:var(--accent);font-weight:700;cursor:pointer}button.secondary{color:var(--text);background:#263653}button:disabled{opacity:.55;cursor:wait}.actions{display:flex;gap:10px;flex-wrap:wrap;align-items:center;margin-top:15px}.note{padding:11px 13px;border:1px solid #385175;border-radius:9px;background:#101a2d;color:var(--muted)}.status{min-height:24px;margin-top:12px;color:var(--ok);white-space:pre-wrap}.status.bad{color:var(--bad)}ul{margin:10px 0 0;padding:0;list-style:none}li{padding:9px 0;border-bottom:1px solid rgba(43,57,87,.75);overflow-wrap:anywhere}li:last-child{border-bottom:0}code{color:#b8d7ff}.pill{display:inline-block;padding:2px 8px;border-radius:99px;background:#253653;color:#cfe3ff;font-size:12px;margin-left:7px}.top-actions{display:flex;gap:9px;align-items:center}a{color:#75b7ff;text-decoration:none}a:hover{text-decoration:underline}
  </style>
</head>
<body>
<main>
  <header>
    <div><h1>OpenList-Ani 追番入口</h1><p class="muted">RSS 自动追新番，或上传种子立即交给迅雷离线下载。</p></div>
    <div class="top-actions"><button class="secondary" onclick="refresh()">刷新状态</button><a href="/docs" target="_blank">API 文档</a></div>
  </header>
  <div class="grid">
    <section class="card">
      <h2>RSS 订阅</h2>
      <p class="muted small">填入 Mikan 等来源的 RSS 链接。保存后会立即更新追踪器，后续新发布的条目会自动下载。</p>
      <label for="rssUrl">RSS 链接</label><input id="rssUrl" type="url" placeholder="https://mikanani.me/RSS/Subscription/..." autocomplete="off">
      <div class="actions"><button id="rssBtn" onclick="addRss()">添加并开始追踪</button></div><div id="rssStatus" class="status"></div>
      <label>当前订阅</label><ul id="rssList"><li class="muted">加载中…</li></ul>
    </section>
    <section class="card">
      <h2>上传种子</h2>
      <p class="muted small">适合补旧番、季度合集或 RSS 之外的单个资源。上传后会解析种子并立即创建迅雷下载任务。</p>
      <label for="torrentFile">.torrent 文件</label><input id="torrentFile" type="file" accept=".torrent,application/x-bittorrent">
      <label for="torrentTitle">名称（可选，留空使用种子名称）</label><input id="torrentTitle" type="text" maxlength="200" placeholder="例如：[组名] 番剧名 - 01 [1080p]">
      <div class="note small" style="margin-top:14px">目标目录：<code id="targetPath">读取中…</code></div>
      <div class="actions"><button id="torrentBtn" onclick="uploadTorrent()">上传并开始下载</button></div><div id="torrentStatus" class="status"></div>
    </section>
  </div>
  <section class="card"><h2>下载任务</h2><p class="muted small">这里显示 OpenList-Ani 当前仍在跟进的任务；完成后会按既有规则整理到番库。</p><ul id="taskList"><li class="muted">加载中…</li></ul></section>
</main>
<script>
  const $=id=>document.getElementById(id);
  function setStatus(id,message,bad=false){const e=$(id);e.textContent=message||'';e.classList.toggle('bad',!!bad)}
  async function api(path,options={}){const r=await fetch(path,options),text=await r.text();let data={};try{data=text?JSON.parse(text):{}}catch{data={message:text}}if(!r.ok)throw new Error(data.detail||data.message||`请求失败（${r.status}）`);return data}
  function esc(s){return String(s??'').replace(/[&<>"']/g,m=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[m]))}
  async function refresh(){try{const s=await api('/api/ui/state');$('targetPath').textContent=s.download_path||'由配置决定';$('rssList').innerHTML=s.rss_urls.length?s.rss_urls.map(u=>`<li>${esc(u)}</li>`).join(''):'<li class="muted">还没有 RSS 订阅</li>';$('taskList').innerHTML=s.tasks.length?s.tasks.map(t=>`<li><strong>${esc(t.title)}</strong><span class="pill">${esc(t.state)}</span><div class="muted small">${esc(t.anime_name||'')} ${t.episode?`· 第 ${esc(t.episode)} 集`:''}</div></li>`).join(''):'<li class="muted">暂无进行中的任务</li>'}catch(e){setStatus('rssStatus','状态读取失败：'+e.message,true)}}
  async function addRss(){const url=$('rssUrl').value.trim();if(!url){setStatus('rssStatus','请先填写 RSS 链接',true);return}const b=$('rssBtn');b.disabled=true;setStatus('rssStatus','正在验证并保存 RSS…');try{const d=await api('/api/ui/rss',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({url})});setStatus('rssStatus',d.message+(d.preview?`\n示例：${d.preview}`:''));$('rssUrl').value='';await refresh()}catch(e){setStatus('rssStatus',e.message,true)}finally{b.disabled=false}}
  async function uploadTorrent(){const f=$('torrentFile').files[0];if(!f){setStatus('torrentStatus','请选择 .torrent 文件',true);return}const b=$('torrentBtn');b.disabled=true;setStatus('torrentStatus','正在上传、解析种子并创建迅雷任务…');try{const body=await f.arrayBuffer(),d=await api('/api/ui/torrent',{method:'POST',headers:{'Content-Type':'application/x-bittorrent','X-Filename':encodeURIComponent(f.name),'X-Title':encodeURIComponent($('torrentTitle').value.trim())},body});setStatus('torrentStatus',d.message);$('torrentFile').value='';$('torrentTitle').value='';await refresh()}catch(e){setStatus('torrentStatus',e.message,true)}finally{b.disabled=false}}
  refresh();setInterval(refresh,15000);
</script>
</body></html>"""
