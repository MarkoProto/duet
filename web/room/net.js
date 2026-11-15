/* The line to the room.
   A WebSocket when one can open, held requests when it cannot. Either way: every stored event has a
   number, a returning page asks for what came after the last one it saw, and anything sent with an
   id is re-sent until the room has echoed it, so nothing is lost and nothing happens twice. */
(() => {
  const net = D.net = { seq: 0, la: 0, you: null, room: null, phase: 'connecting', starts: null, ends: null, why: null, by: null, pres: [false, false], online: false, mode: 'ws' };
  const handlers = {};
  const base = new URL('.', location.href);
  let key = '';
  const found = location.hash.match(/k=([\w-]+)/);
  // A room reached through the lobby says so with p=1: it takes held requests, not a socket.
  const viaLobby = /(^#|&)p=1(&|$)/.test(location.hash);
  try {
    if (found) { key = found[1]; sessionStorage.setItem('duet-k', key); history.replaceState(null, '', location.pathname + location.search); }
    else key = sessionStorage.getItem('duet-k') || '';
  } catch { key = found ? found[1] : ''; }
  try {
    if (viaLobby || sessionStorage.getItem('duet-p') === '1') { net.mode = 'poll'; sessionStorage.setItem('duet-p', '1'); }
  } catch { if (viaLobby) net.mode = 'poll'; }

  net.on = (type, fn) => { (handlers[type] ||= []).push(fn); };
  const emit = (type, msg, replay) => { for (const fn of handlers[type] || []) { try { fn(msg, replay); } catch (e) { console.error(e); } } };

  /* The room's clock. The quickest of the last few round trips is the most honest one. */
  let offset = 0;
  const samples = [];
  const sync = (serverNow, rtt) => {
    samples.push({ rtt, off: serverNow + rtt / 2 - performance.now() });
    if (samples.length > 8) samples.shift();
    offset = samples.reduce((a, c) => (c.rtt < a.rtt ? c : a)).off;
  };
  net.now = () => performance.now() + offset;
  net.left = () => (net.ends == null ? null : net.ends - net.now());

  /* Things sent with an id wait here until the room echoes them. */
  const pending = new Map();
  const later = [];
  let n = 0;
  net.mid = () => (net.you ?? 'x') + '-' + Date.now().toString(36) + '-' + (n++).toString(36);

  function take(msg, replay) {
    if (msg.s != null) {
      if (msg.s <= net.seq) return;
      net.seq = msg.s;
    }
    if (msg.t === 'pong') return sync(msg.now, performance.now() - msg.c);
    if (msg.t === 'phase') {
      net.phase = msg.p;
      if (msg.starts != null) net.starts = msg.starts;
      if (msg.ends != null) net.ends = msg.ends;
      if (msg.p === 'over') { net.why = msg.why; net.by = msg.by; }
    }
    if (msg.t === 'pres') net.pres = msg.on;
    if (msg.mid && msg.by === net.you) pending.delete(msg.mid);
    if (msg.t === 'err' && msg.mid) pending.delete(msg.mid);
    emit(msg.t, msg, replay);
  }

  function hello(h) {
    const first = net.room == null;
    Object.assign(net, { you: h.you, room: h.room, phase: h.phase, starts: h.starts, ends: h.ends, why: h.why, by: h.by, la: h.la, pres: h.pres });
    sync(h.now, 400);
    if (first) emit('hello', h);
    for (const ev of h.log) take(ev, true);
    emit('ready', h, !first);
    for (const msg of pending.values()) raw(msg);
    for (const msg of later.splice(0)) raw(msg);
    for (const ms of [200, 900, 1800]) setTimeout(ping, ms);
  }

  const ping = () => { if (net.online) raw({ t: 'ping', c: performance.now() }); };
  setInterval(ping, 10000);
  // Pings are answered within seconds; a socket silent for 25 seconds is dead, whatever it says.
  setInterval(() => {
    if (net.mode === 'ws' && ws && ws.readyState === 1 && lastHeard && performance.now() - lastHeard > 25000) {
      lastHeard = 0;
      ws.close();
    }
  }, 5000);

  let ws = null, tries = 0, wsFails = 0, closed = false, outbox = [], flushTimer = 0, pollRun = 0, sending = false, sendFails = 0, lastHeard = 0;

  function raw(msg) {
    if (net.mode === 'ws') {
      if (ws && ws.readyState === 1) ws.send(JSON.stringify(msg));
    } else {
      outbox.push(msg);
      if (!flushTimer) flushTimer = setTimeout(flush, 60);
    }
  }

  /* Send. Momentary things are dropped while offline; the rest waits for the line to come back. */
  net.send = msg => {
    if (msg.mid) pending.set(msg.mid, msg);
    if (net.online) raw(msg);
    else if (!msg.mid && msg.t !== 'live' && msg.t !== 'ping') later.push(msg);
  };

  const setOnline = on => { if (net.online !== on) { net.online = on; emit('online', on); } };

  function retry() {
    if (closed) return;
    const wait = Math.min(8000, 400 * 2 ** tries++) * (0.6 + Math.random() * 0.8);
    setTimeout(connect, wait);
  }

  function connect() {
    if (closed) return;
    if (!key) { closed = true; return emit('denied'); }
    if (net.mode === 'poll') return pollStart();
    const url = new URL('ws', base);
    url.protocol = base.protocol === 'https:' ? 'wss:' : 'ws:';
    url.search = '?k=' + key + '&after=' + net.seq;
    let opened = false;
    const s = ws = new WebSocket(url);
    const giveUp = setTimeout(() => { if (!opened) s.close(); }, 5000);
    s.onmessage = e => {
      lastHeard = performance.now();
      let msg;
      try { msg = JSON.parse(e.data); } catch { return; }
      if (msg.t === 'hello') { opened = true; clearTimeout(giveUp); tries = 0; wsFails = 0; setOnline(true); hello(msg); }
      else take(msg);
    };
    s.onclose = () => {
      clearTimeout(giveUp);
      if (ws !== s) return;
      ws = null;
      setOnline(false);
      // Never opened, twice in a row: something between us and the room refuses sockets.
      if (!opened && ++wsFails >= 2) net.mode = 'poll';
      retry();
    };
  }

  async function pollStart() {
    const run = ++pollRun;
    try {
      const r = await fetch(new URL('hello?k=' + key + '&after=' + net.seq, base), { cache: 'no-store' });
      if (r.status === 403) { closed = true; return emit('denied'); }
      if (r.status === 404 && net.room != null) return gone();
      if (!r.ok) throw new Error('no room');
      const h = await r.json();
      tries = 0; setOnline(true); hello(h);
    } catch { setOnline(false); if (tries > 10 && net.room == null) { closed = true; return emit('denied'); } return retry(); }
    while (!closed && run === pollRun) {
      try {
        const r = await fetch(new URL('poll?k=' + key + '&after=' + net.seq + '&la=' + net.la + '&hold=25', base), { cache: 'no-store' });
        if (r.status === 404) return gone();
        if (!r.ok) throw new Error('poll');
        const p = await r.json();
        if (p.la) net.la = p.la;
        for (const ev of p.ev || []) take(ev);
        for (const lv of p.lv || []) take(lv);
      } catch { setOnline(false); return retry(); }
    }
  }

  /* Held-request mode sends in batches, one batch at a time so they arrive in the order they were
     made. A batch that fails goes back to the front and is tried again, so an answer, a move or a
     report is never lost to one bad request. */
  async function flush() {
    flushTimer = 0;
    if (sending || closed) return;
    const batch = outbox.splice(0, 64);
    if (!batch.length) return;
    sending = true;
    let ok = false;
    try {
      const r = await fetch(new URL('send?k=' + key, base), { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(batch) });
      if (r.status === 404) return gone();
      if (!r.ok) throw new Error('send');
      const out = await r.json();
      for (const reply of out.r || []) take(reply);
      ok = true;
      sendFails = 0;
    } catch {
      outbox.unshift(...batch.filter(m => m.t !== 'ping' && m.t !== 'live'));
      sendFails++;
    } finally {
      sending = false;
    }
    if (outbox.length && !closed) flushTimer = setTimeout(flush, ok ? 60 : Math.min(4000, 300 * 2 ** sendFails));
  }

  /* The room is not there any more (it ended and was removed): back to the lobby, which knows how
     the session ended, instead of trying again for ever. */
  function gone() {
    if (closed) return;
    closed = true;
    pollRun++;
    setOnline(false);
    emit('gone');
  }

  net.connect = connect;
  net.close = () => { closed = true; pollRun++; if (ws) { const s = ws; ws = null; s.close(); } setOnline(false); };

  document.addEventListener('visibilitychange', () => {
    if (!document.hidden && !net.online && !closed) { tries = 0; }
  });
})();
