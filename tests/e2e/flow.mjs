// Two people through the whole of Duet, in two real Chrome windows: a desktop and a phone.
//   node flow.mjs [folder for pictures]
// It starts its own lobby with short clocks and local rooms, so it needs no keys and touches nothing else.
//   REAL=daytona node flow.mjs   the same flow with each room in a real Daytona sandbox
//                                (key from DAYTONA_API_KEY or ~/Secrets/Daytona/key; every sandbox is deleted)
import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import { createWriteStream, writeFileSync, mkdirSync, mkdtempSync, rmSync, readdirSync, readFileSync, existsSync } from 'node:fs';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const out = process.argv[2] || '/tmp/duet-shots';
mkdirSync(out, { recursive: true });
const port = 8900 + Math.floor(Math.random() * 90);
const base = `http://127.0.0.1:${port}`;
const data = mkdtempSync(join(tmpdir(), 'duet-e2e-'));
const real = process.env.REAL === 'daytona';
const daytonaKey = !real ? '' : (process.env.DAYTONA_API_KEY || readFileSync(join(homedir(), 'Secrets/Daytona/key'), 'utf8')).trim();
const env = { ...process.env, DUET_BIND: `127.0.0.1:${port}`, DUET_PUBLIC_URL: base, DUET_DATA: data, DUET_ROOMS: real ? 'daytona' : 'local', DUET_ROOM_BIN_DIR: resolve('../../dist'),
  DUET_SESSION_SECS: process.env.SESSION || '16', DUET_LONG_SESSION_SECS: '12', DUET_REMATCH_AFTER_SECS: '14', DUET_REMATCH_EARLY_SECS: '2', DUET_VOTE_SECS: '40', DUET_SETTLE_SECS: '0.8', DUET_GATHER_SECS: '3', DUET_ACTIVITIES: 'wavelength,sketch,signal',
  DAYTONA_API_KEY: daytonaKey, NOSANA_API_KEY: '', NOSANA_LLM_URL: '', HOME: data };
const server = spawn('../../target/debug/duet', [], { env, stdio: ['ignore', 'pipe', 'inherit'] });
if (real) server.stdout.pipe(createWriteStream(join(out, 'lobby.log'))); else server.stdout.on('data', () => {});
let passed = 0;
const failures = [];
const check = (name, ok, detail = '') => { if (ok) { passed++; console.log('  PASS', name); } else { failures.push(name); console.log('  FAIL', name, detail); } };
const errors = [];
// With REAL=daytona, what each browser saw on the way, written to lobby-trace.log in the pictures folder.
const trace = [];

async function person(browser, name, lang, viewport, scale) {
  const ctx = await browser.newContext({ viewport, deviceScaleFactor: scale, hasTouch: viewport.width < 500, isMobile: viewport.width < 500, locale: lang === 'ja' ? 'ja-JP' : 'en-GB' });
  await ctx.addInitScript(() => { try { sessionStorage.setItem('duet-tut-off', '1'); } catch {} }); // these tests go straight to the game
  const page = await ctx.newPage();
  page.on('console', m => { if (m.type() === 'error' && !/favicon|Failed to load resource/.test(m.text())) errors.push(`${name}: ${m.text()}`); });
  page.on('pageerror', e => errors.push(`${name}: ${e}`));
  if (real) {
    const at = () => new Date().toISOString().slice(11, 23);
    const short = u => u.replace(/#.*/, '#…').replace(/([?&]k=)[^&]+/, '$1…');
    page.on('console', m => trace.push(`${at()} ${name} console.${m.type()}: ${m.text()}`));
    page.on('framenavigated', f => { if (f === page.mainFrame()) trace.push(`${at()} ${name} page ${short(f.url())}`); });
    page.on('requestfailed', r => trace.push(`${at()} ${name} FAILED ${r.method()} ${short(r.url())}: ${r.failure()?.errorText}`));
    page.on('response', r => { if (r.status() >= 300 || /daytonaproxy/.test(r.url())) trace.push(`${at()} ${name} ${r.status()} ${r.request().method()} ${short(r.url())}`); });
    page.on('websocket', ws => {
      trace.push(`${at()} ${name} socket open ${short(ws.url())}`);
      let got = 0, sent = 0;
      ws.on('framereceived', f => { if (got++ < 4) trace.push(`${at()} ${name} socket got ${String(f.payload).slice(0, 90)}`); });
      ws.on('framesent', f => { if (sent++ < 4) trace.push(`${at()} ${name} socket sent ${String(f.payload).slice(0, 90)}`); });
      ws.on('socketerror', e => trace.push(`${at()} ${name} socket error ${e}`));
      ws.on('close', () => trace.push(`${at()} ${name} socket closed`));
    });
  }
  return { page, name, lang, shot: tag => page.screenshot({ path: `${out}/flow-${tag}-${name.toLowerCase()}.png` }) };
}
const screenOf = p => p.page.evaluate(() => document.querySelector('#view .screen:not(.leave) h2, #view .screen:not(.leave) h1')?.textContent || '');
const until = async (p, test, ms = 15000, what = '') => {
  const end = Date.now() + ms;
  for (;;) {
    try { if (await test()) return true; } catch {}
    if (Date.now() > end) { check(`${p.name}: ${what}`, false, 'timed out; screen says: ' + await screenOf(p).catch(() => '?')); return false; }
    await sleep(120);
  }
};
// In a room: on the lobby's own /r/ address, or on the room's own Daytona address.
const inRoom = p => { const u = p.page.url(); return u.includes('/r/') || !u.startsWith(base); };
const has = (p, text) => p.page.getByText(text, { exact: false }).first().isVisible();

async function signUp(p, picks) {
  await p.page.goto(base);
  await p.page.waitForSelector('.screen .input');
  await sleep(p === first ? 2300 : 300); // the first visitor watches the intro
  if (p.lang === 'ja') { await p.page.locator('#langs button[data-lang=ja]').click(); await p.page.waitForSelector('.screen .input'); await sleep(300); }
  await p.page.fill('.screen .input', p.name);
  await p.page.click('.agree');
  await p.page.click('.screen .btn');
  for (const pick of picks) {
    await p.page.waitForSelector('.q .opt');
    await sleep(260);
    await p.page.locator('.q .opt').nth(pick).click();
    await sleep(330);
  }
  await p.page.waitForSelector('.q .grid .chip');
  await sleep(250);
  return p;
}
let first;

try {
  // Wait for the lobby to answer: with Daytona it first picks an API and looks for leftover sandboxes.
  for (let i = 0; ; i++) {
    try { if ((await fetch(base + '/api/state?v=0')).ok) break; } catch {}
    if (i > 300) throw new Error('the lobby did not start in 30 seconds');
    await sleep(100);
  }
  const browser = await chromium.launch({ channel: 'chrome' });
  const ana = first = await person(browser, 'Ana', 'en', { width: 1100, height: 720 }, 1);
  const aoi = await person(browser, 'Aoi', 'ja', { width: 390, height: 800 }, 2);

  console.log('1. The front page');
  await ana.page.goto(base);
  await sleep(2500);
  await ana.shot('01-hello');
  check('the front page says what it is', await has(ana, 'with someone new'));
  check('Start is held back until there is a name and the 18+ tick', await ana.page.locator('.screen .btn.off').count() === 1);
  check('the page people use shows no technical words', !/sandbox|sponsor|daytona|nosana|\bms\b|local room|destroy/i.test(await ana.page.evaluate(() => document.body.innerText)));
  check('the page opens in English', await ana.page.evaluate(() => document.documentElement.lang === 'en'));
  check('buttons carry line icons, not emoji', await ana.page.evaluate(() => !/\p{Extended_Pictographic}/u.test(document.querySelector('#view').innerText) && document.querySelectorAll('#view .btn svg.ic').length === 1));
  const sizes = await ana.page.evaluate(() => performance.getEntriesByType('resource').length + '|' + performance.getEntriesByType('navigation')[0].transferSize);
  check('the page arrives as one small request, plus fonts and three photos', Number(sizes.split('|')[0]) <= 8 && Number(sizes.split('|')[1]) < 40000, sizes);

  console.log('2. Questions');
  await signUp(ana, [2, 0, 2, 0, 1]); // both, calm, talk, team, talker
  // Someone else arriving changes the lobby; it must not throw away the picks Ana has not sent yet.
  const kai = await person(browser, 'Kai', 'en', { width: 390, height: 800 }, 1);
  await kai.page.goto(base);
  await kai.page.waitForSelector('.screen .input');
  await kai.page.fill('.screen .input', 'Kai');
  await kai.page.click('.agree');
  await kai.page.click('.screen .btn');
  await kai.page.waitForSelector('.q .opt');
  await sleep(1500);
  check('someone else joining does not reset the questions being answered', await ana.page.locator('.q .grid .chip').count() > 0 && await ana.page.locator('.q .opt').count() === 0);
  await kai.page.evaluate(() => fetch('/api/forget', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' }));
  await kai.page.context().close();
  await ana.shot('02-interests');
  for (const i of [0, 3, 5]) await ana.page.locator('.q .chip').nth(i).click();
  await ana.page.locator('.q .chip').nth(7).click(); // a fourth is refused
  check('at most three interests can be picked', await ana.page.locator('.q .chip.now').count() === 3);
  await ana.page.click('.q .btn');
  await ana.page.waitForSelector('.q textarea');
  await ana.page.fill('.q textarea', 'A stranger held the door and we both laughed.');
  await ana.shot('03-smile');
  await ana.page.click('.q .btn');
  await until(ana, () => has(ana, 'Finding your person'), 8000, 'reaches the waiting screen');
  await sleep(600);
  await ana.shot('04-waiting');
  check('someone waiting alone is told so', await has(ana, 'first one here'));

  // Aoi says it in her own words instead of tapping (read by the rules here: no model is set).
  await aoi.page.goto(base);
  await aoi.page.waitForSelector('.screen .input');
  await sleep(300);
  await aoi.page.locator('#langs button[data-lang=ja]').click();
  await aoi.page.waitForSelector('.screen .input');
  await sleep(300);
  await aoi.page.fill('.screen .input', 'Aoi');
  await aoi.page.click('.agree');
  await aoi.page.click('.screen .btn');
  await aoi.page.waitForSelector('.q .wordslink');
  await sleep(300);
  await aoi.page.click('.q .wordslink');
  await aoi.page.waitForSelector('.q textarea');
  const aoiNote = '今日は落ち着いた気分。一緒に協力するのが好き。ゆっくり話したい。聞き役です。音楽と料理が好き。';
  await aoi.page.fill('.q textarea', aoiNote);
  await aoi.page.click('.q .btn');
  await until(aoi, async () => await aoi.page.locator('.heard .chip').count() >= 6, 8000, 'sees the taps read from her words');
  await sleep(700);
  await aoi.shot('02-own-words');
  const heard = await aoi.page.locator('.heard').innerText();
  check('own words become the taps: language, mood, what to do, style, role, interests', ['日本語', '落ち着いている', 'ただ話す', '協力したい', '聞き役', '音楽', '食べること'].every(w => heard.includes(w)), heard);
  check('the page reading the words shows no technical words', !/sandbox|sponsor|daytona|nosana|model|\bAI\b|\bms\b/i.test(await aoi.page.evaluate(() => document.body.innerText)));
  await aoi.page.click('.heard .btn');
  check('one tap switches the page to Japanese', await aoi.page.evaluate(() => document.documentElement.lang === 'ja' && document.querySelector('#langs .now').dataset.lang === 'ja'));

  console.log('3. Paired, and the room is built');
  await until(ana, () => has(ana, 'You and Aoi'), 12000, 'is paired with Aoi');
  await sleep(600);
  await ana.shot('05-build');
  await aoi.shot('05-build');
  check('the reason for the pair is shown', await has(ana, 'You both picked music and food'));
  await until(ana, () => inRoom(ana), 12000, 'is taken into the room');
  await until(aoi, () => inRoom(aoi), 12000, 'is taken into the room');
  check('people stay on Duet\'s own address, even when the room runs in Daytona', ana.page.url().startsWith(base + '/r/') && aoi.page.url().startsWith(base + '/r/'), ana.page.url());
  await until(ana, async () => !ana.page.url().includes('k='), 6000, 'the key leaves the address bar');
  check('the key leaves the address bar', !ana.page.url().includes('k='));
  const roomAddress = ana.page.url();
  await until(ana, async () => !(await ana.page.locator('#app.gated').count()) && await ana.page.locator('#clock').isVisible(), 8000, 'the room opens with the clock running');
  await sleep(1500);

  console.log('4. Inside the room');
  check('two talkers-and-listeners get Same Wavelength', await ana.page.locator('.wl').count() === 1);
  await ana.page.fill('#t', 'Hello Aoi!');
  await ana.page.press('#t', 'Enter');
  await aoi.page.fill('#t', 'こんにちは、Anaさん');
  await aoi.page.click('#send');
  await until(ana, () => has(ana, 'こんにちは、Anaさん'), 5000, 'sees the reply');
  check('the phone sees the first message', await has(aoi, 'Hello Aoi!'));
  await aoi.page.click('.wl .input');
  await aoi.page.keyboard.type('ram');
  await ana.page.fill('.wl .input', 'Ramen');
  await ana.page.click('.wl .btn');
  await until(aoi, () => has(aoi, '決定しました'), 3000, 'hears that Ana locked in');
  await sleep(600);
  check('the other locking in leaves what is being typed alone', await aoi.page.inputValue('.wl .input') === 'ram' && await aoi.page.evaluate(() => document.activeElement?.classList.contains('input')), await aoi.page.inputValue('.wl .input'));
  await aoi.page.keyboard.type('en');
  await aoi.page.click('.wl .btn');
  await until(ana, () => ana.page.locator('.wl .ans.sync').first().isVisible(), 5000, 'both answers are revealed as a match');
  await ana.page.locator('.emo').nth(4).click();
  await sleep(500);
  await ana.shot('06-room');
  await aoi.shot('06-room');
  const hood = await browser.newPage({ viewport: { width: 1200, height: 760 } });
  await hood.goto(base + '/hood');
  await sleep(1400);
  await hood.screenshot({ path: `${out}/flow-07-hood-live.png` });
  const adminText = await hood.evaluate(() => document.body.innerText);
  check('the admin page shows the open room and no names', adminText.includes('Same Wavelength') && !adminText.includes('Ana'));
  check('the admin page says plainly what runs the rooms and the pairing', adminText.includes(real ? 'Rooms run in Daytona sandboxes' : 'Rooms run on this computer (no Daytona key)') && adminText.includes('Built-in rules only (no Nosana key)'));
  check('the admin page lists the three jobs the AI does, here done by rules', adminText.includes('Reads someone') && adminText.includes('Plans the pair') && adminText.includes('Writes the note') && adminText.includes('1 by rules'));
  check('the room itself shows no technical words', !/sandbox|sponsor|daytona|nosana|local room|destroy/i.test(await ana.page.evaluate(() => document.body.innerText)));
  // A reload in the middle restores the same room, the same chat and the same game.
  await aoi.page.reload();
  await until(aoi, () => has(aoi, 'Hello Aoi!'), 6000, 'a reload brings the chat back');
  check('a reload brings the revealed answers back', await aoi.page.locator('.wl .ans.sync').count() === 2);

  console.log('5. The bell, and the two questions');
  await until(ana, () => ana.page.locator('#done').isVisible(), 25000, 'sees the closing scene');
  await sleep(1200);
  await ana.shot('08-closing');
  await until(ana, () => has(ana, 'The room is gone'), 15000, 'is asked the two questions');
  await until(aoi, () => has(aoi, '部屋はなくなりました'), 15000, 'is asked the two questions');
  await sleep(500);
  await ana.shot('09-vote');
  check('the person is told in plain words that nothing was kept', await has(ana, 'Nothing said in it was kept') && !/sandbox|\bms\b|destroy/i.test(await ana.page.evaluate(() => document.body.innerText)));
  check('yes is a white button and no is a black one', await ana.page.evaluate(() => { const c = s => getComputedStyle(document.querySelector(s)).backgroundColor; return c('.pick.yes') === 'rgb(255, 255, 255)' && c('.pick.no') === 'rgb(7, 5, 13)'; }));
  const dead = await ana.page.context().newPage();
  const gone = await dead.goto(roomAddress);
  check('the old room address is dead', gone.status() === 404);
  await dead.close();
  for (const p of [ana, aoi]) { await p.page.locator('.pick').nth(0).click(); await p.page.locator('.pick').nth(2).click(); }
  await ana.page.click('.screen .btn.wide');
  await until(ana, () => has(ana, 'Waiting for Aoi'), 5000, 'waits for the other answer');
  check('one yes alone reveals nothing', !(await has(ana, 'You both said yes')));
  await aoi.page.click('.screen .btn.wide');
  await until(ana, () => has(ana, 'You both said yes'), 8000, 'learns of the mutual yes');
  await until(aoi, () => has(aoi, '二人とも「はい」でした'), 8000, 'learns of the mutual yes');
  await sleep(900);
  await ana.shot('10-yes');
  await aoi.shot('10-yes');
  const noteEn = await ana.page.locator('.notecard').innerText().catch(() => '');
  const noteJa = await aoi.page.locator('.notecard').innerText().catch(() => '');
  check('both see a note on what they did and what is next', /in sync once in \d+ rounds? of Same Wavelength/.test(noteEn) && noteEn.includes('Next time') && noteJa.includes('以心伝心') && noteJa.includes('次回'), noteEn + ' | ' + noteJa);
  const ics = await ana.page.evaluate(() => fetch('/api/calendar.ics').then(r => r.text()));
  check('the calendar file is a real one', ics.includes('BEGIN:VEVENT') && ics.includes('SUMMARY:Duet with Aoi') && /DTSTART:\d{8}T\d{6}Z/.test(ics));

  console.log('6. Round two');
  for (const p of [ana, aoi]) await p.page.locator('.screen .link').click(); // done for now
  await until(ana, () => has(ana, 'Round two with Aoi'), 6000, 'sees the planned session at home');
  await sleep(300);
  await ana.shot('11-home');
  await until(ana, () => inRoom(ana), 20000, 'round two starts when both are here');
  await until(aoi, () => inRoom(aoi), 12000, 'round two starts when both are here');
  await sleep(2600);
  check('round two is a different activity', await ana.page.locator('.wl').count() === 0 && await ana.page.locator('.sk, .sg').count() === 1);
  await ana.shot('12-round-two');
  await aoi.shot('12-round-two');
  // Leaving early ends it for both.
  await ana.page.click('#more');
  await ana.page.locator('#menu button', { hasText: 'Leave' }).first().click();
  await ana.page.locator('#modalbox .btn.danger').click();
  await until(aoi, () => aoi.page.locator('#done').isVisible(), 6000, 'is told the other person left');
  check('the one left behind is told who left', (await aoi.page.textContent('#donetext')).includes('Ana'));
  await until(aoi, () => has(aoi, '部屋はなくなりました'), 15000, 'still gets the two questions');
  await until(ana, () => has(ana, 'The room is gone'), 15000, 'still gets the two questions');
  await ana.page.locator('.pick').nth(1).click(); await ana.page.locator('.pick').nth(3).click(); await ana.page.click('.screen .btn.wide');
  await aoi.page.locator('.pick').nth(0).click(); await aoi.page.locator('.pick').nth(2).click(); await aoi.page.click('.screen .btn.wide');
  await until(aoi, () => has(aoi, 'ありがとうございました'), 8000, 'gets the neutral ending');
  await until(ana, () => has(ana, 'Thank you for the time'), 8000, 'gets the neutral ending');
  await sleep(700);
  await aoi.shot('13-no');
  check('a no is never attributed', !(await has(aoi, 'Ana')));

  console.log('7. What is kept');
  await hood.reload(); await sleep(1300);
  await hood.screenshot({ path: `${out}/flow-14-hood-after.png` });
  check('the admin page keeps the proof that each closed room is gone', (await hood.evaluate(() => document.getElementById('recent').innerText)).includes('checked: it is gone') && !/destroy/i.test(await hood.evaluate(() => document.body.innerText)));
  await sleep(1200);
  const saved = existsSync(join(data, 'state.json')) ? readFileSync(join(data, 'state.json'), 'utf8') : '';
  check('the record of what was done is kept', saved.includes('"activity":"wavelength"') && saved.includes('"synced":1'));
  check('nothing that was typed is kept', !saved.includes('Hello Aoi') && !saved.includes('こんにちは') && !saved.includes('Ramen') && !saved.includes('ゆっくり話したい'));
  check('no report folder exists, since nobody reported', !existsSync(join(data, 'reports')) || readdirSync(join(data, 'reports')).length === 0);
  check('no errors in either browser', errors.length === 0, errors.join(' | '));
  await browser.close();
} catch (e) {
  failures.push('the run itself: ' + e.message);
  console.log('  FAIL the run itself:', e.message.split('\n')[0]);
} finally {
  // Ctrl-C, not a plain kill, so the lobby deletes the Daytona rooms it built ahead.
  server.kill(real ? 'SIGINT' : 'SIGTERM');
  if (real) await Promise.race([new Promise(r => server.once('exit', r)), sleep(20000)]);
  rmSync(data, { recursive: true, force: true });
  if (real) writeFileSync(join(out, 'lobby-trace.log'), trace.join('\n') + '\n');
}
console.log(`\n${passed} passed, ${failures.length} failed` + (failures.length ? ':\n  - ' + failures.join('\n  - ') : ''));
process.exit(failures.length ? 1 : 0);
