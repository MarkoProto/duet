// Opens one room in two Chrome windows and saves pictures of it. A scratch tool for looking at
// the room page without the lobby:  node room-look.mjs [wavelength|sketch|signal] [out-folder]
import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import { writeFileSync, mkdirSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

const kind = process.argv[2] || 'wavelength';
const out = process.argv[3] || '/tmp/duet-shots';
mkdirSync(out, { recursive: true });
const port = 3900 + Math.floor(Math.random() * 90);
const cfg = {
  room_id: 'r_look',
  people: [
    { key: 'look-key-for-ana-000000', name: 'Ana', color: '#ffb84d', lang: 'en' },
    { key: 'look-key-for-aoi-111111', name: 'Aoi', color: '#ff4f8b', lang: 'ja' },
  ],
  activity: { kind, title: { en: kind === 'sketch' ? 'Four Hands' : kind === 'signal' ? 'Signal' : 'Same Wavelength', ja: kind === 'sketch' ? '四つの手' : kind === 'signal' ? 'シグナル' : '以心伝心' }, data: {}, seed: 42 },
  host: {
    why: { en: 'You both picked music and late nights.', ja: '二人とも音楽と夜更かしを選びました。' },
    opener: { en: 'Start with the last song you had on repeat.', ja: '最近リピートした曲の話から。' },
    nudges: [{ en: 'Quiet? Ask what they would do with a free day.', ja: '静かですね。休みの日に何をするか聞いてみて。' }],
    last_minute: { en: 'One minute left. Say one thing you liked.', ja: 'あと1分。よかったことを一つ伝えて。' },
  },
  session_ms: Number(process.env.SESSION_MS || 600000), join_wait_ms: 90000, away_ms: 90000, nudge_after_ms: 75000,
  return_url: 'about:blank', runtime: { kind: 'local', id: 'r_look' }, long: false,
};
writeFileSync(`${out}/room.json`, JSON.stringify(cfg));
const server = spawn('../../target/debug/duet-room', ['--port', String(port), '--host', '127.0.0.1', '--secret', 'look-secret-0123456789', '--config', `${out}/room.json`], { stdio: 'inherit' });
await sleep(500);
const browser = await chromium.launch({ channel: 'chrome' });
const errors = [];
const open = async (key, viewport, scale = 1) => {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: scale, hasTouch: viewport.width < 500, isMobile: viewport.width < 500 });
  const page = await ctx.newPage();
  page.on('console', m => { if (m.type() === 'error') errors.push(m.text()); });
  page.on('pageerror', e => errors.push(String(e)));
  await page.goto(`http://127.0.0.1:${port}/#k=${key}`);
  return page;
};
try {
  const a = await open(cfg.people[0].key, { width: 1100, height: 680 });
  await sleep(700);
  await a.screenshot({ path: `${out}/${kind}-1-waiting.png` });
  const b = await open(cfg.people[1].key, { width: 390, height: 780 }, 2);
  await sleep(600);
  await a.screenshot({ path: `${out}/${kind}-2-hello.png` });
  await sleep(2600);
  await a.fill('#t', 'Hi Aoi! First time here');
  await a.press('#t', 'Enter');
  await b.fill('#t', 'こんにちは！よろしく');
  await b.press('#t', 'Enter');
  await a.fill('#t', '🔥');
  await a.press('#t', 'Enter');
  await sleep(300);
  if (kind === 'wavelength') {
    await a.fill('.wl .input', 'Ramen');
    await a.click('.wl .btn');
    await sleep(200);
    await a.screenshot({ path: `${out}/${kind}-3-locked.png` });
    await b.fill('.wl .input', 'ramen!');
    await b.click('.wl .btn');
    await sleep(900);
  } else if (kind === 'sketch') {
    const box = await a.locator('.sheetbox').boundingBox();
    await a.mouse.move(box.x + 80, box.y + 120); await a.mouse.down();
    for (let i = 0; i < 40; i++) await a.mouse.move(box.x + 80 + i * 9, box.y + 120 + Math.sin(i / 4) * 60);
    await a.mouse.up();
    const bb = await b.locator('.sheetbox').boundingBox();
    await b.touchscreen.tap(bb.x + 60, bb.y + 60);
    await sleep(500);
  } else if (kind === 'signal') {
    await sleep(300);
    const can = b.locator('.sg .cell.can');
    if (await can.count()) await can.first().click();
    await a.locator('.sg .cell').nth(7).click();
    await sleep(500);
  }
  await b.click('.emo >> nth=0');
  await sleep(350);
  await a.screenshot({ path: `${out}/${kind}-4-desktop.png` });
  await b.screenshot({ path: `${out}/${kind}-5-phone.png` });
  console.log('page errors:', errors.length ? errors : 'none');
} finally {
  await browser.close();
  server.kill();
}
