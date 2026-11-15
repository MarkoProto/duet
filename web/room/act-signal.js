/* Signal: one person sees the safe path, the other walks it. Three pings per level, the rest by chat.
   Both screens build the same level from the room's seed, and every step is an ordered move, so the
   two screens can never disagree about where the walker stands. */
(() => {
  const { h, t } = D;
  D.acts = D.acts || {};
  const PINGS = 3;

  /* A path that never crosses itself, from the left edge to the right edge: a randomised depth-first
     walk that backs out of dead ends, bounded so a bad seed cannot hang the page. */
  function makePath(seed, level) {
    const n = Math.min(5 + level, 8);
    const rnd = D.rng((seed ^ 0x9e3779b9) + level * 7919);
    const minLen = n + 2 + Math.floor(rnd() * 3), maxLen = 2 * n + 5;
    const key = (x, y) => y * n + x;
    const start = [0, Math.floor(rnd() * n)];
    const path = [start], seen = new Set([key(...start)]), options = [];
    const around = ([x, y]) => {
      const list = [[x + 1, y], [x, y - 1], [x, y + 1], [x - 1, y]].filter(([a, b]) => a >= 0 && b >= 0 && a < n && b < n && !seen.has(key(a, b)));
      for (let i = list.length - 1; i > 0; i--) { const j = Math.floor(rnd() * (i + 1)); [list[i], list[j]] = [list[j], list[i]]; }
      return list;
    };
    options.push(around(start));
    let budget = 20000;
    while (path.length && budget-- > 0) {
      const here = path[path.length - 1];
      if (here[0] === n - 1 && path.length >= minLen) return { n, path };
      const next = path.length < maxLen && here[0] < n - 1 ? options[options.length - 1].pop() : undefined;
      if (!next) { seen.delete(key(...here)); path.pop(); options.pop(); continue; }
      seen.add(key(...next)); path.push(next); options.push(around(next));
    }
    const row = Math.floor(n / 2); // only if the search ran out: a straight walk across
    return { n, path: Array.from({ length: n }, (_, x) => [x, row]) };
  }
  D.signalPath = makePath;

  D.acts.signal = (root, ctx) => {
    const { you, other } = ctx;
    let level = 0, at = 0, strikes = 0, pings = 0, done = 0;
    const cache = {};
    const lv = L => cache[L] ||= makePath(ctx.seed, L);
    const guide = L => L % 2, walker = L => 1 - (L % 2);
    const role = h('p', { class: 'role' }), hud = h('div', { class: 'hud' }), grid = h('div', { class: 'grid' });
    const box = h('div', { class: 'actbox sg' }, h('p', { class: 'acttitle', text: D.pick(ctx.title) }), hud, role, grid);
    root.replaceChildren(box);
    let cells = [];

    function render() {
      const { n, path } = lv(level);
      const iGuide = guide(level) === you;
      const here = path[at], goal = path[path.length - 1];
      role.textContent = t(iGuide ? 'sg_guide' : 'sg_walk', { name: ctx.people[other].name });
      hud.replaceChildren(
        h('span', { class: 'pill', text: t('sg_level', { n: level + 1 }) }),
        h('span', { class: 'pill' + (PINGS - pings ? ' hot' : '') }, D.icon('map-pin'), t('sg_pings', { n: PINGS - pings })),
        h('span', { class: 'pill', text: t('sg_strikes', { n: strikes }) }));
      if (cells.length !== n * n) {
        grid.style.gridTemplateColumns = `repeat(${n},1fr)`;
        cells = [];
        for (let y = 0; y < n; y++) for (let x = 0; x < n; x++) {
          const c = h('button', { class: 'cell', type: 'button', 'aria-label': `${x + 1}, ${y + 1}`, onclick: () => tap(x, y) });
          cells.push(c);
        }
        grid.replaceChildren(...cells);
      }
      const onPath = new Map(path.map(([x, y], i) => [y * n + x, i]));
      cells.forEach((c, i) => {
        const x = i % n, y = Math.floor(i / n), step = onPath.get(i);
        const near = Math.abs(x - here[0]) + Math.abs(y - here[1]) === 1;
        c.className = 'cell' + (iGuide && step != null ? ' path' : '') + (!iGuide && step != null && step <= at ? ' trail' : '')
          + (x === here[0] && y === here[1] ? ' here' : '') + (x === goal[0] && y === goal[1] ? ' goal' : '') + (!iGuide && near ? ' can' : '');
        c.style.setProperty('--who', ctx.people[walker(level)].color);
      });
    }

    function tap(x, y) {
      if (!ctx.open()) return;
      const { path } = lv(level), here = path[at];
      if (guide(level) === you) { if (pings < PINGS) ctx.send({ k: 'ping', L: level, x, y }); return; }
      if (Math.abs(x - here[0]) + Math.abs(y - here[1]) !== 1) return;
      ctx.send({ k: 'step', L: level, i: at, x, y });
    }
    const cellAt = (x, y) => cells[y * lv(level).n + x];

    function fit() {
      const r = root.getBoundingClientRect();
      const small = r.height < 430;
      box.classList.toggle('tight', small);
      box.style.setProperty('--side', Math.max(150, Math.min(r.width - 28, r.height - (small ? 96 : 130), 460)) + 'px');
    }
    new ResizeObserver(fit).observe(root);
    fit(); render();

    return {
      onAct(by, d, replay) {
        if (!d || d.L !== level) return;
        const { n, path } = lv(level);
        const inside = Number.isInteger(d.x) && Number.isInteger(d.y) && d.x >= 0 && d.y >= 0 && d.x < n && d.y < n;
        if (!inside) return;
        if (d.k === 'ping' && by === guide(level) && pings < PINGS) {
          pings++;
          render();
          if (!replay) { const ring = h('i', { class: 'ring' }); cellAt(d.x, d.y).append(ring); setTimeout(() => ring.remove(), 2300); D.buzz(12); }
        } else if (d.k === 'step' && by === walker(level) && d.i === at) {
          const next = path[at + 1];
          if (next && next[0] === d.x && next[1] === d.y) {
            at++;
            if (at === path.length - 1) {
              done++; level++; at = 0; pings = 0;
              ctx.score({ kind: 'signal', levels: done, slips: strikes });
              render();
              if (!replay) { D.toast(t('sg_done')); D.confetti(60); D.buzz(60); D.anim(grid, [{ transform: 'scale(.9)', opacity: .3 }, { transform: 'none', opacity: 1 }], 450); }
              return;
            }
            render();
            if (!replay) D.pop(cellAt(d.x, d.y), 6);
          } else {
            strikes++; at = 0;
            render();
            if (!replay) { const c = cellAt(d.x, d.y); c.classList.add('zap'); setTimeout(() => c.classList.remove('zap'), 500); if (by === you) D.toast(t('sg_zap'), 1800); D.buzz([40, 30, 40]); }
          }
        }
      },
    };
  };
})();
