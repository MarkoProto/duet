/* Small helpers shared by the lobby and the room. No framework: the pages are a few screens. */
const D = window.D = { strings: { en: {}, ja: {} } };

D.$ = id => document.getElementById(id);
D.reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
D.touch = matchMedia('(pointer: coarse)').matches;

/* Builds an element. Text always goes in as text, never as HTML. */
D.h = (tag, props, ...kids) => {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(props || {})) {
    if (v == null || v === false) continue;
    if (k === 'class') el.className = v;
    else if (k === 'text') el.textContent = v;
    else if (k === 'style') Object.assign(el.style, v);
    else if (k === 'vars') for (const [name, val] of Object.entries(v)) el.style.setProperty(name, val);
    else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'html') el.innerHTML = v; // only for our own SVG constants
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid);
  return el;
};

/* Language: English, unless the person switched to Japanese. */
D.lang = (() => {
  try { const saved = localStorage.getItem('duet-lang'); if (saved === 'en' || saved === 'ja') return saved; } catch {}
  return 'en';
})();
document.documentElement.lang = D.lang;
D.setLang = lang => {
  D.lang = lang === 'ja' ? 'ja' : 'en';
  document.documentElement.lang = D.lang;
  try { localStorage.setItem('duet-lang', D.lang); } catch {}
};
D.t = (key, vars) => {
  let s = D.strings[D.lang][key] ?? D.strings.en[key] ?? key;
  if (vars) for (const [k, v] of Object.entries(vars)) s = s.split('{' + k + '}').join(v);
  return s;
};
/* A line that arrived in both languages. */
D.pick = bi => !bi ? '' : typeof bi === 'string' ? bi : (bi[D.lang] || bi.en || bi.ja || '');

/* Animate with the Web Animations API. With reduced motion the end state is applied at once. */
D.anim = (el, frames, opts) => {
  if (!el || !el.animate) return { finished: Promise.resolve() };
  const o = typeof opts === 'number' ? { duration: opts } : { ...opts };
  if (D.reduced) o.duration = 1;
  o.easing ??= 'cubic-bezier(.16,1,.3,1)';
  o.fill ??= 'both';
  const a = el.animate(frames, o);
  a.finished.catch(() => {});
  return a;
};
D.wait = ms => new Promise(r => setTimeout(r, ms));

let toastTimer;
D.toast = (text, ms = 2600) => {
  let t = D.$('toast');
  if (!t) { t = D.h('div', { id: 'toast', role: 'status' }); document.body.append(t); }
  t.hidden = false; t.textContent = text;
  t.style.animation = 'none'; void t.offsetWidth; t.style.animation = '';
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { t.hidden = true; }, ms);
};

D.initial = name => { const c = [...(name || '?').trim()][0] || '?'; return c.toUpperCase(); };
D.face = (name, color, big) => D.h('span', { class: 'face' + (big ? ' big' : ''), vars: { '--who': color }, text: D.initial(name), title: name });

/* The mark: two rings that meet, with a spark where they cross. */
D.markSvg = cls => `<svg class="mark ${cls || ''}" viewBox="0 0 100 100" aria-hidden="true"><circle class="ring r1" cx="38" cy="50" r="30" stroke="#ff4f8b"/><circle class="ring r2" cx="62" cy="50" r="30" stroke="#7c5cff"/><path class="spark" d="M50 33 54.5 45.500 67 50 54.500 54.500 50 67 45.500 54.500 33 50 45.500 45.500z" fill="#ffb84d"/></svg>`;

D.buzz = ms => { try { navigator.vibrate && navigator.vibrate(ms); } catch {} };
D.clock = ms => { const s = Math.max(0, Math.ceil(ms / 1000)); return Math.floor(s / 60) + ':' + String(s % 60).padStart(2, '0'); };

/* A small seeded generator, so both screens build the same things from the same seed. */
D.rng = seed => { let a = (seed >>> 0) || 1; return () => { a |= 0; a = a + 0x6D2B79F5 | 0; let t = Math.imul(a ^ a >>> 15, 1 | a); t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t; return ((t ^ t >>> 14) >>> 0) / 4294967296; }; };

/* Phones: keep the page as tall as what is actually visible, so the keyboard never covers the box being typed in. */
D.fitViewport = () => {
  const vv = window.visualViewport;
  const root = document.documentElement;
  // `kb` marks a phone whose keyboard is up: the page is short, so layouts give the typing side more room.
  const set = () => { const hgt = vv ? vv.height : innerHeight; root.style.setProperty('--vh', hgt + 'px'); root.classList.toggle('kb', D.touch && hgt < 520); };
  set();
  if (vv) { vv.addEventListener('resize', set); vv.addEventListener('scroll', () => scrollTo(0, 0)); }
  addEventListener('resize', set);
};

/* A line icon in one colour: it takes the colour of the text around it. */
D.icon = name => {
  const el = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  for (const [k, v] of Object.entries({ class: 'ic', viewBox: '0 0 24 24', fill: 'none', stroke: 'currentColor', 'stroke-width': 2, 'stroke-linecap': 'round', 'stroke-linejoin': 'round', 'aria-hidden': 'true' })) el.setAttribute(k, v);
  el.innerHTML = (D.icons && D.icons[name]) || '';
  return el;
};

/* Small joys. A handful of shapes burst from wherever something was chosen; a shower of them falls
   when something good happens. Transforms and opacity only, and none at all with reduced motion. */
const SPARK_COLORS = ['#ffb84d', '#ff4f8b', '#7c5cff', '#f4f0fb'];
const SPARK_KINDS = ['', ' sq', ' ring', ' bar'];
const sparkLayer = () => D.$('sparks') || document.body.appendChild(D.h('div', { id: 'sparks', 'aria-hidden': 'true' }));
D.pop = (at, n = 10) => {
  if (D.reduced || !at) return;
  let x, y;
  if (at instanceof Element) { const r = at.getBoundingClientRect(); x = r.left + r.width / 2; y = r.top + r.height / 2; } else ({ x, y } = at);
  const layer = sparkLayer();
  for (let i = 0; i < n; i++) {
    const a = Math.PI * 2 * (i / n) + Math.random() * 0.6, d = 26 + Math.random() * 46;
    const p = D.h('i', { class: 'spark' + SPARK_KINDS[i % 4], style: { left: x + 'px', top: y + 'px' }, vars: { '--c': SPARK_COLORS[i % 4] } });
    layer.append(p);
    p.animate([
      { transform: 'translate(-50%,-50%) scale(.2)', opacity: 1 },
      { transform: `translate(calc(-50% + ${Math.cos(a) * d}px),calc(-50% + ${Math.sin(a) * d}px)) rotate(${Math.random() * 360}deg) scale(1)`, opacity: 1, offset: 0.6 },
      { transform: `translate(calc(-50% + ${Math.cos(a) * d * 1.15}px),calc(-50% + ${Math.sin(a) * d * 1.15 + 26}px)) rotate(${Math.random() * 540}deg) scale(.5)`, opacity: 0 },
    ], { duration: 520 + Math.random() * 260, easing: 'cubic-bezier(.2,.8,.3,1)' }).onfinish = () => p.remove();
  }
};
D.confetti = (n = 70) => {
  if (D.reduced) return;
  const layer = sparkLayer(), w = innerWidth, hgt = innerHeight;
  for (let i = 0; i < n; i++) {
    const p = D.h('i', { class: 'spark' + SPARK_KINDS[i % 4], style: { left: w / 2 + (Math.random() - 0.5) * w * 0.4 + 'px', top: hgt * 0.42 + 'px' }, vars: { '--c': SPARK_COLORS[i % 4] } });
    layer.append(p);
    const dx = (Math.random() - 0.5) * w * 0.95, up = 90 + Math.random() * hgt * 0.32, turn = (Math.random() - 0.5) * 1080;
    p.animate([
      { transform: 'translate(-50%,-50%) scale(.3)', opacity: 1 },
      { transform: `translate(calc(-50% + ${dx * 0.6}px),calc(-50% - ${up}px)) rotate(${turn * 0.4}deg) scale(1.25)`, opacity: 1, offset: 0.28, easing: 'cubic-bezier(.3,0,.7,1)' },
      { transform: `translate(calc(-50% + ${dx}px),calc(-50% + ${hgt * 0.62}px)) rotate(${turn}deg) scale(.9)`, opacity: 0 },
    ], { duration: 1500 + Math.random() * 1100, delay: Math.random() * 180, easing: 'cubic-bezier(.2,.7,.3,1)', fill: 'backwards' }).onfinish = () => p.remove();
  }
};
/* A quick squash and stretch, for something that was just pressed. */
D.jelly = el => {
  if (D.reduced || !el || !el.animate) return;
  el.animate([{ transform: 'scale(1)' }, { transform: 'scale(1.08,.92)' }, { transform: 'scale(.95,1.06)' }, { transform: 'scale(1.03,.98)' }, { transform: 'scale(1)' }], { duration: 420, easing: 'ease-out' });
};
