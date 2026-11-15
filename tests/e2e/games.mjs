// The two games that both people touch, in two real Chrome windows, each against a room of its own:
// what one person does must never reset or pull away the other's screen.
//   node games.mjs [folder for pictures]
// Starts the room program alone (no lobby), so it needs no keys and touches nothing else.
import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import { mkdirSync, writeFileSync } from 'node:fs';
import { setTimeout as sleep } from 'node:timers/promises';

const out = process.argv[2] || '/tmp/duet-shots';
mkdirSync(out, { recursive: true });
let passed = 0;
const failures = [];
const check = (name, ok, detail = '') => { if (ok) { passed++; console.log('  PASS', name); } else { failures.push(name); console.log('  FAIL', name, detail); } };
const until = async (test, ms = 5000) => { const end = Date.now() + ms; for (;;) { try { if (await test()) return true; } catch {} if (Date.now() > end) return false; await sleep(80); } };

const people = [
  { key: 'games-key-for-ana-000000', name: 'Ana', color: '#ffb84d', lang: 'en' },
  { key: 'games-key-for-ben-111111', name: 'Ben', color: '#ff4f8b', lang: 'en' },
];
function room(kind, port, extra = {}) {
  const cfg = {
    room_id: 'r_games_' + kind, people,
    activity: { kind, title: { en: kind, ja: kind }, data: {}, seed: 7 },
    host: { why: { en: 'Test.', ja: 'テスト。' }, opener: { en: 'Hello.', ja: 'こんにちは。' }, nudges: [{ en: 'Hi.', ja: 'やあ。' }], last_minute: { en: 'Almost time.', ja: 'そろそろ。' } },
    session_ms: 600000, join_wait_ms: 90000, away_ms: 90000, nudge_after_ms: 75000,
    return_url: 'about:blank', runtime: { kind: 'local', id: 'r_games' }, long: false,
    ...extra,
  };
  writeFileSync(`${out}/games-${kind}.json`, JSON.stringify(cfg));
  return spawn('../../target/debug/duet-room', ['--port', String(port), '--host', '127.0.0.1', '--secret', 'games-secret-0123456789', '--config', `${out}/games-${kind}.json`], { stdio: 'inherit' });
}

const browser = await chromium.launch({ channel: 'chrome' });
const errors = [];
async function pair(port, { poll = false, before = [], tutorial = false } = {}) {
  const pages = [];
  for (const [i, p] of people.entries()) {
    const ctx = await browser.newContext({ viewport: { width: 1000, height: 720 } });
    if (!tutorial) await ctx.addInitScript(() => { try { sessionStorage.setItem('duet-tut-off', '1'); } catch {} });
    const page = await ctx.newPage();
    page.on('pageerror', e => errors.push(String(e)));
    if (before[i]) await before[i](page);
    await page.goto(`http://127.0.0.1:${port}/#k=${p.key}${poll ? '&p=1' : ''}`);
    pages.push(page);
  }
  for (const page of pages) await until(async () => !(await page.locator('#app.gated').count()) && await page.locator('#clock').isVisible(), 8000);
  return pages;
}
const servers = [];
try {
  console.log('Same Wavelength');
  const p1 = 7100 + Math.floor(Math.random() * 90); // p1 to p1 + 800 stays clear of the ports Chrome refuses (5060, 6566, 6665…)
  servers.push(room('wavelength', p1));
  await sleep(500);
  const [ana, ben] = await pair(p1);
  const noticesShown = await until(() => ana.locator('.hostcard').first().isVisible(), 3000);
  await ben.click('.wl .input');
  await ben.keyboard.type('pas');
  await ana.fill('.wl .input', 'Pasta');
  await ana.click('.wl .btn');
  await until(() => ben.getByText('Ana has locked in').isVisible());
  check('the other locking in leaves what is being typed, and the cursor, alone', await ben.inputValue('.wl .input') === 'pas' && await ben.evaluate(() => document.activeElement?.classList.contains('input')));
  check('a locked answer is final: its button is off', await ana.locator('.wl .btn').isDisabled());
  await ben.keyboard.type('ta');
  await ben.click('.wl .btn');
  await until(() => ana.locator('.wl .ans').first().isVisible());
  await until(() => ben.locator('.wl .ans').first().isVisible());
  await ana.click('.wl .btn');
  await until(() => ana.getByText('Waiting for Ben').isVisible());
  await sleep(500);
  check('one person pressing Next does not move the other on', await ben.locator('.wl .ans').count() === 2 && await ben.getByText('Ana is ready for the next one').isVisible());
  check('the one who pressed waits, and cannot press twice', await ana.locator('.wl .btn').isDisabled());
  await ben.click('.wl .btn');
  check('both pressing Next moves both on together', await until(async () => (await ana.locator('.wl .pill').first().textContent()).startsWith('2') && (await ben.locator('.wl .pill').first().textContent()).startsWith('2')));
  check('the next prompt starts empty for both', await ana.inputValue('.wl .input') === '' && await ben.inputValue('.wl .input') === '');
  const verdicts = await ana.evaluate(() => [['cats', 'bats'], ['アメリカ', 'アフリカ'], ['ねこ', 'ネコ'], ['Pasta', ' pasta!'], ['spaghetti', 'spagetti']].map(([a, b]) => D.wavelengthVerdict(a, b)));
  check('in sync means the same answer: cats/bats differ, ねこ/ネコ match, a typo in a long word is forgiven', JSON.stringify(verdicts) === JSON.stringify(['diff', 'diff', 'sync', 'sync', 'sync']), JSON.stringify(verdicts));
  // Signs, as in the watch-party app: a laugh or a face alone becomes a moving picture.
  await ana.fill('#t', 'haha');
  await ana.keyboard.press('Enter');
  check('"haha" on its own becomes a moving laugh, on both screens', await until(async () => await ana.locator('#log .m.sign .signart.haha').count() === 1 && await ben.locator('#log .m.sign .signart.haha').count() === 1));
  await ben.fill('#t', 'haha that was fun');
  await ben.keyboard.press('Enter');
  await until(() => ana.getByText('haha that was fun').isVisible());
  await ben.fill('#t', '<3');
  await ben.keyboard.press('Enter');
  check('inside a sentence it stays plain text; <3 alone becomes a heart', await until(async () => await ana.locator('#log .signart.heart').count() === 1) && await ana.locator('#log .m.sign').count() === 2);
  check('the picture settles small and still after a few seconds', await until(async () => await ana.locator('#log .signart.haha.settled').count() === 1, 6000));
  await ana.screenshot({ path: `${out}/games-wavelength-ana.png` });
  check('the host\'s notes and the "is here" lines clear themselves after a few seconds', noticesShown && await until(async () => await ana.locator('#log .hostcard, #log .sysline').count() === 0 && await ben.locator('#log .hostcard, #log .sysline').count() === 0, 12000));

  console.log('Four Hands');
  const p2 = p1 + 100;
  servers.push(room('sketch', p2));
  await sleep(500);
  const [a, b] = await pair(p2);
  const turn = page => page.locator('.sk .turn').textContent();
  const line = async (page, from, to) => {
    const box = await page.locator('.sk .sheetbox').boundingBox();
    const at = ([x, y]) => [box.x + box.width * x, box.y + box.height * y];
    await page.mouse.move(...at(from));
    await page.mouse.down();
    for (let i = 1; i <= 8; i++) await page.mouse.move(...at([from[0] + (to[0] - from[0]) * i / 8, from[1] + (to[1] - from[1]) * i / 8]));
    await page.mouse.up();
  };
  check('at the start either may draw', (await turn(a)).includes('Either') && (await turn(b)).includes('Either'));
  await line(a, [0.2, 0.3], [0.6, 0.3]);
  check('after Ana\'s line it is Ben\'s turn, on both screens', await until(async () => (await turn(b)).startsWith('Your turn') && (await turn(a)).includes("Ben's turn")));
  await sleep(600); // her own line comes back from the room and is drawn on the sheet
  const before = await a.locator('.sk canvas').first().evaluate(c => c.toDataURL());
  await line(a, [0.2, 0.6], [0.6, 0.6]);
  await sleep(400);
  const after = await a.locator('.sk canvas').first().evaluate(c => c.toDataURL());
  check('Ana cannot draw a second line in a row', before === after && (await turn(b)).startsWith('Your turn'));
  await line(b, [0.3, 0.7], [0.7, 0.8]);
  check('after Ben\'s line it is Ana\'s turn again', await until(async () => (await turn(a)).startsWith('Your turn') && (await turn(b)).includes("Ana's turn")));
  await line(a, [0.1, 0.1], [0.3, 0.2]);
  await until(async () => (await turn(b)).startsWith('Your turn'));
  await a.getByText('Undo').click();
  check('undoing her own last line gives Ana the turn back', await until(async () => (await turn(a)).startsWith('Your turn') && (await turn(b)).includes("Ana's turn")));
  const painted = page => page.locator('.sk canvas').first().evaluate(c => { const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; let n = 0; for (let i = 3; i < d.length; i += 4) if (d[i]) n++; return n; });
  const [paintedA, paintedB] = [await painted(a), await painted(b)];
  const ring = Array.from({ length: 29 }, (_, i) => [0.8 + 0.08 * Math.cos(i / 28 * 2 * Math.PI), 0.3 + 0.1 * Math.sin(i / 28 * 2 * Math.PI)]);
  const loop = async (page, pts) => {
    const box = await page.locator('.sk .sheetbox').boundingBox();
    const at = ([x, y]) => [box.x + box.width * x, box.y + box.height * y];
    await page.mouse.move(...at(pts[0])); await page.mouse.down();
    for (const p of pts.slice(1)) await page.mouse.move(...at(p), { steps: 2 });
    await page.mouse.up();
  };
  await loop(a, ring);
  await until(async () => (await turn(b)).startsWith('Your turn'));
  await sleep(500);
  check('a circle, a line that ends where it began, is drawn on both screens', await painted(a) > paintedA + 200 && await painted(b) > paintedB + 200, `${paintedA} -> ${await painted(a)}, ${paintedB} -> ${await painted(b)}`);
  check('a closed loop keeps its shape when it is made lighter to send', await a.evaluate(() => D.sketchThin([0, 0, 100, 0, 100, 100, 0, 100, 0, 0], 10).length) >= 8);
  await a.screenshot({ path: `${out}/games-sketch-ana.png` });

  console.log('This or That');
  const p3 = p2 + 100;
  servers.push(room('thisorthat', p3));
  await sleep(500);
  const [c, d] = await pair(p3);
  await c.locator('.tt .opt').first().click();
  await until(() => d.getByText('Ana has picked').isVisible());
  check('one person picking does not show their choice to the other', await d.locator('.tt .opt.now').count() === 0 && await d.locator('.tt .ans').count() === 0);
  await c.locator('.tt .opt').nth(1).click();
  check('a pick can be changed until both are in', await c.locator('.tt .opt.now').textContent() === await c.locator('.tt .opt').nth(1).textContent());
  await d.locator('.tt .opt').nth(1).click();
  check('both picks are shown together, and a match is a match', await until(async () => await c.locator('.tt .ans.sync').count() === 2 && await d.locator('.tt .ans.sync').count() === 2));
  await c.setViewportSize({ width: 390, height: 844 });
  await sleep(400);
  const nextBox = await c.locator('.tt .btn').boundingBox(), barBox = await c.locator('#bottom').boundingBox();
  check('on a phone the whole game fits: Next sits above the reactions bar', !!nextBox && !!barBox && nextBox.y + nextBox.height <= barBox.y + 1, JSON.stringify([nextBox, barBox]));
  await c.setViewportSize({ width: 1000, height: 720 });
  await sleep(300);
  await c.locator('.tt .btn').click();
  await sleep(400);
  check('Next waits for both here too', await d.locator('.tt .ans').count() === 2);
  await d.locator('.tt .btn').click();
  check('both move on together', await until(async () => (await c.locator('.tt .pill').first().textContent()).startsWith('2') && (await d.locator('.tt .pill').first().textContent()).startsWith('2')));
  await c.locator('.tt .opt').first().click();
  await d.locator('.tt .opt').nth(1).click();
  await until(async () => await c.locator('.tt .ans').count() === 2 && await d.locator('.tt .ans').count() === 2);
  check('a different pick is shown as different', await c.locator('.tt .ans.sync').count() === 0);
  const next = await c.locator('.tt .btn').boundingBox();
  await c.mouse.move(next.x + next.width / 2, next.y + next.height / 2);
  await c.mouse.down();
  await d.locator('.tt .btn').click();
  await until(() => c.getByText('Ben is ready for the next one').isVisible());
  await c.mouse.up();
  check('a Next being pressed is not lost when the other presses theirs', await until(async () => (await c.locator('.tt .pill').first().textContent()).startsWith('3') && (await d.locator('.tt .pill').first().textContent()).startsWith('3')));
  await c.screenshot({ path: `${out}/games-thisorthat-ana.png` });

  console.log('Tale for Two');
  const p4 = p3 + 100;
  servers.push(room('story', p4));
  await sleep(500);
  const [e, f] = await pair(p4);
  const turnOf = page => page.locator('.st .turn').textContent();
  check('the story starts with its first line, and either may go first', await e.locator('.st .line.host').count() === 1 && (await turnOf(e)).includes('Either'));
  await f.click('.st .input');
  await f.keyboard.type('Ben is still typ');
  await e.fill('.st .input', 'Ana looked at the clock.');
  await e.click('.st .btn');
  await until(async () => (await f.locator('.st .line:not(.host)').count()) === 1);
  check('the other\'s sentence arriving leaves what is being typed alone', await f.inputValue('.st .input') === 'Ben is still typ' && await f.evaluate(() => document.activeElement?.classList.contains('input')));
  check('then it is Ben\'s turn, on both screens', (await turnOf(f)).startsWith('Your turn') && (await turnOf(e)).includes("Ben's turn"));
  await e.fill('.st .input', 'Ana tries again.');
  await e.click('.st .btn', { force: true });
  await sleep(500);
  check('Ana cannot add two sentences in a row', await f.locator('.st .line:not(.host):not(.twist)').count() === 1);
  await f.keyboard.press('Meta+A');
  await f.keyboard.type('Ben heard footsteps.');
  await f.keyboard.press('Enter');
  await until(async () => (await e.locator('.st .line:not(.host)').count()) === 2);
  await e.fill('.st .input', 'They both turned around.');
  await e.click('.st .btn');
  check('a twist drops in after three sentences, on both screens', await until(async () => await e.locator('.st .line.twist').count() === 1 && await f.locator('.st .line.twist').count() === 1));
  await e.screenshot({ path: `${out}/games-story-ana.png` });

  console.log('Four Hands, the other page closing mid-line');
  const p5 = p4 + 100;
  servers.push(room('sketch', p5));
  await sleep(500);
  const [g, k] = await pair(p5);
  const turnG = () => g.locator('.sk .turn').textContent();
  const sheet = await k.locator('.sk .sheetbox').boundingBox();
  await k.mouse.move(sheet.x + sheet.width * 0.2, sheet.y + sheet.height * 0.5);
  await k.mouse.down();
  for (let i = 1; i <= 5; i++) await k.mouse.move(sheet.x + sheet.width * (0.2 + i * 0.05), sheet.y + sheet.height * 0.5);
  await until(async () => (await turnG()).includes("Ben's turn"), 3000);
  await k.reload(); // the pen never lifted
  check('a line left unfinished by a closed page does not keep the turn for ever', await until(async () => !(await turnG()).includes("Ben's turn"), 8000), await turnG());

  console.log('Held requests (the Daytona path): a send that fails is tried again');
  const p6 = p5 + 100;
  servers.push(room('story', p6));
  await sleep(500);
  let dropped = 0;
  const [m, q] = await pair(p6, {
    poll: true,
    before: [page => page.route('**/send?**', route => (dropped === 0 && (route.request().postData() || '').includes('"line"') ? (dropped++, route.abort()) : route.continue()))],
  });
  await m.fill('.st .input', 'This one goes through in the end.');
  await m.click('.st .btn');
  const reached = await until(async () => (await q.locator('.st .line:not(.host)').count()) === 1, 8000);
  check('a sentence whose first send failed still reaches the other', dropped === 1 && reached, `dropped ${dropped}, reached ${reached}, Ana sees ${await m.locator('.st .line:not(.host)').count()} line(s), mode ${await m.evaluate(() => D.net.mode)}`);

  console.log('Held requests: a room that does not answer at first');
  const p7 = p6 + 100;
  servers.push(room('wavelength', p7));
  await sleep(500);
  let refused = 0;
  const [r, r2] = await pair(p7, {
    poll: true,
    before: [page => page.route('**/hello?**', route => (refused < 4 ? (refused++, route.fulfill({ status: 502, body: 'down' })) : route.continue()))],
  });
  check('the room opens once it answers, instead of calling the link bad', refused === 4 && await r.locator('#done').isHidden() && await r.locator('#clock').isVisible());

  console.log('Held requests: the room removed after the session');
  await r.route('**/poll?**', route => route.fulfill({ status: 404, body: 'gone' }));
  await r.route('**/send?**', route => route.fulfill({ status: 404, body: 'gone' }));
  // Something happens in the room, so Ana's held request comes back and the next one finds no room.
  await r2.fill('.wl .input', 'Tea');
  await r2.click('.wl .btn');
  check('a page whose room is gone goes back instead of waiting for ever', await until(() => r.locator('#done').isVisible(), 8000));

  console.log('Changing the game');
  const p8 = p7 + 100;
  const alt = (kind, en) => ({ kind, title: { en, ja: en }, data: {}, seed: 3 });
  servers.push(room('wavelength', p8, { alternatives: [alt('thisorthat', 'This or That'), alt('story', 'Tale for Two'), alt('signal', 'Signal')] }));
  await sleep(500);
  const [u, v] = await pair(p8);
  const suggest = async (page, game) => {
    await page.click('#more');
    await page.getByText('Change game').click();
    await page.locator('#modalbox .game', { hasText: game }).click();
  };
  await suggest(u, 'This or That');
  await until(() => v.getByText('Ana would like to switch to This or That').isVisible());
  check('one person suggests a game; the other is asked, and nothing changes yet', await v.locator('.wl').count() === 1 && await u.locator('.wl').count() === 1);
  await v.locator('#modalbox .btn', { hasText: 'Switch' }).click();
  check('when the other agrees, both screens switch together', await until(async () => await u.locator('.tt').count() === 1 && await v.locator('.tt').count() === 1));
  await suggest(u, 'Tale for Two');
  await until(() => v.getByText('Ana would like to switch to Tale for Two').isVisible());
  await v.locator('#modalbox .btn', { hasText: 'Keep this one' }).click();
  check('when the other says no, both keep playing, and the one who asked is told', await until(() => u.getByText('Ben would rather keep playing this one').isVisible()) && await u.locator('.tt').count() === 1 && await v.locator('.tt').count() === 1);
  await u.click('#more');
  await u.getByText('Change game').click();
  const offered = await u.locator('#modalbox .game b').allTextContents();
  check('a game already played is not offered again', !offered.includes('This or That') && !offered.includes('Same Wavelength') && offered.includes('Signal'), offered.join(', '));
  await u.locator('#modalbox .btn', { hasText: 'Cancel' }).click();
  await v.reload();
  check('after a reload the new game is still the one on screen', await until(async () => await v.locator('.tt').count() === 1, 8000));

  console.log('How to play');
  const p9 = p8 + 100;
  servers.push(room('story', p9, { activity: { kind: 'story', title: { en: 'Tale for Two', ja: '二人の物語' }, data: {}, seed: 1 }, alternatives: [alt('thisorthat', 'This or That')] }));
  await sleep(500);
  const [w1, w2] = await pair(p9, { tutorial: true });
  const tut = page => page.locator('#act .tut');
  check('before the game, a short how-to shows its first screen', await until(() => tut(w1).isVisible()) && (await tut(w1).textContent()).includes('How to play') && (await tut(w1).textContent()).includes('Tale for Two') && (await tut(w1).textContent()).includes('one sentence each'));
  await w1.locator('#act .tut .btn').click();
  check('Next shows the second screen, and the button now says Start', (await tut(w1).textContent()).includes('twist') && (await w1.locator('#act .tut .btn').textContent()).includes('Start'));
  await w1.locator('#act .tut .btn').click();
  check('Start opens the game; the other still reads at their own pace', await until(async () => await tut(w1).count() === 0) && await tut(w2).isVisible());
  await w1.reload();
  await until(() => w1.locator('#clock').isVisible(), 8000);
  await sleep(600);
  check('after a reload the how-to is not shown again', await tut(w1).count() === 0);
  await w2.locator('#act .tut .btn').click();
  await w2.locator('#act .tut .btn').click();
  await w1.click('#more');
  await w1.getByText('Change game').click();
  await w1.locator('#modalbox .game', { hasText: 'This or That' }).click();
  await until(() => w2.getByText('Ana would like to switch to This or That').isVisible());
  await w2.locator('#modalbox .btn', { hasText: 'Switch' }).click();
  check('a game switched to gets its own how-to', await until(async () => (await tut(w1).count()) === 1 && (await tut(w1).textContent()).includes('Two options')));
  await sleep(800);
  check('the how-to sits over the game, so a tap reaches the card, not the game', await w1.evaluate(() => { const r = document.getElementById('act').getBoundingClientRect(); return !!document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2)?.closest('.tut'); }));
  await w1.screenshot({ path: `${out}/games-tutorial-ana.png` });

  console.log('Four Hands over held requests: the other line grows smoothly');
  const p10 = p9 + 100;
  servers.push(room('sketch', p10));
  await sleep(500);
  // Ben's updates are held back a third of a second each, as through the lobby to Daytona, so the
  // points reach him in bursts of several at a time.
  const [ana2, ben2] = await pair(p10, { poll: true, before: [null, page => page.route('**/poll?**', async route => { try { const r = await route.fetch(); await sleep(350); await route.fulfill({ response: r }); } catch {} })] });
  // Film Ben's sheet: how much of it is drawn, at every frame.
  await ben2.evaluate(() => {
    window.film = [];
    const [base, live] = document.querySelectorAll('.sk canvas');
    const count = c => { const d = c.getContext('2d', { willReadFrequently: true }).getImageData(0, 0, c.width, c.height).data; let n = 0; for (let i = 3; i < d.length; i += 16) if (d[i]) n++; return n; };
    const frame = () => { window.film.push([performance.now(), count(base) + count(live)]); if (window.film.length < 600) requestAnimationFrame(frame); };
    requestAnimationFrame(frame);
  });
  const box2 = await ana2.locator('.sk .sheetbox').boundingBox();
  const at2 = (x, y) => [box2.x + box2.width * x, box2.y + box2.height * y];
  await ana2.mouse.move(...at2(0.1, 0.5));
  await ana2.mouse.down();
  for (let i = 1; i <= 40; i++) { await ana2.mouse.move(...at2(0.1 + 0.8 * i / 40, 0.5 + 0.3 * Math.sin(i / 6))); await sleep(30); }
  await ana2.mouse.up();
  const released = await ben2.evaluate(() => performance.now());
  await until(async () => (await ben2.locator('.sk .turn').textContent()).startsWith('Your turn'), 8000);
  await sleep(2500);
  const film = await ben2.evaluate(() => window.film);
  const full = film[film.length - 1][1];
  let biggest = 0, growing = 0, doneAt = 0;
  for (let i = 1; i < film.length; i++) {
    const step = film[i][1] - film[i - 1][1];
    if (step > 0) { growing++; biggest = Math.max(biggest, step); }
    if (!doneAt && film[i][1] >= full * 0.98) doneAt = film[i][0];
  }
  console.log(`  (largest step ${Math.round(100 * biggest / full)}% of the line, grew over ${growing} frames, all there ${Math.round(doneAt - released)} ms after the pen lifted)`);
  check('the other person\'s line grows smoothly, even when it arrives in bursts', full > 0 && biggest / full < 0.12 && growing >= 20, `largest step ${Math.round(100 * biggest / full)}% of the line, grew in ${growing} frames`);
  check('and it is all there soon after the pen lifts', doneAt > 0 && doneAt - released < 2500, `${Math.round(doneAt - released)} ms after`);
  check('the finished line stays on the sheet', await ben2.locator('.sk canvas').first().evaluate(c => { const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data; for (let i = 3; i < d.length; i += 4) if (d[i]) return true; return false; }));
  await ben2.unrouteAll({ behavior: 'ignoreErrors' });

  check('no errors in any window', errors.length === 0, errors.join(' | '));
} catch (e) {
  failures.push('the run itself: ' + e.message);
  console.log('  FAIL the run itself:', e.message.split('\n')[0]);
} finally {
  await browser.close().catch(() => {});
  for (const s of servers) s.kill();
}
console.log(`\n${passed} passed, ${failures.length} failed` + (failures.length ? ':\n  - ' + failures.join('\n  - ') : ''));
process.exit(failures.length ? 1 : 0);
