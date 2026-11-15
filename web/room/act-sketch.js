/* Four Hands: one drawing, two pens, one line each in turn.
   A finished stroke is thinned, stored as small whole numbers and sent as one ordered move. While the
   pen is still moving, its points are passed on a few at a time so the other person sees it grow.
   Every point carries the moment it was drawn, and the other screen plays the line back at that pace,
   a moment behind: points that arrive in bursts (held requests through the lobby arrive a few times a
   second) still grow as one smooth line. */
(() => {
  const { h, t } = D;
  D.acts = D.acts || {};
  const UNIT = 10000;        // a point is two whole numbers, 0 to 10000, across and down the sheet
  const WIDTHS = [4, 9, 18]; // thousandths of the sheet's width
  const FALLBACK = [
    { en: 'A perfect Sunday', ja: '理想の日曜日' },
    { en: 'The view from a window you both like', ja: '二人が好きな窓からの景色' },
    { en: 'A creature that does not exist', ja: '存在しない生き物' },
    { en: 'What you ate today', ja: '今日食べたもの' },
  ];

  /* Ramer-Douglas-Peucker: keeps the points that carry the shape and drops the rest. Returns which
     points to keep, so their times can be kept with them. */
  function keepAt(p, eps) {
    const n = p.length / 2;
    if (n < 3) return [...Array(n).keys()];
    const keep = new Uint8Array(n); keep[0] = keep[n - 1] = 1;
    const stack = [[0, n - 1]];
    while (stack.length) {
      const [a, b] = stack.pop();
      const ax = p[a * 2], ay = p[a * 2 + 1], dx = p[b * 2] - ax, dy = p[b * 2 + 1] - ay, len = Math.hypot(dx, dy);
      let worst = 0, at = -1;
      for (let i = a + 1; i < b; i++) {
        // A line that ends where it began (a circle, a loop) has no direction: measure from that point.
        const d = len ? Math.abs(dy * (p[i * 2] - ax) - dx * (p[i * 2 + 1] - ay)) / len : Math.hypot(p[i * 2] - ax, p[i * 2 + 1] - ay);
        if (d > worst) { worst = d; at = i; }
      }
      if (worst > eps) { keep[at] = 1; stack.push([a, at], [at, b]); }
    }
    const out = [];
    for (let i = 0; i < n; i++) if (keep[i]) out.push(i);
    return out;
  }
  const thin = (p, eps) => keepAt(p, eps).flatMap(i => [p[i * 2], p[i * 2 + 1]]);
  D.sketchThin = thin;

  /* When each point was drawn, in ms from the line's start: the sender's own times when they came
     with it, otherwise evenly spaced (an older page). */
  const timesFor = (p, t) => Array.isArray(t) && t.length === p.length / 2 && t.every(Number.isFinite) ? t.map(v => Math.max(0, v | 0)) : p.filter((_, i) => i % 2 === 0).map((_, i) => i * 8);

  D.acts.sketch = (root, ctx) => {
    const { you } = ctx;
    const given = ctx.data && Array.isArray(ctx.data.prompts) ? ctx.data.prompts.filter(p => p && (p.en || p.ja)) : [];
    const prompts = given.length ? given : FALLBACK;
    const COLORS = [ctx.people[0].color, ctx.people[1].color, '#f4f0fb', '#7c5cff', '#4cd98a', '#5cc8ff'];
    let strokes = [], promptAt = 0, color = you, width = 1, count = 0, redrawQueued = false, liveQueued = false;
    const known = new Set();
    const growing = new Map(); // strokes still being drawn, or sent and not yet echoed
    let mine = null, sentUpTo = 0, lastSend = 0;
    // Turns: one line each. Whose turn it is follows from the last line in the room's order, so both
    // screens always agree; either may draw the first one. A line sent and not yet back from the
    // room still counts as drawn, and so does a line the other is drawing right now.
    let sentAndWaiting = null;
    const turnOf = () => (strokes.length ? 1 - strokes[strokes.length - 1].by : null);
    // The other's line still being drawn. One that has not grown for a few seconds is let go:
    // their page may have closed mid-line, and the turn must not stay stuck on it.
    const STALE_MS = 3500;
    const otherDrawing = () => [...growing.values()].some(s => s.by !== you && !s.done && performance.now() - (s.at || 0) < STALE_MS);
    setInterval(() => {
      let dropped = false;
      for (const [id, s] of growing) if (s.by !== you && !s.done && performance.now() - (s.at || 0) > STALE_MS) { growing.delete(id); dropped = true; }
      if (dropped) { queueLive(); showTurn(); }
    }, 1000);
    const myTurn = () => !sentAndWaiting && !otherDrawing() && (turnOf() === null || turnOf() === you);

    const title = h('b');
    const turn = h('span', { class: 'pill turn' });
    const base = h('canvas'), live = h('canvas');
    const sheet = h('div', { class: 'sheetbox' }, base, live);
    const paper = h('div', { class: 'paper' }, sheet);
    const swatches = COLORS.map((c, i) => h('button', { class: 'sw' + (i === color ? ' now' : ''), type: 'button', vars: { '--c': c }, 'aria-label': 'color', onclick: () => { color = i; pickTools(); } }));
    const widths = WIDTHS.map((w, i) => h('button', { class: 'wd' + (i === width ? ' now' : ''), type: 'button', 'aria-label': 'pen size', vars: { '--d': 4 + i * 5 + 'px' }, onclick: () => { width = i; pickTools(); } }, h('i')));
    const pickTools = () => { swatches.forEach((b, i) => b.classList.toggle('now', i === color)); widths.forEach((b, i) => b.classList.toggle('now', i === width)); };
    const box = h('div', { class: 'sk' },
      h('div', { class: 'bar' }, title, h('button', { class: 'mini', type: 'button', onclick: e => { if (!myTurn()) return; D.jelly(e.currentTarget); ctx.send({ k: 'prompt', i: promptAt + 1 }); } }, D.icon('shuffle'), h('span', { text: t('sk_new') }))),
      turn,
      paper,
      h('div', { class: 'tools' }, swatches, widths,
        // Undo takes back your line while it is still the last one, which gives the turn back to you.
        h('button', { class: 'mini', type: 'button', onclick: () => { const last = strokes[strokes.length - 1]; if (last && last.by === you && !otherDrawing()) ctx.send({ k: 'undo', id: last.id }); } }, D.icon('undo-2'), h('span', { text: t('sk_undo') })),
        h('button', { class: 'mini', type: 'button', onclick: () => { if (strokes.some(s => s.by === you)) ctx.send({ k: 'clear' }); } }, D.icon('eraser'), h('span', { text: t('sk_clear') }))));
    root.replaceChildren(box);
    function showTurn() {
      const yours = myTurn();
      turn.classList.toggle('good', yours);
      turn.textContent = turnOf() === null && !otherDrawing() && !sentAndWaiting ? t('sk_first') : yours ? t('sk_your_turn') : t('sk_their_turn', { name: ctx.people[ctx.other].name });
      sheet.classList.toggle('wait', !yours && !mine);
    }
    const showPrompt = () => { title.textContent = D.pick(prompts[promptAt % prompts.length]); D.anim(title, [{ opacity: 0, transform: 'translateY(8px)' }, { opacity: 1, transform: 'none' }], 350); };
    showPrompt();

    let W = 0, H = 0, dpr = 1;
    const g = base.getContext('2d'), gl = live.getContext('2d');
    function size() {
      const r = paper.getBoundingClientRect();
      if (r.width < 20 || r.height < 20) return;
      const w = Math.floor(Math.min(r.width, r.height * 4 / 3)), hgt = Math.floor(w * 3 / 4);
      if (w === W && hgt === H) return;
      W = w; H = hgt; dpr = Math.min(devicePixelRatio || 1, 2);
      sheet.style.width = W + 'px'; sheet.style.height = H + 'px';
      for (const c of [base, live]) { c.width = W * dpr; c.height = H * dpr; }
      redraw(); drawLive();
    }
    function ink(c2, s) {
      const p = s.p, n = p.length / 2;
      if (!n) return;
      const X = i => p[i * 2] / UNIT * W * dpr, Y = i => p[i * 2 + 1] / UNIT * H * dpr;
      c2.lineCap = c2.lineJoin = 'round';
      c2.strokeStyle = COLORS[s.c] || COLORS[0];
      c2.lineWidth = Math.max(1, (WIDTHS[s.w] || WIDTHS[1]) / 1000 * W * dpr);
      c2.beginPath();
      c2.moveTo(X(0), Y(0));
      if (n === 1 || (n === 2 && p[0] === p[2] && p[1] === p[3])) c2.lineTo(X(0) + 0.01, Y(0)); // a dot still shows
      // Curves through the midpoints, so a thinned stroke still looks drawn by a hand.
      for (let i = 1; i < n - 1; i++) c2.quadraticCurveTo(X(i), Y(i), (X(i) + X(i + 1)) / 2, (Y(i) + Y(i + 1)) / 2);
      if (n > 1) c2.lineTo(X(n - 1), Y(n - 1));
      c2.stroke();
    }
    function redraw() { redrawQueued = false; g.clearRect(0, 0, base.width, base.height); for (const s of strokes) if (!s.wait) ink(g, s); }
    const queueRedraw = () => { if (!redrawQueued) { redrawQueued = true; requestAnimationFrame(redraw); } };
    function drawLive() { liveQueued = false; gl.clearRect(0, 0, live.width, live.height); for (const s of growing.values()) ink(gl, s.by === you ? s : shown(s)); }
    const queueLive = () => { if (!liveQueued) { liveQueued = true; requestAnimationFrame(drawLive); } };

    /* Playing the other's line back. The playhead runs at the pace it was drawn, kept about one
       delivery gap behind the newest point: a little slower when it gets close, a little faster when
       it falls behind, so the line never stops and starts. Once the line is finished it catches up
       quickly, then the line moves onto the sheet. */
    let gap = 120; // how far apart deliveries arrive, smoothed (ms)
    const lead = () => Math.min(800, Math.max(90, gap * 1.3));
    function shown(s) {
      if (s.ph == null || !s.t.length) return { c: s.c, w: s.w, p: [] };
      const t = s.t;
      let k = s.k || 0;
      while (k + 1 < t.length && t[k + 1] <= s.ph) k++;
      s.k = k;
      const p = s.p.slice(0, (k + 1) * 2);
      if (k + 1 < t.length && t[k + 1] > t[k]) {
        const f = Math.min(1, Math.max(0, (s.ph - t[k]) / (t[k + 1] - t[k])));
        p.push(s.p[k * 2] + (s.p[k * 2 + 2] - s.p[k * 2]) * f, s.p[k * 2 + 1] + (s.p[k * 2 + 3] - s.p[k * 2 + 1]) * f);
      }
      return { c: s.c, w: s.w, p };
    }
    let playing = false, lastFrame = 0;
    function play(now) {
      const dt = Math.min(100, lastFrame ? now - lastFrame : 16);
      lastFrame = now;
      let busy = false;
      for (const s of [...growing.values()]) {
        if (s.by === you || !s.t.length) continue;
        const newest = s.t[s.t.length - 1];
        if (s.ph == null) {
          if (!s.done && newest - s.t[0] < lead() && performance.now() - s.first < lead()) { busy = true; continue; }
          s.ph = s.t[0];
        }
        const behind = newest - s.ph;
        const speed = s.done ? Math.min(6, Math.max(1.5, behind / 150)) : Math.min(3, Math.max(0.35, behind / lead()));
        s.ph = Math.min(newest, s.ph + dt * speed);
        if (s.done && s.ph >= newest) {
          growing.delete(s.id);
          s.final.wait = false;
          if (W) ink(g, s.final);
          continue;
        }
        busy = true;
      }
      drawLive();
      if (busy) requestAnimationFrame(play); else { playing = false; lastFrame = 0; }
    }
    const kick = () => { if (!playing) { playing = true; lastFrame = 0; requestAnimationFrame(play); } };

    const at = e => { const r = sheet.getBoundingClientRect(); return [Math.round(Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) * UNIT), Math.round(Math.min(1, Math.max(0, (e.clientY - r.top) / r.height)) * UNIT)]; };
    function push(e) {
      const [x, y] = at(e), p = mine.p, n = p.length;
      if (n && Math.abs(p[n - 2] - x) + Math.abs(p[n - 1] - y) < 12) return;
      if (n >= 4000) return;
      const when = Math.round((e.timeStamp || performance.now()) - mine.t0);
      p.push(x, y);
      mine.t.push(Math.max(mine.t.length ? mine.t[mine.t.length - 1] : 0, when));
    }
    function share(force) {
      const now = performance.now();
      if (!mine || (!force && now - lastSend < 50) || mine.p.length <= sentUpTo) return;
      ctx.live('pen', { id: mine.id, c: mine.c, w: mine.w, p: mine.p.slice(sentUpTo, sentUpTo + 240), t: mine.t.slice(sentUpTo / 2, sentUpTo / 2 + 120) });
      sentUpTo = Math.min(mine.p.length, sentUpTo + 240);
      lastSend = now;
    }
    sheet.addEventListener('pointerdown', e => {
      if (e.button || mine || !ctx.open()) return;
      if (!myTurn()) { turn.classList.remove('nudge'); void turn.offsetWidth; turn.classList.add('nudge'); return; }
      e.preventDefault();
      sheet.setPointerCapture(e.pointerId);
      mine = { id: you + '-' + (count++) + '-' + Math.random().toString(36).slice(2, 7), by: you, c: color, w: width, p: [], t: [], t0: e.timeStamp || performance.now(), pid: e.pointerId };
      sentUpTo = 0;
      growing.set(mine.id, mine);
      push(e); queueLive();
    });
    sheet.addEventListener('pointermove', e => {
      if (!mine || e.pointerId !== mine.pid) return; // a second finger or a palm does not join the line
      for (const c of (e.getCoalescedEvents?.() || [e])) push(c);
      queueLive(); share(false);
    });
    const finish = e => {
      if (!mine || (e && e.pointerId !== mine.pid)) return;
      const s = mine; mine = null;
      share(true);
      let eps = 10, kept = keepAt(s.p, eps);
      while (kept.length > 600) kept = keepAt(s.p, eps *= 1.6);
      const p = kept.flatMap(i => [s.p[i * 2], s.p[i * 2 + 1]]), t = kept.map(i => s.t[i]);
      s.p = p;
      s.t = t;
      queueLive();
      sentAndWaiting = s.id;
      showTurn();
      ctx.send({ k: 'stroke', id: s.id, c: s.c, w: s.w, p, t });
    };
    for (const ev of ['pointerup', 'pointercancel', 'lostpointercapture']) sheet.addEventListener(ev, finish);

    new ResizeObserver(size).observe(paper);
    requestAnimationFrame(size);
    showTurn();

    return {
      onLive(by, k, d) {
        if (k !== 'pen' || !d || typeof d.id !== 'string' || !Array.isArray(d.p) || known.has(d.id)) return;
        let s = growing.get(d.id);
        const now = performance.now();
        if (!s) { if (growing.size > 8) return; s = { id: d.id, by, c: d.c | 0, w: d.w | 0, p: [], t: [], first: now }; growing.set(d.id, s); }
        if (s.done) return;
        // Deliveries that come together (one held request) count as one.
        if (s.at && now - s.at > 15) gap = gap * 0.75 + Math.min(1500, now - s.at) * 0.25;
        s.at = now;
        const pts = d.p.length - (d.p.length % 2);
        if (s.p.length < 8000 && pts) {
          const last = s.t.length ? s.t[s.t.length - 1] : 0;
          const t = Array.isArray(d.t) && d.t.length === pts / 2 ? d.t : null;
          for (let i = 0; i < pts / 2; i++) {
            s.p.push(d.p[i * 2] | 0, d.p[i * 2 + 1] | 0);
            s.t.push(Math.max(s.t.length ? s.t[s.t.length - 1] : 0, t ? t[i] | 0 : last + (i + 1) * 8));
          }
        }
        kick();
        showTurn();
      },
      onAct(by, d, replay) {
        if (!d) return;
        if (d.k === 'stroke' && typeof d.id === 'string' && Array.isArray(d.p) && !known.has(d.id) && strokes.length < 3000) {
          const s = { id: d.id, by, c: d.c | 0, w: d.w | 0, p: d.p };
          known.add(d.id); strokes.push(s);
          if (sentAndWaiting === d.id) sentAndWaiting = null;
          if (by === you || replay || document.hidden) {
            growing.delete(d.id);
            if (W) ink(g, s);
          } else {
            // Keep playing it back, now from the finished line, then it moves onto the sheet.
            s.wait = true;
            const live = growing.get(d.id) || { id: d.id, by, c: s.c, w: s.w, p: [], t: [], first: performance.now(), at: performance.now() };
            live.p = d.p;
            live.t = timesFor(d.p, d.t);
            live.k = 0;
            live.done = true;
            live.final = s;
            growing.set(d.id, live);
            kick();
          }
          queueLive();
          if (!replay) ctx.score({ kind: 'sketch', strokes: strokes.length });
        } else if (d.k === 'undo') {
          const n = strokes.length;
          strokes = strokes.filter(s => !(s.id === d.id && s.by === by));
          if (growing.get(d.id)?.by === by) { growing.delete(d.id); queueLive(); }
          if (strokes.length !== n) queueRedraw();
        } else if (d.k === 'clear') {
          strokes = strokes.filter(s => s.by !== by);
          for (const [id, s] of growing) if (s.by === by && s.done) growing.delete(id);
          queueRedraw(); queueLive();
        } else if (d.k === 'prompt' && typeof d.i === 'number' && d.i === promptAt + 1) {
          promptAt = d.i;
          if (!replay) showPrompt(); else title.textContent = D.pick(prompts[promptAt % prompts.length]);
        }
        showTurn();
      },
    };
  };
})();
