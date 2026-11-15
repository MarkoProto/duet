/* Reactions that float up the stage, hold-to-burst, and the nudge. Straight from Duet. */
(() => {
  const { $, h, t } = D;
  const net = D.net;
  const QUICK = ['😂', '😮', '🥹', '🔥', '❤️', '👏', '👀', '✨'];

  function float(emoji, count, who) {
    const stage = $('burst');
    for (let i = 0; i < Math.min(count || 1, 12); i++) setTimeout(() => {
      if (stage.childElementCount >= 40) return;
      const e = h('div', { class: 'float', text: emoji, style: { left: 6 + Math.random() * 84 + '%' }, vars: { '--r': (Math.random() * 40 - 20) + 'deg' } });
      if (who && !i) e.append(h('small', { text: who }));
      e.onanimationend = () => e.remove();
      stage.append(e);
    }, i * 90);
  }

  function react(emoji, count) {
    float(emoji, count);
    net.send({ t: 'live', k: 'float', d: { e: emoji, n: count } });
  }

  /* Hold for a burst. A finger waits a moment first, because it may be swiping along the row. */
  function holdable(b, emoji) {
    let timer = 0, waiting = 0, n = 0;
    const stop = () => { clearInterval(timer); clearTimeout(waiting); timer = 0; waiting = 0; };
    const begin = () => { waiting = 0; n = 1; react(emoji, 1); timer = setInterval(() => { if (++n > 20) return stop(); react(emoji, 2); }, 280); };
    b.addEventListener('pointerdown', e => { if (e.pointerType === 'touch') waiting = setTimeout(begin, 180); else begin(); });
    b.addEventListener('pointerup', () => { if (waiting) { stop(); react(emoji, 1); } else stop(); });
    for (const ev of ['pointerleave', 'pointercancel']) b.addEventListener(ev, stop);
    b.addEventListener('keydown', e => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); react(emoji, 1); } });
    b.addEventListener('contextmenu', e => e.preventDefault());
  }

  function nudged(who) {
    const s = $('stage');
    s.classList.remove('shake'); void s.offsetWidth; s.classList.add('shake');
    D.toast(t('nudged', { name: who }));
    D.buzz([30, 40, 30]);
  }

  D.fx = {
    float,
    big(text) { const b = $('big'); b.hidden = false; b.replaceChildren(h('span', { text })); clearTimeout(b._t); b._t = setTimeout(() => { b.hidden = true; }, 900); },
    init(room) {
      const row = $('reactrow');
      for (const emoji of QUICK) { const b = h('button', { class: 'emo', type: 'button', text: emoji }); holdable(b, emoji); row.append(b); }
      row.append(h('button', { class: 'action', type: 'button', onclick: e => { net.send({ t: 'live', k: 'nudge', d: 1 }); D.jelly(e.currentTarget); D.pop(e.currentTarget, 6); D.toast(t('nudge_sent'), 1400); } }, D.icon('hand'), h('span', { text: t('nudge') })));
      net.on('live', m => {
        if (m.k === 'float' && m.d && typeof m.d.e === 'string') float(m.d.e.slice(0, 16), +m.d.n || 1, room.people[m.by].name);
        else if (m.k === 'nudge') nudged(room.people[m.by].name);
      });
    },
  };
})();
