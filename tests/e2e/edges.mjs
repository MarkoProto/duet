// The harder cases, in real Chrome windows:
//   1. a browser that cannot open a WebSocket still chats, plays and hears the bell
//   2. a report made from the room's menu ends the session, and the lobby keeps that chat and only that
//   3. the room speaks the language the person switches to
//   node edges.mjs [folder for pictures]
import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, readdirSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const out = process.argv[2] || '/tmp/duet-shots';
mkdirSync(out, { recursive: true });
const port = 8800 + Math.floor(Math.random() * 90);
const base = `http://127.0.0.1:${port}`;
const data = mkdtempSync(join(tmpdir(), 'duet-edges-'));
const env = { ...process.env, DUET_BIND: `127.0.0.1:${port}`, DUET_PUBLIC_URL: base, DUET_DATA: data, DUET_ROOMS: 'local', DUET_SESSION_SECS: '300', DUET_VOTE_SECS: '40',
  DUET_SETTLE_SECS: '0.5', DUET_GATHER_SECS: '2', DUET_ACTIVITIES: 'wavelength,sketch,signal', DAYTONA_API_KEY: '', NOSANA_API_KEY: '', NOSANA_LLM_URL: '', HOME: data };
const server = spawn('../../target/debug/duet', [], { env, stdio: ['ignore', 'pipe', 'inherit'] });
server.stdout.on('data', () => {});
let passed = 0;
const failures = [], errors = [];
const check = (name, ok, detail = '') => { if (ok) { passed++; console.log('  PASS', name); } else { failures.push(name); console.log('  FAIL', name, detail); } };
const until = async (what, test, ms = 12000) => {
  const end = Date.now() + ms;
  for (;;) { try { if (await test()) return true; } catch {} if (Date.now() > end) { check(what, false, 'timed out'); return false; } await sleep(120); }
};

async function person(browser, name, modePick, { noSockets = false, lang = 'en' } = {}) {
  const ctx = await browser.newContext({ viewport: { width: 1000, height: 700 }, locale: lang === 'ja' ? 'ja-JP' : 'en-GB' });
  await ctx.addInitScript(() => { try { sessionStorage.setItem('duet-tut-off', '1'); } catch {} }); // these tests go straight to the game
  // A network that refuses sockets: every attempt closes at once, as some proxies make it.
  if (noSockets) await ctx.addInitScript(() => { window.WebSocket = class { constructor() { setTimeout(() => this.onclose && this.onclose({}), 20); } send() {} close() {} }; });
  const page = await ctx.newPage();
  page.on('pageerror', e => errors.push(`${name}: ${e}`));
  await page.goto(base);
  await page.waitForSelector('.screen .input');
  await page.fill('.screen .input', name);
  await page.click('.agree');
  await page.click('.screen .btn');
  for (const pick of [0, 1, modePick, 0, 2]) { await page.waitForSelector('.q .opt'); await sleep(250); await page.locator('.q .opt').nth(pick).click(); await sleep(320); }
  await page.waitForSelector('.q .grid .chip'); await sleep(250);
  await page.click('.q .btn');
  await page.waitForSelector('.q textarea');
  await page.locator('.q .link').click();
  return page;
}

try {
  await sleep(700);
  const browser = await chromium.launch({ channel: 'chrome' });

  console.log('1. No WebSocket');
  const ana = await person(browser, 'Ana', 1);                       // wants to play: Signal
  const ben = await person(browser, 'Ben', 1, { noSockets: true });
  await until('both reach the room', async () => ana.url().includes('/r/') && ben.url().includes('/r/'), 20000);
  await until('the room opens for the one without sockets', async () => !(await ben.locator('#app.gated').count()) && await ben.locator('#clock').isVisible(), 15000);
  check('the page switched to held requests by itself', await ben.evaluate(() => D.net.mode === 'poll' && D.net.online));
  check('the other page kept its socket', await ana.evaluate(() => D.net.mode === 'ws' && D.net.online));
  await ben.fill('#t', 'no sockets here'); await ben.press('#t', 'Enter');
  await until('a message from the held-request page arrives', () => ana.getByText('no sockets here').isVisible());
  await ana.fill('#t', 'and back again'); await ana.press('#t', 'Enter');
  await until('a message to the held-request page arrives', () => ben.getByText('and back again').isVisible());
  check('its own message is confirmed, not left pending', await ben.locator('.m.mine.pending, .m.mine.failed').count() === 0);
  check('two players get Signal', await ana.locator('.sg').count() === 1);
  // Level one: Ana guides, Ben walks. Ben reads the path out of Ana's screen (the test may cheat).
  const path = await ana.evaluate(() => D.signalPath(D.net.room.activity.seed >>> 0, 0));
  for (const [x, y] of path.path.slice(1)) { await ben.locator('.sg .cell').nth(y * path.n + x).click(); await sleep(260); }
  await until('a level walked over held requests is completed on both screens', async () => (await ana.locator('.sg .pill').first().textContent()).includes('2') && (await ben.locator('.sg .pill').first().textContent()).includes('2'));
  await ana.locator('.emo').nth(0).click();
  await until('a floating reaction crosses to the held-request page', () => ben.locator('#burst .float').first().isVisible(), 5000);
  await sleep(300);
  await ben.screenshot({ path: `${out}/edges-1-no-sockets.png` });

  console.log('2. The room in the other language');
  await ana.click('#more');
  await ana.locator('#menu .seg button', { hasText: '日本語' }).click();
  await until('the room reloads in Japanese with the chat intact', async () => (await ana.locator('#t').getAttribute('placeholder')).includes('メッセージ') && await ana.getByText('no sockets here').isVisible());
  check('the game survives the reload at level two', (await ana.locator('.sg .pill').first().textContent()).includes('2'));

  console.log('3. A report from the menu');
  await ben.fill('#t', 'something unkind'); await ben.press('#t', 'Enter');
  await until('the unkind message arrives', () => ana.getByText('something unkind').isVisible());
  await ana.click('#more');
  await ana.locator('#menu button.warn').click();
  check('the report cannot be sent without a reason', await ana.locator('#modalbox .btn.danger.off').count() === 1);
  await ana.screenshot({ path: `${out}/edges-2-report.png` });
  await ana.locator('#modalbox .chip').nth(0).click();
  await ana.fill('#modalbox textarea', 'see the last message');
  await ana.locator('#modalbox .btn.danger').click();
  await until('the session ends for the reporter', () => ana.locator('#done').isVisible());
  await until('and for the one reported, with a plain ending', async () => (await ben.locator('#donetext').textContent()).includes('The session ended'));
  await until('both are back in the lobby', async () => !ana.url().includes('/r/') && !ben.url().includes('/r/') && await ben.getByText('The room is gone').isVisible(), 20000);
  await sleep(800);
  const dir = join(data, 'reports');
  const files = existsSync(dir) ? readdirSync(dir) : [];
  check('exactly one report is kept', files.length === 1);
  const kept = files.length ? JSON.parse(readFileSync(join(dir, files[0]), 'utf8')) : {};
  check('it holds the chat of that session and the reason', JSON.stringify(kept).includes('something unkind') && kept.room?.report?.reason === 'rude' && kept.room?.report?.note === 'see the last message');
  check('the reported person is not told', !(await ben.evaluate(() => document.body.innerText)).toLowerCase().includes('report'));
  check('no errors in either browser', errors.length === 0, errors.join(' | '));
  await browser.close();
} catch (e) {
  failures.push('the run itself: ' + e.message.split('\n')[0]);
  console.log('  FAIL the run itself:', e.message.split('\n')[0]);
} finally {
  server.kill();
  rmSync(data, { recursive: true, force: true });
}
console.log(`\n${passed} passed, ${failures.length} failed` + (failures.length ? ':\n  - ' + failures.join('\n  - ') : ''));
process.exit(failures.length ? 1 : 0);
