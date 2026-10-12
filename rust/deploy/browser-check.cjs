const fs = require('fs');
const { chromium } = require(process.env.PLAYWRIGHT_PACKAGE || 'playwright');

(async () => {
  const base = process.env.OPENLIST_ANI_TEST_URL || 'http://127.0.0.1:27666';
  const storageState = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
  storageState.cookies = storageState.cookies.map(cookie => ({ ...cookie, domain: new URL(base).hostname, secure: base.startsWith('https:') }));
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH });
  const context = await browser.newContext({ storageState, viewport: { width: 1440, height: 1000 } });
  const page = await context.newPage();
  const failures = [];
  page.on('pageerror', error => failures.push(error.message));
  page.on('response', response => {
    if (response.url().startsWith(base + '/api/') && response.status() >= 400) failures.push(response.status() + ' ' + response.url());
  });
  const visited = [];
  try {
    for (const route of ['#/', '#/downloads', '#/logs', '#/settings', '#/add']) {
      await page.goto(base + '/' + route);
      await page.waitForFunction(() => document.querySelector('h1') && !document.body.innerText.includes('页面加载失败'));
      await page.waitForTimeout(300);
      visited.push({ route, heading: await page.locator('h1').innerText() });
    }
    const state = await page.evaluate(async () => (await fetch('/api/ui/state')).json());
    const jojo = state.rss_subscriptions.find(item => item.anime_name === '套马的汉子');
    if (!jojo) throw new Error('JOJO subscription not found');
    await page.goto(base + '/#/subscription/' + encodeURIComponent(jojo.url));
    await page.waitForSelector('#correctForm');
    if (await page.locator('#detailFilenameSeason').inputValue() !== '1') throw new Error('JOJO filename season changed');
    if (await page.locator('#detailDirectorySeason').inputValue() !== '6') throw new Error('JOJO directory season changed');
    visited.push({ route: 'JOJO details', heading: await page.locator('h1').innerText(), filenameSeason: 1, directorySeason: 6 });
    if (failures.length) throw new Error(failures.join('\n'));
    console.log(JSON.stringify({ browser: 'passed', subscriptions: state.rss_subscriptions.length, visited }, null, 2));
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error.message); process.exitCode = 1; });
