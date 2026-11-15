// A simulated evening on Duet, photographed: ten people, five rooms, one of them followed
// screen by screen on a phone. Writes the pictures to docs/screens and the storyboard to docs/.
//   node tour.mjs
// It runs its own lobby with a 50-second session and local rooms. No keys, nothing else touched.
import { chromium } from 'playwright-core';
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';

const docs = resolve('../../docs');
const out = join(docs, 'screens');
mkdirSync(out, { recursive: true });
for (const f of readdirSync(out)) if (f.endsWith('.png')) rmSync(join(out, f));
const port = 8700 + Math.floor(Math.random() * 90);
const base = `http://127.0.0.1:${port}`;
const data = mkdtempSync(join(tmpdir(), 'duet-tour-'));
const env = { ...process.env, DUET_BIND: `127.0.0.1:${port}`, DUET_PUBLIC_URL: base, DUET_DATA: data, DUET_ROOMS: 'local', DUET_SESSION_SECS: '50', DUET_VOTE_SECS: '90',
  DUET_SETTLE_SECS: '0.6', DUET_GATHER_SECS: '2', DUET_ACTIVITIES: 'wavelength,sketch,signal', DAYTONA_API_KEY: '', NOSANA_API_KEY: '', NOSANA_LLM_URL: '', HOME: data };
const server = spawn('../../target/debug/duet', [], { env, stdio: ['ignore', 'pipe', 'inherit'] });
server.stdout.on('data', () => {});
// A second lobby with the real 15-minute clock, only for the welcome screens.
const front = `http://127.0.0.1:${port + 100}`;
const frontData = mkdtempSync(join(tmpdir(), 'duet-tour-front-'));
const frontServer = spawn('../../target/debug/duet', [], { env: { ...env, DUET_BIND: `127.0.0.1:${port + 100}`, DUET_PUBLIC_URL: front, DUET_DATA: frontData, DUET_SESSION_SECS: '900' }, stdio: ['ignore', 'pipe', 'inherit'] });
frontServer.stdout.on('data', () => {});
const until = async (what, test, ms = 20000) => {
  const end = Date.now() + ms;
  for (;;) { try { if (await test()) return; } catch {} if (Date.now() > end) throw new Error('timed out: ' + what); await sleep(100); }
};
const post = (page, path, body) => page.evaluate(([p, b]) => fetch(p, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(b) }).then(r => r.ok), [path, body]);
const inRoom = page => until('room opens', async () => page.url().includes('/r/') && !(await page.locator('#app.gated').count()) && await page.locator('#clock').isVisible(), 30000);
const shot = (page, name) => page.screenshot({ path: join(out, name + '.png') });

/* Someone in the background: signs up through the same requests the page makes, then the page takes over. */
async function extra(browser, name, answers) {
  const ctx = await browser.newContext({ viewport: { width: 420, height: 720 }, locale: 'en-GB' });
  await ctx.addInitScript(() => { try { sessionStorage.setItem('duet-tut-off', '1'); } catch {} }); // these tests go straight to the game
  const page = await ctx.newPage();
  await page.goto(base);
  await post(page, '/api/join', { name, lang: 'en', adult: true });
  await page.reload();
  await page.waitForSelector('.q .opt');
  // The page is now listening, so it follows the person from the queue into the room by itself.
  await post(page, '/api/answers', { langs: ['en'], smile: '', ...answers });
  return page;
}
const players = { mood: 'playful', mode: 'play', style: 'rival', talk: 'both', interests: ['games', 'sports', 'tech'] };
const makers = { mood: 'tired', mode: 'make', style: 'either', talk: 'both', interests: ['art', 'nature', 'books'] };

let browser;
try {
  await sleep(700);
  browser = await chromium.launch({ channel: 'chrome' });

  // Earlier this evening: two pairs met, one of them wants to meet again.
  const early = [await extra(browser, 'Kai', players), await extra(browser, 'Mio', players)];
  await Promise.all(early.map(inRoom));
  const early2 = [await extra(browser, 'Leo', makers), await extra(browser, 'Yui', makers)];
  await Promise.all(early2.map(inRoom));
  await sleep(3000);
  for (const [pair, again] of [[early, [true, true]], [early2, [true, false]]]) {
    await pair[0].evaluate(() => D.net.send({ t: 'leave' }));
    for (const [i, page] of pair.entries()) {
      await until('asked the two questions', () => page.getByText('The room is gone').isVisible(), 20000);
      await post(page, '/api/vote', { liked: true, again: again[i] });
    }
  }

  // The person we follow, on a phone.
  const phone = await browser.newContext({ viewport: { width: 390, height: 800 }, deviceScaleFactor: 2, hasTouch: true, isMobile: true, locale: 'en-GB' });
  await phone.addInitScript(() => { try { sessionStorage.setItem('duet-tut-off', '1'); } catch {} }); // these tests go straight to the game
  const ana = await phone.newPage();
  await ana.goto(front);
  await ana.waitForSelector('.screen .input');
  await sleep(2600);
  await shot(ana, 'user-01-welcome');
  await ana.fill('.screen .input', 'Ana');
  await ana.click('.agree');
  await sleep(350);
  await shot(ana, 'user-02-name');
  await ana.goto(base);
  await ana.waitForSelector('.screen .input');
  await ana.fill('.screen .input', 'Ana');
  await ana.click('.agree');
  await ana.click('.screen .btn');
  const pick = async n => { await ana.waitForSelector('.q .opt'); await sleep(420); await ana.locator('.q .opt').nth(n).click(); await sleep(380); };
  await pick(0);                                   // English
  await ana.waitForSelector('.q .opt'); await sleep(1100);
  await shot(ana, 'user-03-question');
  await pick(0); await pick(2); await pick(0); await pick(1);   // calm, just talk, team up, talker
  await ana.waitForSelector('.q .grid .chip'); await sleep(350);
  for (const i of [0, 3, 4]) { await ana.locator('.q .chip').nth(i).click(); await sleep(120); }
  await sleep(300);
  await shot(ana, 'user-04-interests');
  await ana.click('.q .btn');
  await ana.waitForSelector('.q textarea');
  await ana.fill('.q textarea', 'A stranger held the door and we both laughed.');
  await sleep(350);
  await shot(ana, 'user-05-one-small-thing');
  await ana.click('.q .btn');
  await until('waiting', () => ana.getByText('Finding your person').isVisible());
  await sleep(1500);
  await shot(ana, 'user-06-finding');

  // The other person, on a laptop.
  const laptop = await browser.newContext({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 1.5, locale: 'en-GB' });
  await laptop.addInitScript(() => { try { sessionStorage.setItem('duet-tut-off', '1'); } catch {} }); // these tests go straight to the game
  const aoi = await laptop.newPage();
  await aoi.goto(front);
  await aoi.waitForSelector('.screen .input');
  await sleep(2600);
  await shot(aoi, 'user-laptop-welcome');
  await aoi.goto(base);
  await aoi.waitForSelector('.screen .input');
  await post(aoi, '/api/join', { name: 'Aoi', lang: 'en', adult: true });
  await aoi.reload();
  await aoi.waitForSelector('.q .opt');
  await post(aoi, '/api/answers', { langs: ['en', 'ja'], mood: 'calm', mode: 'talk', style: 'team', talk: 'listener', interests: ['music', 'food'], smile: '' });
  await until('paired', () => ana.getByText('You and Aoi').isVisible());
  await sleep(900);
  await shot(ana, 'user-07-paired');
  await shot(aoi, 'user-laptop-paired');
  await until('the hello', () => ana.locator('#veil.hi').isVisible(), 20000);
  await sleep(450);
  await shot(ana, 'user-08-say-hi');
  await Promise.all([inRoom(ana), inRoom(aoi)]);
  await sleep(1300);

  // Meanwhile four more people arrive and are paired.
  const later = Promise.all([extra(browser, 'Hana', players), extra(browser, 'Sora', players), extra(browser, 'Emi', makers), extra(browser, 'Taro', makers)]);

  const say = async (page, text) => { await page.fill('#t', text); await page.press('#t', 'Enter'); await sleep(650); };
  await say(aoi, 'Hi Ana! First time here 🙂');
  await say(ana, 'Hi Aoi, same for me');
  // The prompts are shuffled, so the two answer whatever was asked, and happen to agree.
  const ANSWERS = { 'A food you could eat every day for a week': 'Ramen', 'The best season': 'Autumn', 'A sound that calms you down': 'Rain', 'Something you always carry': 'Headphones',
    'A perfect Sunday, in one word': 'Slow', 'The first thing you do after waking up': 'Coffee', 'A smell that takes you back to childhood': 'Rain', 'The best time of day': 'Evening',
    'A word you say too often': 'Sorry', 'A small luxury worth paying for': 'Good coffee', 'What you would do with one free hour right now': 'A walk', 'An instrument you wish you could play': 'Piano',
    'A song everyone knows the words to': 'Happy Birthday', 'A film worth watching twice': 'Spirited Away', 'The best snack for a film': 'Popcorn', 'The best thing to eat at midnight': 'Ramen',
    'A topping that makes anything better': 'Cheese', 'A city you would move to tomorrow': 'Lisbon', 'Window seat or aisle seat': 'Window' };
  const answer = ANSWERS[(await ana.locator('.wl .prompt').textContent()).trim()] || 'Coffee';
  await ana.fill('.wl .input', answer);
  await ana.click('.wl .btn');
  await sleep(500);
  await aoi.fill('.wl .input', answer.toLowerCase() + '!');
  await aoi.click('.wl .btn');
  await until('the reveal', () => ana.locator('.wl .ans.sync').first().isVisible());
  await say(aoi, 'No way, the same answer!');
  await say(ana, 'We are going to get along 😄');
  await aoi.locator('#log .m.mine').last().dblclick();
  await aoi.locator('.emo').nth(4).click();
  await sleep(1500);
  await shot(ana, 'user-09-together');
  await shot(aoi, 'user-laptop-together');

  const others = await later;
  await Promise.all(others.map(inRoom));
  const admin = await browser.newPage({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1.5 });
  await admin.goto(base + '/hood');
  await sleep(1600);
  await shot(admin, 'admin-01-rooms-open');

  await until('the last seconds', () => ana.locator('#clock.last').isVisible(), 60000);
  await sleep(600);
  await shot(ana, 'user-10-last-seconds');
  await until('the bell', () => ana.locator('#done').isVisible(), 60000);
  await sleep(1500);
  await shot(ana, 'user-11-time-is-up');
  await until('the two questions', () => ana.getByText('The room is gone').isVisible(), 30000);
  await until('the two questions', () => aoi.getByText('The room is gone').isVisible(), 30000);
  await sleep(500);
  await ana.locator('.pick').nth(0).click();
  await ana.locator('.pick').nth(2).click();
  await sleep(450);
  await shot(ana, 'user-12-two-questions');
  await ana.click('.screen .btn.wide');
  await sleep(700);
  await shot(ana, 'user-13-waiting-for-the-other');
  await aoi.locator('.pick').nth(0).click();
  await aoi.locator('.pick').nth(2).click();
  await aoi.click('.screen .btn.wide');
  await until('both said yes', () => ana.getByText('You both said yes').isVisible());
  await sleep(4200); // let the confetti clear
  await shot(ana, 'user-14-both-said-yes');
  await shot(aoi, 'user-laptop-both-said-yes');
  await ana.locator('.screen .link').click();
  await until('home', () => ana.getByText('Round two with Aoi').isVisible());
  await sleep(700);
  await shot(ana, 'user-15-see-you-tomorrow');

  // The other rooms run out; their people answer the two questions.
  for (const [i, page] of others.entries()) {
    await until('others asked', () => page.getByText('The room is gone').isVisible(), 70000);
    await post(page, '/api/vote', { liked: true, again: i < 2 });
  }
  await sleep(2500);
  await shot(admin, 'admin-02-rooms-closed');
  // One ending that is not a yes, for completeness.
  const yui = early2[1];
  await shot(yui, 'user-16-no-second-round');

  // The storyboard: the phone screens in order, on one sheet.
  const frames = [
    ['user-02-name', 'Say who you are', 'A name and one tick. No email, no password.'],
    ['user-03-question', 'A few taps', 'Six quick questions about your mood and how you like to spend time.'],
    ['user-04-interests', 'What you like', 'Pick up to three things you enjoy.'],
    ['user-06-finding', 'We look for your person', 'A short wait while someone who fits you is found.'],
    ['user-07-paired', 'You are paired', 'You see who it is, and why the two of you were put together.'],
    ['user-08-say-hi', 'Say hi', 'A private room opens, for just the two of you.'],
    ['user-09-together', 'Fifteen minutes together', 'One small activity, a chat and reactions, and a clock you share.'],
    ['user-11-time-is-up', 'Time is up', 'The room closes for good. Nothing said in it is kept.'],
    ['user-12-two-questions', 'Two private questions', 'Did you like it? Do you want more time? The other person only finds out if you both say yes.'],
    ['user-14-both-said-yes', 'See you tomorrow', 'If you both say yes, you meet again the next day, for longer.'],
  ];
  const html = `<!doctype html><meta charset="utf-8"><title>Duet storyboard</title>
<style>
@font-face{font-family:"Bricolage Grotesque";font-weight:600;src:url(../web/shared/fonts/bricolage-latin.woff2) format("woff2")}
@font-face{font-family:"Figtree";font-weight:400 600;src:url(../web/shared/fonts/figtree-latin.woff2) format("woff2")}
*{box-sizing:border-box}
body{margin:0;width:1760px;padding:56px 60px 44px;color:#f4f0fb;font:17px/1.4 Figtree,system-ui,sans-serif;background:radial-gradient(circle at 12% 8%,rgba(255,79,139,.30),transparent 46%),radial-gradient(circle at 88% 92%,rgba(124,92,255,.38),transparent 50%),radial-gradient(circle at 78% 6%,rgba(255,184,77,.16),transparent 36%),#0c0914}
header{display:flex;align-items:center;gap:18px;margin-bottom:8px}
header svg{width:64px;height:64px}
h1{margin:0;font:600 50px/1.05 "Bricolage Grotesque",sans-serif;letter-spacing:-.01em}
h1 span{background:linear-gradient(115deg,#ffb84d,#ff4f8b 52%,#7c5cff);-webkit-background-clip:text;background-clip:text;color:transparent}
.lead{margin:0 0 38px;color:#ada4c4;font-size:21px}
.grid{display:grid;grid-template-columns:repeat(5,1fr);gap:44px 34px}
.step{position:relative;display:flex;flex-direction:column;align-items:center;text-align:center}
.phone{position:relative;width:236px;border-radius:30px;padding:8px;background:#171228;border:1.500px solid #3a2f57;box-shadow:0 22px 50px rgba(0,0,0,.5)}
.phone img{display:block;width:100%;border-radius:23px}
.n{position:absolute;left:-14px;top:-14px;width:44px;height:44px;border-radius:50%;display:grid;place-items:center;font:600 21px "Bricolage Grotesque",sans-serif;color:#1a0b1e;background:linear-gradient(115deg,#ffb84d,#ff4f8b 52%,#7c5cff);box-shadow:0 6px 18px rgba(255,79,139,.45)}
.step h2{margin:16px 0 4px;font:600 22px/1.2 "Bricolage Grotesque",sans-serif}
.step p{margin:0;color:#ada4c4;font-size:15.500px;max-width:270px;text-wrap:balance}
.step:not(:nth-child(5n))::after{content:"";position:absolute;right:-27px;top:240px;width:20px;height:20px;border-top:3px solid #ff4f8b;border-right:3px solid #ff4f8b;transform:rotate(45deg);opacity:.9}
footer{margin-top:36px;display:flex;justify-content:space-between;color:#ada4c4;font-size:15px}
</style>
<header><svg viewBox="0 0 100 100"><circle cx="38" cy="50" r="30" fill="none" stroke="#ff4f8b" stroke-width="8"/><circle cx="62" cy="50" r="30" fill="none" stroke="#7c5cff" stroke-width="8"/><path d="M50 33 54.5 45.5 67 50 54.5 54.5 50 67 45.5 54.5 33 50 45.5 45.5z" fill="#ffb84d"/></svg>
<h1>Duet: <span>how a person moves through the app</span></h1></header>
<p class="lead">Fifteen minutes with someone new. One small thing to do together, and then the room is gone.</p>
<div class="grid">${frames.map(([img, title, text], i) => `<div class="step"><div class="phone"><span class="n">${i + 1}</span><img src="screens/${img}.png"></div><h2>${title}</h2><p>${text}</p></div>`).join('')}</div>
<footer><span>If one of the two says no, both see the same neutral goodbye, and neither is told who said it.</span><span>Screens from a real run of the app, with the clock shortened to 50 seconds.</span></footer>`;
  writeFileSync(join(docs, 'storyboard.html'), html);
  const board = await browser.newPage({ viewport: { width: 1760, height: 1000 }, deviceScaleFactor: 1.5 });
  await board.goto('file://' + join(docs, 'storyboard.html'));
  await sleep(600);
  await board.screenshot({ path: join(docs, 'storyboard.png'), fullPage: true });
  await browser.close();
  console.log('pictures:', readdirSync(out).filter(f => f.endsWith('.png')).length, 'in', out);
} catch (e) {
  console.log('FAILED:', e.message.split('\n')[0]);
  process.exitCode = 1;
} finally {
  await browser?.close().catch(() => {});
  server.kill();
  frontServer.kill();
  rmSync(frontData, { recursive: true, force: true });
  rmSync(data, { recursive: true, force: true });
}
