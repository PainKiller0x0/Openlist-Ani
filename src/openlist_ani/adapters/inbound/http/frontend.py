"""The small, built-in web entry point for RSS subscriptions and torrents."""

INDEX_HTML = r"""<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width,initial-scale=1">
  <title>OpenList-Ani 追番</title>
  <style>
    :root{color-scheme:dark;--bg:#0b1020;--card:#151d31;--line:#2b3957;--text:#edf3ff;--muted:#9aa9c7;--accent:#58a6ff;--ok:#39d98a;--bad:#ff7b86;--warn:#f6c85f}
    *{box-sizing:border-box}body{margin:0;min-height:100vh;font:15px/1.55 system-ui,-apple-system,"Microsoft YaHei",sans-serif;color:var(--text);background:radial-gradient(circle at top,#192640 0,#0b1020 52%)}main{width:min(1050px,calc(100% - 32px));margin:30px auto 60px}header{display:flex;justify-content:space-between;align-items:flex-start;gap:20px;margin-bottom:20px}h1{margin:0 0 4px;font-size:28px}h2{margin:0;font-size:20px}p{margin:6px 0}.muted{color:var(--muted)}.small{font-size:13px}.card{background:rgba(21,29,49,.95);border:1px solid var(--line);border-radius:15px;padding:22px;margin:16px 0;box-shadow:0 12px 35px rgba(0,0,0,.17)}.grid{display:grid;grid-template-columns:1.25fr .75fr;gap:16px}@media(max-width:820px){.grid{grid-template-columns:1fr}header{display:block}}label{display:block;color:var(--muted);font-size:13px;margin:14px 0 6px}input[type=url],input[type=text],input[type=file]{width:100%;padding:11px 12px;border:1px solid var(--line);border-radius:9px;background:#0c1426;color:var(--text);outline:none}input:focus{border-color:var(--accent);box-shadow:0 0 0 3px rgba(88,166,255,.14)}button{border:0;border-radius:9px;padding:10px 15px;color:#07111f;background:var(--accent);font-weight:700;cursor:pointer}button.secondary{color:var(--text);background:#263653}button.danger{color:#ffd9dd;background:#512d3b}button:disabled{opacity:.55;cursor:wait}.actions{display:flex;gap:10px;flex-wrap:wrap;align-items:center;margin-top:15px}.status{min-height:24px;margin-top:12px;color:var(--ok);white-space:pre-wrap}.status.bad{color:var(--bad)}.status.warn{color:var(--warn)}.note{padding:12px 14px;border:1px solid #385175;border-radius:9px;background:#101a2d;color:var(--muted)}.runtime{display:flex;align-items:center;gap:11px;margin:16px 0;padding:13px 14px;border:1px solid #275f4c;border-radius:10px;background:#10251f}.dot{width:10px;height:10px;border-radius:50%;background:var(--ok);box-shadow:0 0 12px var(--ok)}.dot.warn{background:var(--warn);box-shadow:0 0 12px var(--warn)}.runtime strong{color:#d9ffe9}.metrics{display:grid;grid-template-columns:repeat(3,1fr);gap:10px;margin-top:13px}.metric{padding:10px;border-radius:9px;background:#0d1629;border:1px solid var(--line)}.metric b{display:block;font-size:13px;color:var(--muted);font-weight:500}.metric span{display:block;margin-top:3px;font-size:13px}.feed{display:grid;grid-template-columns:68px minmax(0,1fr) auto;align-items:center;gap:15px;padding:14px 0;border-top:1px solid rgba(43,57,87,.75)}.feed:first-child{border-top:0}.feed-poster{width:68px;height:96px;object-fit:cover;border-radius:9px;background:#0d1629;border:1px solid var(--line)}.feed-placeholder{display:flex;align-items:center;justify-content:center;color:var(--muted);font-size:20px}.feed-main{min-width:0}.feed-title{font-size:16px;font-weight:700}.feed-url{overflow-wrap:anywhere;color:var(--muted);font-size:13px;margin-top:4px}.feed-actions{display:flex;gap:8px;flex-wrap:wrap;justify-content:flex-end}.pill{display:inline-block;padding:2px 8px;border-radius:99px;background:#253653;color:#cfe3ff;font-size:12px;margin-left:7px}.pill.paused{background:#4c3c25;color:#ffe2a5}.top-actions{display:flex;gap:9px;align-items:center;flex-wrap:wrap}@media(max-width:600px){.feed{grid-template-columns:56px minmax(0,1fr)}.feed-poster{width:56px;height:80px}.feed-actions{grid-column:2;justify-content:flex-start}}a{color:#75b7ff;text-decoration:none}a:hover{text-decoration:underline}ul{margin:10px 0 0;padding:0;list-style:none}li{padding:9px 0;border-bottom:1px solid rgba(43,57,87,.75);overflow-wrap:anywhere}li:last-child{border-bottom:0}code{color:#b8d7ff}
  </style>
</head>
<body>
<main>
  <header>
    <div><h1>OpenList-Ani 追番</h1><p class="muted">订阅 RSS，自动检查新集数；需要补旧番时也可以直接上传种子。</p></div>
    <div class="top-actions"><button class="secondary" onclick="refresh()">刷新状态</button><a href="/docs" target="_blank">API 文档</a></div>
  </header>

  <section class="card">
    <div class="top-actions" style="justify-content:space-between"><div><h2>RSS 订阅</h2><p class="muted small">这是持续追番入口，不是一次性下载。添加成功后订阅会保持运行。</p></div><button id="scanBtn" onclick="scanNow()">立即扫描</button></div>
    <div id="runtime" class="runtime"><span class="dot"></span><div><strong id="runtimeTitle">读取运行状态…</strong><div id="runtimeText" class="muted small"></div></div></div>
    <div class="metrics"><div class="metric"><b>轮询间隔</b><span id="interval">—</span></div><div class="metric"><b>最近扫描</b><span id="lastScan">—</span></div><div class="metric"><b>下次扫描</b><span id="nextScan">—</span></div></div>
    <label for="rssName">番剧名称（可选）</label>
    <input id="rssName" type="text" maxlength="200" placeholder="例如：尼古喵喵；留空将由 LLM / RSS 自动识别" autocomplete="off">
    <label for="rssUrl">RSS 链接</label>
    <div class="top-actions"><input id="rssUrl" type="url" placeholder="https://mikanani.me/RSS/Subscription/..." autocomplete="off"><button id="rssBtn" onclick="addRss()">添加并识别</button></div>
    <div id="rssStatus" class="status"></div>
    <label>已添加的订阅</label><div id="rssList"><div class="muted">加载中…</div></div>
  </section>

  <div class="grid">
    <section class="card">
      <h2>补番：上传种子</h2>
      <p class="muted small">上传后会解析种子并立即创建迅雷任务，仍使用当前保存路径和重命名规则。</p>
      <label for="torrentFile">.torrent 文件</label><input id="torrentFile" type="file" accept=".torrent,application/x-bittorrent">
      <label for="torrentTitle">名称（可选）</label><input id="torrentTitle" type="text" maxlength="200" placeholder="留空使用种子名称">
      <div class="note small" style="margin-top:14px">目标目录：<code id="targetPath">读取中…</code></div>
      <div class="actions"><button id="torrentBtn" onclick="uploadTorrent()">上传并开始下载</button></div><div id="torrentStatus" class="status"></div>
    </section>
    <section class="card">
      <h2>进行中的任务</h2>
      <p class="muted small">下载完成后任务会从这里消失，并进入现有整理流程。</p>
      <ul id="taskList"><li class="muted">加载中…</li></ul>
    </section>
  </div>
</main>
<script>
  const $=id=>document.getElementById(id);
  function setStatus(id,message,bad=false){const e=$(id);e.textContent=message||'';e.classList.toggle('bad',!!bad)}
  function fmt(value){if(!value)return '—';const d=new Date(value);return Number.isNaN(d.getTime())?value:d.toLocaleString('zh-CN',{hour12:false})}
  async function api(path,options={}){const r=await fetch(path,options),text=await r.text();let data={};try{data=text?JSON.parse(text):{}}catch{data={message:text}}if(!r.ok)throw new Error(data.detail||data.message||`请求失败（${r.status}）`);return data}
  function esc(s){return String(s??'').replace(/[&<>"']/g,m=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[m]))}
  function renderStatus(s){const r=s.rss_status||{};const running=!!r.running;const failed=r.status==='error';$('runtimeTitle').textContent=running?'正在扫描 RSS…':failed?'RSS 扫描异常':r.enabled?'RSS 追踪运行中':'RSS 追踪未启用';$('runtimeText').textContent=running?'正在拉取订阅并检查新条目，请稍候。':(r.message||'等待下一次轮询。');$('runtime').style.borderColor=failed?'#6b3540':running?'#66552a':'#275f4c';$('runtime').querySelector('.dot').classList.toggle('warn',failed||running);$('interval').textContent=r.interval_seconds?`${Math.round(r.interval_seconds/60)} 分钟`: '—';$('lastScan').textContent=r.last_scan_finished_at?`${fmt(r.last_scan_finished_at)}（${r.last_scan_new_count||0} 个新条目）`:'尚未扫描';$('nextScan').textContent=fmt(r.next_scan_at);$('scanBtn').disabled=running}
  async function refresh(){try{const s=await api('/api/ui/state');renderStatus(s);$('targetPath').textContent=s.download_path||'由配置决定';const feeds=s.rss_subscriptions&&s.rss_subscriptions.length?s.rss_subscriptions:(s.rss_urls||[]).map(url=>({url,name:'',enabled:true}));$('rssList').innerHTML=feeds.length?feeds.map(f=>{const enabled=f.enabled!==false;const poster=f.poster_url?`<img class="feed-poster" src="${esc(f.poster_url)}" alt="" loading="lazy">`:'<div class="feed-poster feed-placeholder">番</div>';return `<div class="feed">${poster}<div class="feed-main"><div class="feed-title">${esc(f.name||'未命名订阅')}<span class="pill ${enabled?'':'paused'}">${enabled?'追番中':'已暂停'}</span></div><div class="feed-url">${esc(f.url)}</div>${f.tmdb_id?`<div class="muted small">TMDb #${esc(f.tmdb_id)}</div>`:''}</div><div class="feed-actions"><button class="secondary" onclick="refreshMetadata(${JSON.stringify(f.url)},${JSON.stringify(f.name||'')},${f.tmdb_id||'null'})">识别信息</button><button class="secondary" onclick="toggleRss(${JSON.stringify(f.url)},${!enabled})">${enabled?'暂停追番':'继续追番'}</button><button class="danger" onclick="removeRss(${JSON.stringify(f.url)})">删除</button></div></div>`}).join(''):'<div class="muted">还没有订阅。添加 RSS 后，这里会显示名称、海报和运行状态。</div>';$('taskList').innerHTML=s.tasks.length?s.tasks.map(t=>`<li><strong>${esc(t.title)}</strong><span class="pill">${esc(t.state)}</span><div class="muted small">${esc(t.anime_name||'')} ${t.episode?`· 第 ${esc(t.episode)} 集`:''}</div></li>`).join(''):'<li class="muted">暂无进行中的任务</li>'}catch(e){setStatus('rssStatus','状态读取失败：'+e.message,true)}}
  async function scanNow(){const b=$('scanBtn');b.disabled=true;setStatus('rssStatus','正在手动扫描 RSS，请稍候…');try{const d=await api('/api/ui/scan',{method:'POST'});renderStatus({rss_status:d});setStatus('rssStatus',d.message||'手动扫描完成');await refresh()}catch(e){setStatus('rssStatus',e.message,true)}finally{b.disabled=false}}
  async function addRss(){const url=$('rssUrl').value.trim(),name=$('rssName').value.trim();if(!url){setStatus('rssStatus','请先填写 RSS 链接',true);return}const b=$('rssBtn');b.disabled=true;setStatus('rssStatus','正在验证 RSS，并尝试识别番剧名称与 TMDb 海报…');try{const d=await api('/api/ui/rss',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({url,name})});setStatus('rssStatus',d.message+(d.name?`\n识别名称：${d.name}`:'')+(d.preview?`\n最新条目：${d.preview}`:''));$('rssUrl').value='';$('rssName').value='';await refresh()}catch(e){setStatus('rssStatus',e.message,true)}finally{b.disabled=false}}
  async function toggleRss(url,enabled){setStatus('rssStatus',enabled?'正在恢复追番…':'正在暂停追番…');try{const d=await api('/api/ui/rss/toggle',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({url,enabled})});setStatus('rssStatus',d.message);await refresh()}catch(e){setStatus('rssStatus',e.message,true)}}
  async function refreshMetadata(url,name,tmdb_id){setStatus('rssStatus','正在重新识别订阅名称、TMDb 和海报…');try{const d=await api('/api/ui/rss/metadata',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({url,name,tmdb_id})});setStatus('rssStatus',`已更新：${d.name||'未识别名称'}${d.tmdb_id?` · TMDb #${d.tmdb_id}`:''}`);await refresh()}catch(e){setStatus('rssStatus',e.message,true)}}
  async function removeRss(url){if(!confirm('删除这个订阅？删除后会丢失名称和海报；以后想暂时停用请使用“暂停追番”。'))return;try{const d=await api('/api/ui/rss',{method:'DELETE',headers:{'Content-Type':'application/json'},body:JSON.stringify({url})});setStatus('rssStatus',d.message);await refresh()}catch(e){setStatus('rssStatus',e.message,true)}}
  async function uploadTorrent(){const f=$('torrentFile').files[0];if(!f){setStatus('torrentStatus','请选择 .torrent 文件',true);return}const b=$('torrentBtn');b.disabled=true;setStatus('torrentStatus','正在上传、解析种子并创建迅雷任务…');try{const body=await f.arrayBuffer(),d=await api('/api/ui/torrent',{method:'POST',headers:{'Content-Type':'application/x-bittorrent','X-Filename':encodeURIComponent(f.name),'X-Title':encodeURIComponent($('torrentTitle').value.trim())},body});setStatus('torrentStatus',d.message);$('torrentFile').value='';$('torrentTitle').value='';await refresh()}catch(e){setStatus('torrentStatus',e.message,true)}finally{b.disabled=false}}
  refresh();setInterval(refresh,15000);
</script>
</body></html>"""
