/* Chat, in Duet's manner: your own on the right, names on the other's, replies, reactions on a
   message, typing dots, join and leave lines, and the host's lines as cards. Held in the page only. */
(() => {
  const { $, h, t } = D;
  const net = D.net;
  const chat = D.chat = {};
  const QUICK = ['❤️', '😂', '😮', '🥹', '👍', '🔥'];
  const PICK = '😀 😄 😁 😆 😅 😂 🤣 🙂 😉 😊 😇 🥰 😍 🤩 😘 😋 😜 🤪 🤗 🤭 🤔 🤨 😐 😶 😏 🙄 😬 😌 😴 🥱 😷 🤒 🥳 😎 🤓 😕 😟 😮 😯 😲 😳 🥺 🥹 😢 😭 😱 😖 😤 😡 🤯 😈 💀 👻 👽 🤖 😺 🙈 🙉 🙊 ❤️ 🧡 💛 💚 💙 💜 🖤 🤍 💔 💕 💖 ✨ ⭐ 🌟 🔥 💯 💥 🎉 🎊 🎈 🎁 👍 👎 👏 🙌 🙏 🤝 👋 ✌️ 🤞 🤟 👌 💪 👀 🧠 🫶 🌸 🌈 ☀️ 🌙 ⚡ ❄️ 🌊 🍀 🐶 🐱 🐼 🦊 🐸 🐧 🦄 🍕 🍜 🍣 🍙 🍰 🍫 ☕ 🍵 🍺 🎮 🎧 🎵 🎬 📚 ✈️ 🗾 🗼 ⚽ 🏀 🎨 🎯 🚀'.split(' ');

  let log, box, people, you, other;
  let atBottom = true, unread = 0, replyTo = null, menuFor = null, typingTimer = 0, lastTyped = 0;
  const byId = new Map();   // message id -> { el, by, text, marks: Map(emoji -> Set(person)) }
  const waiting = new Map(); // my own bubbles not yet echoed: mid -> { el, text, re, timer }

  const name = i => people[i].name;
  const emojiOnly = s => { try { return /^(?:\p{Extended_Pictographic}|\p{Emoji_Component}|‍|️|\s)+$/u.test(s) && !/^[\d\s#*]+$/.test(s) && [...s.matchAll(/\p{Extended_Pictographic}/gu)].length <= 3; } catch { return false; } };

  function scrollDown(smooth) {
    log.scrollTo({ top: log.scrollHeight, behavior: smooth && !D.reduced ? 'smooth' : 'auto' });
    unread = 0; $('latest').hidden = true;
  }
  function added(mine) {
    $('empty')?.remove();
    if (atBottom || mine) requestAnimationFrame(() => scrollDown(true));
    else { unread++; const b = $('latest'); b.hidden = false; b.textContent = t('latest') + ' · ' + unread; }
    if (!mine && document.querySelector('#app.folded')) $('dot').hidden = false;
  }

  /* A message that is nothing but a typed face, a mark or a laugh (<3, :), T_T, haha, hi) becomes a
     small moving picture, as in Duet the watch-party app. It moves for a few seconds when it arrives,
     then settles small and still; one already there when the page loads is settled from the start. */
  const SIGN_MOVES_MS = 4000;
  function bubble(by, text, re, fresh) {
    const mine = by === you;
    const prev = log.lastElementChild;
    const sign = !re && D.signFor ? D.signFor(text) : null;
    const el = h('div', { class: 'm' + (mine ? ' mine' : '') + (prev && prev.classList.contains('m') && +prev.dataset.by === by ? ' same' : '') + (sign ? ' big sign' : emojiOnly(text) ? ' big' : ''), vars: { '--who': people[by].color } });
    el.dataset.by = by;
    if (!mine) el.append(h('span', { class: 'who', text: name(by) }));
    if (re && byId.has(re)) {
      const src = byId.get(re);
      el.append(h('button', { class: 'quote', type: 'button', vars: { '--who': people[src.by].color }, onclick: () => flash(src.el) },
        h('b', { text: src.by === you ? name(you) : name(src.by) }), h('span', { text: src.text })));
    }
    if (sign) {
      const art = h('span', { class: 'signart ' + sign + (fresh ? '' : ' settled'), html: D.signArt(sign) });
      el.append(art, h('span', { class: 'vh', text }));
      if (fresh) setTimeout(() => art.classList.add('settled'), SIGN_MOVES_MS);
    } else el.append(h('span', { class: 'txt', text }));
    return el;
  }
  const flash = el => { el.scrollIntoView({ block: 'center', behavior: D.reduced ? 'auto' : 'smooth' }); el.classList.remove('flash'); void el.offsetWidth; el.classList.add('flash'); };

  function register(el, m) {
    el.dataset.id = m.id;
    byId.set(m.id, { el, by: m.by, text: m.text, marks: new Map() });
  }

  net.on('chat', (m, replay) => {
    const mine = m.by === you;
    const mineWaiting = mine && waiting.get(m.mid);
    if (mineWaiting) {
      clearTimeout(mineWaiting.timer);
      waiting.delete(m.mid);
      mineWaiting.el.classList.remove('pending', 'failed');
      register(mineWaiting.el, m);
      return;
    }
    const el = bubble(m.by, m.text, m.re, !replay);
    if (replay) el.style.animation = 'none';
    register(el, m);
    log.append(el);
    added(mine);
    if (!mine) { stopTyping(); }
  });

  net.on('err', m => {
    const w = m.mid && waiting.get(m.mid);
    if (!w) return;
    if (m.code === 'slow') D.toast(t('slow'));
    fail(m.mid);
  });

  function fail(mid) {
    const w = waiting.get(mid);
    if (!w) return;
    clearTimeout(w.timer);
    w.el.classList.remove('pending');
    w.el.classList.add('failed');
    w.el.dataset.fail = t('not_sent');
    w.el.onclick = () => { w.el.onclick = null; w.el.classList.remove('failed'); w.el.classList.add('pending'); post(mid, w); };
  }
  function post(mid, w) {
    net.send({ t: 'chat', mid, text: w.text, re: w.re || undefined });
    w.timer = setTimeout(() => fail(mid), 9000);
  }

  function say(text) {
    text = text.trim();
    if (!text || net.phase === 'over') return;
    const mid = net.mid();
    const el = bubble(you, text.slice(0, 600), replyTo, true);
    el.classList.add('pending');
    log.append(el);
    const w = { el, text, re: replyTo };
    waiting.set(mid, w);
    post(mid, w);
    setReply(null);
    added(true);
  }

  /* Reactions on a message sit half over its bottom edge. */
  function paint(entry) {
    let row = entry.el.querySelector('.rxs');
    if (!entry.marks.size) { row?.remove(); entry.el.classList.remove('hasrx'); return; }
    if (!row) { row = h('div', { class: 'rxs' }); entry.el.append(row); entry.el.classList.add('hasrx'); }
    const had = new Set([...row.children].map(c => c.dataset.e));
    row.replaceChildren(...[...entry.marks].map(([e, who]) => {
      const b = h('button', { class: 'rx' + (who.has(you) ? ' on' : '') + (had.has(e) ? '' : ' fresh'), type: 'button', text: who.size > 1 ? e + ' ' + who.size : e,
        onclick: ev => { ev.stopPropagation(); mark(entry.el.dataset.id, e, !who.has(you)); } });
      b.dataset.e = e;
      return b;
    }));
  }
  const mark = (id, e, on) => net.send({ t: 'mreact', id, e, on });
  net.on('mreact', m => {
    const entry = byId.get(m.id);
    if (!entry) return;
    const who = entry.marks.get(m.e) || new Set();
    if (m.on) who.add(m.by); else who.delete(m.by);
    if (who.size) entry.marks.set(m.e, who); else entry.marks.delete(m.e);
    paint(entry);
    if (atBottom) requestAnimationFrame(() => scrollDown(false));
  });

  /* Notices clear themselves after a few seconds: the chat is for the two people. Old ones are not
     shown again after a reload. */
  const fleeting = (el, ms) => setTimeout(() => { el.classList.add('going'); setTimeout(() => el.remove(), 450); }, ms);
  const line = (text, replay) => { if (replay) return; const el = h('div', { class: 'sysline', text }); log.append(el); added(false); fleeting(el, 5000); };
  net.on('sys', (m, replay) => {
    if (m.by === you && m.k !== 'left') return;
    if (m.k === 'join') line(t('joined', { name: name(m.by) }), replay);
    else if (m.k === 'away') line(t('away', { name: name(m.by) }), replay);
    else if (m.k === 'back') line(t('back', { name: name(m.by) }), replay);
    else if (m.k === 'left' && m.by !== you) line(t('left', { name: name(m.by) }), replay);
  });
  net.on('host', (m, replay) => {
    if (replay) return;
    const el = h('div', { class: 'hostcard' }, h('small', { text: t('host') }), h('p', { text: D.pick(m.text) }));
    log.append(el);
    added(false);
    fleeting(el, 8000);
  });

  /* Typing dots */
  function stopTyping() { clearTimeout(typingTimer); $('typing').replaceChildren(); }
  net.on('live', m => {
    if (m.k !== 'typing') return;
    if (!m.d) return stopTyping();
    $('typing').replaceChildren(h('i'), h('i'), h('i'), ' ' + t('typing', { name: name(m.by) }));
    clearTimeout(typingTimer);
    typingTimer = setTimeout(stopTyping, 3500);
  });

  function setReply(id) {
    replyTo = id && byId.has(id) ? id : null;
    const bar = $('replybar');
    bar.hidden = !replyTo;
    if (replyTo) {
      const src = byId.get(replyTo);
      bar.style.setProperty('--who', people[src.by].color);
      bar.querySelector('b').textContent = name(src.by);
      bar.querySelector('span').textContent = src.text;
      box.focus();
    }
  }

  /* The menu on a message: quick reactions, reply, copy. Right click, or a long press. */
  function openMenu(el, x, y) {
    const id = el.dataset.id;
    if (!id) return;
    menuFor = id;
    const entry = byId.get(id), menu = $('msgmenu');
    $('msgemoji').replaceChildren(...QUICK.map(e => {
      const on = entry.marks.get(e)?.has(you);
      return h('button', { class: 'emo' + (on ? ' on' : ''), type: 'button', text: e, onclick: () => { mark(id, e, !on); closeMenu(); } });
    }));
    menu.hidden = false;
    const r = menu.getBoundingClientRect();
    menu.style.left = Math.max(8, Math.min(x - r.width / 2, innerWidth - r.width - 8)) + 'px';
    menu.style.top = Math.max(8, Math.min(y - r.height - 10, innerHeight - r.height - 8)) + 'px';
    D.buzz(8);
  }
  const closeMenu = () => { $('msgmenu').hidden = true; menuFor = null; };

  function openPicker() {
    const p = $('picker');
    if (!p.hidden) { p.hidden = true; return; }
    if (!p.children.length) p.append(...PICK.map(e => h('button', { class: 'emo', type: 'button', text: e, onclick: () => {
      const at = box.selectionStart ?? box.value.length;
      box.setRangeText(e, at, box.selectionEnd ?? at, 'end');
      fit();
      if (!D.touch) box.focus();
    } })));
    p.hidden = false;
    const r = $('emo').getBoundingClientRect(), pr = p.getBoundingClientRect();
    p.style.left = Math.max(8, Math.min(r.left, innerWidth - pr.width - 8)) + 'px';
    p.style.top = Math.max(8, r.top - pr.height - 8) + 'px';
  }

  /* The paper plane leaves, and a new one slides in. */
  const fly = () => D.anim($('send').firstElementChild, [{ transform: 'none', opacity: 1 }, { transform: 'translate(16px,-12px) scale(.6)', opacity: 0, offset: 0.45 }, { transform: 'translate(-12px,10px) scale(.6)', opacity: 0, offset: 0.46 }, { transform: 'none', opacity: 1 }], { duration: 460, easing: 'ease-out', fill: 'none' });
  const fit = () => { box.style.height = '41px'; box.style.height = Math.min(125, box.scrollHeight + 2) + 'px'; };

  chat.init = room => {
    people = room.people; you = net.you; other = 1 - you;
    log = $('log'); box = $('t');
    $('empty').textContent = t('empty_chat');
    box.placeholder = t('chat_placeholder', { name: name(other) });
    $('msgreply').textContent = t('reply');
    $('msgcopy').textContent = t('copy');
    $('where').textContent = t('just_two');

    log.addEventListener('scroll', () => { atBottom = log.scrollHeight - log.scrollTop - log.clientHeight < 60; if (atBottom) { unread = 0; $('latest').hidden = true; } }, { passive: true });
    $('latest').onclick = () => scrollDown(true);
    new ResizeObserver(() => { if (atBottom) scrollDown(false); }).observe(log);

    $('f').addEventListener('submit', e => { e.preventDefault(); if (box.value.trim()) fly(); say(box.value); box.value = ''; fit(); net.send({ t: 'live', k: 'typing', d: false }); lastTyped = 0; });
    box.addEventListener('keydown', e => { if (e.key === 'Enter' && !e.shiftKey && !e.altKey && !(e.isComposing || e.keyCode === 229)) { e.preventDefault(); $('f').requestSubmit(); } });
    box.addEventListener('input', () => {
      fit();
      const now = Date.now();
      if (box.value && now - lastTyped > 2000) { lastTyped = now; net.send({ t: 'live', k: 'typing', d: true }); }
    });
    // Pressing Send must not take the focus away: on a phone that would close the keyboard, move the
    // page under the finger, and lose the tap.
    $('send').addEventListener('pointerdown', e => e.preventDefault());
    $('emo').onclick = e => { e.stopPropagation(); openPicker(); };
    $('replyclose').onclick = () => setReply(null);

    log.addEventListener('contextmenu', e => { const el = e.target.closest('.m'); if (el) { e.preventDefault(); openMenu(el, e.clientX, e.clientY); } });
    let press = 0, startX = 0, startY = 0;
    log.addEventListener('pointerdown', e => {
      if (e.pointerType !== 'touch') return;
      const el = e.target.closest('.m'); if (!el) return;
      startX = e.clientX; startY = e.clientY;
      press = setTimeout(() => openMenu(el, startX, startY), 450);
    });
    log.addEventListener('pointermove', e => { if (press && Math.hypot(e.clientX - startX, e.clientY - startY) > 10) { clearTimeout(press); press = 0; } });
    for (const ev of ['pointerup', 'pointercancel']) log.addEventListener(ev, () => { clearTimeout(press); press = 0; });
    // A double click gives a heart. It should not also select the word under the pointer.
    log.addEventListener('mousedown', e => { if (e.detail > 1 && e.target.closest('.m')) e.preventDefault(); });
    log.addEventListener('dblclick', e => { const el = e.target.closest('.m'); if (el?.dataset.id) { const entry = byId.get(el.dataset.id); mark(el.dataset.id, '❤️', !entry.marks.get('❤️')?.has(you)); } });
    $('msgreply').onclick = () => { setReply(menuFor); closeMenu(); };
    $('msgcopy').onclick = async () => { const entry = byId.get(menuFor); closeMenu(); try { await navigator.clipboard.writeText(entry.text); D.toast(t('copied')); } catch {} };
    document.addEventListener('pointerdown', e => {
      if (!$('msgmenu').hidden && !e.target.closest('#msgmenu')) closeMenu();
      if (!$('picker').hidden && !e.target.closest('#picker') && !e.target.closest('#emo')) $('picker').hidden = true;
    });
    document.addEventListener('keydown', e => { if (e.key === 'Escape') { closeMenu(); $('picker').hidden = true; setReply(null); } });
  };

  chat.close = () => { box.disabled = true; $('send').disabled = true; stopTyping(); };
})();
