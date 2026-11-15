/* The room page: who is here, the clock, the opening and closing scenes, and the menu. */
(() => {
  const { $, h, t } = D;
  const net = D.net;
  D.fitViewport();
  let room = null, act = null, opened = false, ended = false, warned = false, lastBig = -1, clockTimer = 0;
  // Changing the game: the activities played in this room, and a suggestion waiting for an answer.
  const played = new Set();
  let offer = null;

  const you = () => net.you, other = () => 1 - net.you;
  const name = i => room.people[i].name;

  function modal(...kids) { $('modalbox').replaceChildren(...kids); $('modal').hidden = false; }
  const closeModal = () => { $('modal').hidden = true; };
  $('modal').addEventListener('pointerdown', e => { if (e.target === $('modal')) closeModal(); });

  /* The activity on the stage. A switch both agreed to puts another one there; whatever the old one
     was doing is left behind, and the new one starts fresh on both screens. */
  function start(a) {
    const make = D.acts[a.kind];
    const ctx = {
      you: you(), other: other(), people: room.people, data: a.data, seed: a.seed >>> 0, title: a.title,
      open: () => net.phase === 'live' || net.phase === 'ending',
      send: d => net.send({ t: 'act', mid: net.mid(), d }),
      seal: (slot, v) => net.send({ t: 'seal', slot, v }),
      live: (k, d) => net.send({ t: 'live', k, d }),
      score: d => net.send({ t: 'score', d }),
    };
    act = make ? make($('act'), ctx) : null;
    played.add(a.kind);
    teach(a);
  }

  /* How to play: two short screens over the game before it starts, read at each person's own pace.
     Shown once per game in a room; a reload does not show it again. */
  const TEACH = {
    wavelength: ['tu_wl_1', 'tu_wl_2'],
    thisorthat: ['tu_tt_1', 'tu_tt_2'],
    sketch: ['tu_sk_1', 'tu_sk_2'],
    story: ['tu_st_1', 'tu_st_2'],
    signal: ['tu_sg_1', 'tu_sg_2'],
  };
  function teach(a) {
    const steps = TEACH[a.kind];
    const seen = 'duet-tut-' + room.id + '-' + a.kind;
    try { if (!steps || sessionStorage.getItem(seen) || sessionStorage.getItem('duet-tut-off')) return; } catch { if (!steps) return; }
    let i = 0;
    const text = h('p', { class: 'tuttext' });
    const dots = h('div', { class: 'tutdots' }, steps.map(() => h('i')));
    const label = h('span');
    const next = h('button', { class: 'btn', type: 'button' }, label, D.icon('arrow-right'));
    const layer = h('div', { class: 'tut' }, h('div', { class: 'tutcard' },
      h('small', { text: t('tu_how') }), h('h2', { text: D.pick(a.title) }), text, dots, next));
    const show = () => {
      text.textContent = t(steps[i], { name: name(other()) });
      [...dots.children].forEach((d, j) => d.classList.toggle('now', j === i));
      label.textContent = t(i === steps.length - 1 ? 'tu_start' : 'tu_next');
      D.anim(text, [{ opacity: 0, transform: 'translateY(6px)' }, { opacity: 1, transform: 'none' }], 260);
    };
    next.onclick = e => {
      D.jelly(e.currentTarget);
      if (++i < steps.length) return show();
      try { sessionStorage.setItem(seen, '1'); } catch {}
      layer.classList.add('going');
      setTimeout(() => layer.remove(), 220);
    };
    show();
    $('act').append(layer);
  }

  function build() {
    document.title = 'Duet · ' + name(other());
    $('brand').innerHTML = D.markSvg('beat');
    $('pair').replaceChildren(D.face(name(0), room.people[0].color), D.face(name(1), room.people[1].color), h('span', { class: 'names', text: name(you()) + ' & ' + name(other()) }));
    D.chat.init(room);
    D.fx.init(room);
    start(room.activity);

    $('toggle').onclick = () => { $('app').classList.toggle('folded'); $('dot').hidden = true; };
    $('more').onclick = e => { e.stopPropagation(); const m = $('menu'); m.hidden = !m.hidden; };
    document.addEventListener('pointerdown', e => { if (!$('menu').hidden && !e.target.closest('#menu') && !e.target.closest('#more')) $('menu').hidden = true; });
    $('menu').replaceChildren(
      h('div', { class: 'seg' }, ...[['en', 'English'], ['ja', '日本語']].map(([code, label]) => h('button', { type: 'button', class: D.lang === code ? 'now' : '', text: label,
        onclick: () => { try { sessionStorage.setItem('duet-room-lang', code); } catch {} location.reload(); } }))),
      h('button', { type: 'button', onclick: askSwitch }, D.icon('shuffle'), h('span', { text: t('menu_switch') })),
      h('button', { type: 'button', onclick: askLeave }, D.icon('log-out'), h('span', { text: t('menu_leave') })),
      h('button', { type: 'button', class: 'warn', onclick: askReport }, D.icon('flag'), h('span', { text: t('menu_report') })));

    $('veilfaces').replaceChildren(D.face(name(you()), room.people[you()].color, true), h('span', { class: 'amp', text: '+' }), h('span', { class: 'ghost' }));
    $('veiltext').textContent = t('waiting_for', { name: name(other()) });
    $('veilsub').textContent = t('waiting_sub');
  }

  function askLeave() {
    $('menu').hidden = true;
    modal(h('h2', { text: t('leave_q') }), h('p', { text: t('leave_sub') }),
      h('div', { class: 'row' }, h('button', { class: 'btn quiet', type: 'button', text: t('stay'), onclick: closeModal }),
        h('button', { class: 'btn danger', type: 'button', text: t('leave_yes'), onclick: () => { closeModal(); net.send({ t: 'leave' }); } })));
  }

  /* Changing the game: one suggests, the other decides. Both screens switch together, and a game
     already played in this room is not offered again. */
  const gameName = kind => { const a = kind === room.activity.kind ? room.activity : (room.alternatives || []).find(x => x.kind === kind); return a ? D.pick(a.title) : kind; };
  const left = () => (room.alternatives || []).filter(a => !played.has(a.kind));
  const swap = (k, to) => net.send({ t: 'act', mid: net.mid(), d: { k, to } });
  function askSwitch() {
    $('menu').hidden = true;
    if (!ctxOpen()) return;
    const games = left();
    if (!games.length) return D.toast(t('sw_none'));
    modal(h('h2', { text: t('sw_title') }), h('p', { text: t('sw_pick', { name: name(other()) }) }),
      h('div', { class: 'games' }, games.map(a => h('button', { class: 'game', type: 'button', onclick: () => {
        closeModal();
        swap('swap-ask', a.kind);
        D.toast(t('sw_sent', { name: name(other()) }), 3000);
      } }, h('b', { text: D.pick(a.title) }), h('span', { text: t('g_' + a.kind) })))),
      h('div', { class: 'row' }, h('button', { class: 'btn quiet', type: 'button', text: t('cancel'), onclick: closeModal })));
  }
  function onSwap(by, d, replay) {
    if (d.k === 'swap-ask') {
      if (!left().some(a => a.kind === d.to)) return;
      offer = { by, to: d.to };
      if (by === other() && !replay && !ended) {
        modal(h('h2', { text: t('sw_ask', { name: name(by), game: gameName(d.to) }) }), h('p', { text: t('g_' + d.to) }),
          h('div', { class: 'row' },
            h('button', { class: 'btn quiet', type: 'button', text: t('sw_no'), onclick: () => { closeModal(); swap('swap-no', d.to); } }),
            h('button', { class: 'btn', type: 'button', text: t('sw_yes'), onclick: () => { closeModal(); swap('swap-yes', d.to); } })));
      }
    } else if (d.k === 'swap-yes' && offer && offer.to === d.to && by !== offer.by) {
      const next = left().find(a => a.kind === d.to);
      offer = null;
      if (!next) return;
      start(next);
      if (!replay) { D.toast(t('sw_done', { game: D.pick(next.title) }), 3000); D.buzz(30); }
    } else if (d.k === 'swap-no' && offer && by !== offer.by) {
      if (!replay && offer.by === you()) D.toast(t('sw_kept', { name: name(by) }), 3500);
      offer = null;
    }
  }
  const ctxOpen = () => net.phase === 'live' || net.phase === 'ending';

  function askReport() {
    $('menu').hidden = true;
    let reason = '';
    const send = h('button', { class: 'btn danger off', type: 'button', text: t('report_send'), onclick: () => {
      if (!reason) return;
      net.send({ t: 'report', reason, note: note.value });
      closeModal();
    } });
    const chips = ['rude', 'uncomfortable', 'scam', 'other'].map(r => h('button', { class: 'chip', type: 'button', text: t('r_' + r), onclick: e => {
      reason = r; chips.forEach(c => c.classList.remove('now')); e.currentTarget.classList.add('now'); send.classList.remove('off');
    } }));
    const note = h('textarea', { maxlength: 500, placeholder: t('report_note') });
    modal(h('h2', { text: t('report_title', { name: name(other()) }) }), h('p', { text: t('report_sub', { name: name(other()) }) }),
      h('div', { class: 'chips' }, chips), note,
      h('div', { class: 'row' }, h('button', { class: 'btn quiet', type: 'button', text: t('cancel'), onclick: closeModal }), send));
  }

  /* Both in: say hi, then the stage comes forward. A reload in the middle skips the greeting. */
  function open(withHello) {
    if (opened) return;
    opened = true;
    const app = $('app'), veil = $('veil');
    const lift = () => { app.classList.remove('gated'); app.classList.add('entering'); setTimeout(() => { veil.hidden = true; app.classList.remove('entering'); }, 1100); };
    if (!withHello) { veil.hidden = true; app.classList.remove('gated'); }
    else {
      $('hello').textContent = t('hi', { name: name(other()) });
      veil.classList.add('hi');
      setTimeout(() => { veil.classList.add('through'); lift(); }, D.reduced ? 300 : 1300);
    }
    $('clock').hidden = false;
    tick();
    clearInterval(clockTimer);
    clockTimer = setInterval(tick, 250);
  }

  function tick() {
    const left = net.left();
    if (left == null || ended) return;
    $('time').textContent = D.clock(left);
    document.querySelector('#clock .arc').style.strokeDashoffset = 100 * (1 - Math.max(0, Math.min(1, left / room.session_ms)));
    const last = left <= Math.min(60000, room.session_ms / 3);
    $('clock').classList.toggle('last', last);
    const secs = Math.ceil(left / 1000);
    if (secs <= 5 && secs >= 1 && secs !== lastBig) { lastBig = secs; D.fx.big(String(secs)); }
  }

  function finish(why, by) {
    if (ended) return;
    ended = true;
    clearInterval(clockTimer);
    closeModal(); $('menu').hidden = true;
    if (room) D.chat.close();
    const text = why === 'time' ? t('over_time')
      : why === 'left' ? (by === you() ? t('over_you_left') : t('over_left', { name: name(by) }))
      : why === 'gone' ? (by === you() ? t('over_ended') : t('over_gone', { name: name(by) }))
      : why === 'noshow' ? t('over_noshow', { name: name(other()) })
      : t('over_ended');
    $('donetext').textContent = text;
    $('donesub').textContent = t('closing');
    $('done').hidden = false;
    const back = $('doneback');
    back.href = room.return_url || '/';
    back.textContent = t('going_back');
    setTimeout(() => net.close(), 1500);
    setTimeout(() => { back.hidden = false; }, 2400);
    setTimeout(() => location.replace(back.href), 4600);
  }

  net.on('gone', () => {
    if (ended || !room) return;
    finish('ended', null);
  });
  net.on('denied', () => {
    $('veil').hidden = true;
    $('donetext').textContent = t('bad_link');
    $('donesub').textContent = t('bad_link_sub');
    $('done').hidden = false;
  });
  net.on('hello', hello => {
    room = hello.room;
    let lang = room.people[hello.you].lang;
    try { lang = sessionStorage.getItem('duet-room-lang') || lang; } catch {}
    D.lang = lang === 'ja' ? 'ja' : 'en';
    document.documentElement.lang = D.lang;
    build();
    const faces = document.querySelectorAll('#pair .face');
    (hello.pres || []).forEach((here, i) => faces[i]?.classList.toggle('away', !here));
  });
  net.on('ready', (_, again) => {
    // The second person to arrive is already "live" in their greeting: they get the hello too.
    if (net.phase === 'live' || net.phase === 'ending') open(!again && net.now() - net.starts < 4000);
    if (net.phase === 'over') finish(net.why, net.by);
  });
  net.on('phase', (m, replay) => {
    if (replay) return; // 'ready' settles the screen once the whole replay is in
    if (m.p === 'live') open(true);
    else if (m.p === 'ending' && !warned) {
      // The last stretch is a minute in a full session, less in a short one: say what is really left.
      warned = true;
      const secs = Math.round((net.left() ?? 60000) / 1000);
      D.toast(secs >= 55 ? t('minute_left') : t('secs_left', { n: Math.max(1, secs) }), 3500);
      D.buzz(50);
    }
    else if (m.p === 'over') finish(m.why, m.by);
  });
  net.on('online', on => {
    const n = $('net');
    n.hidden = on || ended;
    n.querySelector('span').textContent = t('reconnecting');
  });
  net.on('pres', m => {
    const faces = document.querySelectorAll('#pair .face');
    m.on.forEach((here, i) => faces[i]?.classList.toggle('away', !here));
  });
  net.on('act', (m, replay) => {
    if (m.d && typeof m.d.k === 'string' && m.d.k.startsWith('swap-')) return onSwap(m.by, m.d, replay);
    act?.onAct?.(m.by, m.d, replay);
  });
  net.on('sealed', m => act?.onSealed?.(m.by, m.slot));
  net.on('reveal', (m, replay) => act?.onReveal?.(m.slot, m.v, replay));
  net.on('live', m => act?.onLive?.(m.by, m.k, m.d));

  net.connect();
})();
