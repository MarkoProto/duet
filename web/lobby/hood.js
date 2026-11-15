/* The admin page: who is here, which rooms are open, and how the parts behind them are doing.
   The technical detail lives here so that the pages people use can stay free of it. */
(() => {
  const { $, h } = D;
  $('brandmark').innerHTML = D.markSvg('beat');
  const STEPS = { create: 'Room created', move: 'Program loaded', start: 'Program started', arm: 'Set up for this pair' };
  // The admin key comes from the address the lobby printed: /hood#key=…
  const key = new URLSearchParams(location.hash.slice(1)).get('key');
  let want = null, asking = false;
  const chip = m => h('span', { class: 'markchip' + (m.done ? ' done' : '') }, m.done ? '✓ ' : h('i', { class: 'spin small' }), (STEPS[m.k] || m.k) + (m.done && m.ms != null ? ' · ' + took(m.ms) : ''));
  const ACT = { wavelength: 'Same Wavelength', sketch: 'Four Hands', signal: 'Signal', thisorthat: 'This or That', story: 'Tale for Two' };
  const short = id => String(id || '').slice(0, 8);
  const took = ms => ms == null ? '–' : ms < 1 ? 'instant' : ms < 1000 ? Math.round(ms) + ' ms' : (ms / 1000).toFixed(1) + ' s';
  const last = {};

  /* The lists are redrawn every second. Each card is made once per id and only its contents change
     after that, so a card's entrance animation plays when it arrives, not every second. */
  function keep(box, items, idOf, fill, empty) {
    if (!items.length) {
      const none = empty();
      if (box.children.length !== 1 || box.firstElementChild.textContent !== none.textContent) box.replaceChildren(none);
      return;
    }
    const old = new Map([...box.children].filter(c => c.dataset.id).map(c => [c.dataset.id, c]));
    const cards = items.map((item, i) => {
      const id = String(idOf(item, i));
      let card = old.get(id);
      if (!card) { card = document.createElement('div'); card.dataset.id = id; }
      fill(card, item);
      return card;
    });
    if (cards.length !== box.children.length || cards.some((c, i) => box.children[i] !== c)) box.replaceChildren(...cards);
  }
  function tile(key, label, value, sub, row = 'tiles') {
    let el = $('tile-' + key);
    if (!el) { el = h('div', { class: 'tile', id: 'tile-' + key }, h('small', { text: label }), h('b'), h('span')); $(row).append(el); }
    const b = el.querySelector('b');
    if (last[key] !== value) { last[key] = value; b.textContent = value; b.classList.remove('bump'); void b.offsetWidth; b.classList.add('bump'); }
    el.querySelector('span').textContent = sub || '';
  }
  /* A timing as "usually X", with the slowest one seen underneath. */
  const usual = (t, div = 1) => {
    if (!t) return ['–', 'not measured yet'];
    const times = 'measured ' + t.n + (t.n === 1 ? ' time' : ' times');
    return [took(t.p50 / div), t.p95 / div < 1 ? times : 'slowest ' + took(t.p95 / div) + ' · ' + times];
  };
  /* "in 0.8 s", or "instantly" when it was too quick to measure. */
  const within = ms => (ms == null ? '' : ms < 1 ? 'instantly' : 'in ' + took(ms));
  const where = r => !r ? 'Being prepared' : r.kind === 'daytona' ? 'Daytona sandbox ' + short(r.id) : 'Running on this computer';
  const faces = pair => pair.map((p, i) => h('span', { class: 'face', vars: { '--who': i ? '#ff4f8b' : '#ffb84d' }, text: p }));

  function draw(d) {
    const daytona = d.sponsors.rooms === 'daytona', ai = d.sponsors.ai === 'nosana';
    $('badges').replaceChildren(
      h('span', { class: 'badge' + (daytona ? ' on' : ''), text: daytona ? 'Rooms run in Daytona sandboxes' + (d.daytona_api ? ', ' + d.daytona_api : '') : 'Rooms run on this computer (no Daytona key)' }),
      h('span', { class: 'badge' + (ai ? ' on' : ''), text: ai ? 'The AI on Nosana reads, plans and writes' : 'Built-in rules only (no Nosana key)' }));
    const live = d.rooms.filter(r => r.phase === 'live').length;
    tile('here', 'People here now', d.people.here, d.people.waiting + ' waiting to be paired');
    tile('rooms', 'Rooms open', d.rooms.length, live + ' in session · ' + (d.rooms.length - live) + ' being prepared');
    tile('done', 'Sessions finished', d.totals.sessions, d.totals.mutual + ' ended with both saying yes');
    tile('later', 'Second rounds planned', d.set_for_later, 'pairs who meet again');
    tile('reports', 'Reports to review', d.totals.reports, 'only these chats are kept');
    const [ready, readySub] = usual(d.timings.room_ready_ms);
    tile('ready', 'To open a room', ready, readySub, 'speed');
    const [model, modelSub] = usual(d.timings.model_ms);
    tile('model', 'For the AI to plan a pair', model, d.timings.model_ms ? modelSub : (ai ? 'not used yet' : 'no AI connected'), 'speed');
    const [pairing, pairingSub] = usual(d.timings.pairing_us, 1000);
    tile('pairing', 'To pair everyone waiting', pairing, pairingSub, 'speed');
    const steps = d.timings.steps || {};
    for (const [k, label] of [['create', 'To create a sandbox'], ['move', 'To load the room program'], ['start', 'To start it']]) {
      const [v, sub] = daytona ? usual(steps[k]) : ['–', ''];
      tile('step-' + k, label, v, !daytona ? 'only for Daytona sandboxes' : steps[k] ? sub : 'not measured yet', 'speed');
    }
    const [gone, goneSub] = usual(d.timings.destroy_ms);
    tile('gone', 'To remove a closed room', gone, goneSub, 'speed');

    // Nosana's three jobs, each with its rules underneath so nobody waits on it.
    const job = (id, label, counts, t) => {
      const [v, sub] = usual(t);
      const n = counts || { model: 0, rules: 0 };
      tile('ai-' + id, label, n.model + n.rules ? v : '–', (n.model + ' by the AI · ' + n.rules + ' by rules') + (t ? ' · ' + sub : ''), 'ai');
    };
    const jobs = d.ai || {};
    job('read', '1 · Reads someone\'s own words into the taps', jobs.reads, d.timings.read_ms);
    job('plan', '2 · Plans the pair: activity, reason, host lines', jobs.plans, d.timings.model_ms);
    job('note', '3 · Writes the note for round two', jobs.notes, d.timings.note_ms);

    drawWarm(d.warm || {}, daytona);

    keep($('rooms'), d.rooms, r => r.id, (card, r) => { card.className = 'room'; card.replaceChildren(
      h('div', { class: 'top' }, faces(r.pair), h('b', { text: ACT[r.activity] || 'Choosing an activity…' }),
        r.long ? h('span', { class: 'markchip done', text: 'second round' }) : null,
        r.by_model ? h('span', { class: 'markchip done', text: 'planned by the AI' }) : null,
        r.warm ? h('span', { class: 'markchip done', text: 'room built ahead' }) : null,
        h('span', { class: 'left', text: r.phase === 'live' ? D.clock(r.left_ms) + ' left' : 'preparing' })),
      r.phase === 'live' ? h('div', { class: 'bar' }, h('i', { style: { transform: `scaleX(${Math.min(1, r.left_ms / (d.session_secs * 1000 * (r.long ? 3 : 1)))})` } })) : null,
      h('div', { class: 'marks' }, r.marks.map(chip)),
      h('span', { class: 'where', text: where(r.runtime) + (r.ready_ms != null ? ' · ready ' + within(r.ready_ms) : '') }));
    }, () => h('div', { class: 'none', text: 'No rooms are open. A room opens when two people are paired.' }));

    keep($('recent'), d.recent, (c, i) => c.id || 'closed-' + i, (card, c) => {
      const r = c.receipt || {};
      card.className = 'room gone';
      card.replaceChildren(
        h('div', { class: 'top' }, h('b', { text: ACT[c.activity] || 'Session' }),
          h('span', { class: 'markchip' + (c.mutual ? ' done' : ''), text: c.mutual == null ? 'the two never met' : c.mutual ? 'both said yes' : 'no second round' }),
          h('span', { class: 'left', text: 'lasted ' + D.clock(c.secs * 1000) })),
        h('span', { class: 'where ' + (r.gone ? 'ok' : 'warn'), text: (r.kind === 'daytona' ? 'Daytona sandbox ' + short(r.id) : 'Room') + ' removed ' + within(r.ms) + (r.gone ? ' · checked: it is gone' : ' · not yet confirmed gone') }));
    }, () => h('div', { class: 'none', text: 'No room has closed yet.' }));
  }

  /* Rooms built ahead: each sandbox with its steps and timings, and the batch they were built in. */
  function drawWarm(w, daytona) {
    if (!w.on) {
      $('batch').textContent = '';
      $('warmctl').replaceChildren();
      return $('warm').replaceChildren(h('div', { class: 'none', text: daytona ? 'Not keeping rooms ahead.' : 'Rooms are built ahead only when they run in Daytona sandboxes.' }));
    }
    const b = w.batch;
    $('batch').textContent = !b ? '' : b.done_ms != null
      ? 'Last batch: ' + b.n + ' sandbox' + (b.n === 1 ? '' : 'es') + ' built at the same time, all done in ' + took(b.done_ms) + (b.failed ? ' · ' + b.failed + ' failed' : '')
      : 'Building ' + b.n + ' sandbox' + (b.n === 1 ? '' : 'es') + ' at the same time… ' + took(b.running_ms);
    if (key) {
      if (want == null) want = w.target;
      const count = h('b', { class: 'want', text: String(want) });
      const nudge = by => h('button', { class: 'mini', type: 'button', text: by > 0 ? '+' : '−', onclick: () => { want = Math.max(0, Math.min(8, want + by)); count.textContent = want; } });
      const go = h('button', { class: 'mini go', type: 'button', text: 'Build them now', onclick: async () => {
        if (asking) return;
        asking = true; go.classList.add('off');
        try {
          const r = await fetch('/api/hood/warm', { method: 'POST', headers: { 'Content-Type': 'application/json', 'x-duet-admin': key }, body: JSON.stringify({ n: want }) });
          if (!r.ok) D.toast(r.status === 403 ? 'The admin key in this address is not right.' : r.status === 409 ? 'Still building the last batch. Try again in a moment.' : 'Could not start the build.');
        } catch { D.toast('No connection to the lobby.'); }
        asking = false;
      } });
      if (!$('warmctl').firstChild || $('warmctl').dataset.n !== String(want)) {
        $('warmctl').dataset.n = String(want);
        $('warmctl').replaceChildren(nudge(-1), count, nudge(1), go);
      }
    }
    const rooms = w.rooms || [];
    keep($('warm'), rooms, (r, i) => r.id || 'new-' + i, (card, r) => { card.className = 'warmcard ' + r.state; card.replaceChildren(
      h('div', { class: 'top' },
        h('b', { text: r.id ? 'Sandbox ' + short(r.id) : 'New sandbox' }),
        h('span', { class: 'markchip' + (r.state === 'ready' ? ' done' : ''), text: r.state === 'ready' ? 'ready' : r.state === 'failed' ? 'failed' : 'building' })),
      h('div', { class: 'marks' }, (r.marks.length ? r.marks : [{ k: 'create', done: false }]).map(chip)),
      h('span', { class: 'where' + (r.state === 'failed' ? ' warn' : r.state === 'ready' ? ' ok' : ''), text:
        r.state === 'failed' ? (r.error || 'failed') :
        r.state === 'ready' ? 'built in ' + took(r.built_ms) + ' · waiting ' + D.clock(r.waiting_ms) :
        'building for ' + took(r.age_ms) }));
    }, () => h('div', { class: 'none', text: w.target ? 'Starting…' : 'None kept ahead right now.' }));
  }

  async function loop() {
    for (;;) {
      try { draw(await (await fetch('/api/hood', { cache: 'no-store' })).json()); } catch {}
      await D.wait(1000);
    }
  }
  loop();
})();
