/* The lobby page. The server decides which screen a person is on; this draws it and animates the
   change. One request is always held open, and is answered the moment the screen should change. */
(() => {
  const { $, h, t } = D;
  D.fitViewport();
  let view = null, ver = 0, polling = false, shown = null, cur = null, gen = 0, ticker = 0, skew = 0;
  let whyAt = 0; // when the reason for the pair first showed, so it is never cut short
  const cameBack = location.hash === '#back';
  if (cameBack) { history.replaceState(null, '', location.pathname); document.documentElement.classList.add('quick'); }
  try { if (sessionStorage.getItem('duet-seen')) document.documentElement.classList.add('quick'); sessionStorage.setItem('duet-seen', '1'); } catch {}

  $('splashmark').innerHTML = D.markSvg('draw');
  $('brandmark').innerHTML = D.markSvg('');
  const now = () => Date.now() + skew;
  const tick = (fn, ms) => { clearInterval(ticker); ticker = setInterval(fn, ms); fn(); };
  const photo = (name, cls, w, hgt) => h('img', { class: cls, src: 'img/' + name + '.webp', alt: '', width: w, height: hgt, draggable: 'false', decoding: 'async' });
  /* A button with a line icon before or after its words. */
  const btn = (cls, icon, text, props, after) => h('button', { class: cls, type: 'button', ...props }, after ? null : D.icon(icon), h('span', { text }), after ? D.icon(icon) : null);
  /* A press that answers back: a squash, and a few sparks from where it was pressed. */
  const joy = (el, n = 10) => { D.jelly(el); D.pop(el, n); D.buzz(8); };

  async function api(path, body) {
    const r = await fetch(path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body || {}) });
    const out = await r.json().catch(() => ({}));
    if (!r.ok) throw Object.assign(new Error(out.error || 'failed'), { code: out.error });
    return out;
  }
  // "busy" means the moment has passed (a vote after the deadline): the next state says what happened.
  const act = async (path, body) => { try { apply(await api(path, body)); } catch (e) { if (e.code !== 'busy') D.toast(t(e.code === 'full' ? 'err_full' : 'err_net')); } };

  function apply(v) {
    if (!v || !v.screen) return;
    view = v; ver = v.v || 0;
    if (v.now) skew = v.now - Date.now();
    render();
    langs();
    if (v.screen !== 'hello') poll();
  }

  async function poll() {
    if (polling) return;
    polling = true;
    while (view && view.screen !== 'hello') {
      try {
        const r = await fetch('/api/state?v=' + ver, { cache: 'no-store' });
        if (!r.ok) throw new Error('state');
        apply(await r.json());
      } catch { await D.wait(1500); }
    }
    polling = false;
  }

  function render() {
    const name = view.screen;
    if (name === shown && update[name]) return update[name](view);
    clearInterval(ticker);
    const from = shown;
    shown = name;
    const next = cur = screens[name](view, from);
    const box = $('view'), old = box.firstElementChild, mine = ++gen;
    if (!old) return box.replaceChildren(next);
    old.classList.add('leave');
    setTimeout(() => { if (mine === gen) { box.replaceChildren(next); box.scrollTop = 0; } }, D.reduced ? 0 : 170);
  }

  const langs = () => { for (const b of $('langs').children) b.classList.toggle('now', b.dataset.lang === D.lang); };
  for (const b of $('langs').children) b.onclick = () => {
    if (b.dataset.lang === D.lang) return;
    D.setLang(b.dataset.lang);
    if (view && view.me) api('/api/lang', { lang: D.lang }).catch(() => {});
    shown = null; if (view) { render(); langs(); }
  };

  const dur = mins => mins === 15 ? t('dur_15') : mins >= 1.500 ? t('dur_mins', { n: Math.round(mins) }) : mins >= 1 ? t('dur_min') : t('dur_secs', { n: Math.round(mins * 60) });
  const when = ms => new Date(ms).toLocaleString(D.lang === 'ja' ? 'ja-JP' : 'en-GB', { weekday: 'long', hour: '2-digit', minute: '2-digit' });
  const span = ms => { const s = Math.max(0, Math.ceil(ms / 1000)), hrs = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60); return hrs ? hrs + 'h ' + String(m).padStart(2, '0') + 'm' : m + ':' + String(s % 60).padStart(2, '0'); };
  const wiggle = el => { el.style.animation = 'none'; void el.offsetWidth; el.style.animation = 'wiggle .38s'; };
  const note = v => v.note ? h('p', { class: 'note', text: t('note_' + v.note) }) : null;
  const copyLink = async e => { const pressed = e.currentTarget; try { await navigator.clipboard.writeText(view.link); D.pop(pressed, 8); D.toast(t('copied'), 3800); } catch { prompt(t('copy_link'), view.link); } };
  const pair = v => h('div', { class: 'pairup' }, D.face(v.me.name, v.color, true), h('b', { text: '&' }), D.face(v.partner.name, v.partner.color, true));
  /* What the two are about to do, with a picture of it. */
  const activityCard = a => !a ? null : h('div', { class: 'actcard' }, photo('act-' + a.kind, '', 56, 56), h('div', {}, h('b', { text: D.pick(a.title) }), h('span', { text: t('act_' + a.kind) })));

  const screens = {
    hello(v) {
      let adult = false;
      const input = h('input', { class: 'input', maxlength: 24, placeholder: t('name_placeholder'), autocomplete: 'nickname', enterkeyhint: 'go', 'aria-label': t('name_placeholder') });
      const agree = h('button', { class: 'agree', type: 'button', role: 'checkbox', 'aria-checked': 'false' }, h('i'), h('span', { text: t('adult') }));
      const go = h('button', { class: 'btn wide off', type: 'submit' }, h('span', { text: t('start') }), D.icon('arrow-right'));
      const sync = () => go.classList.toggle('off', !(input.value.trim() && adult));
      agree.onclick = () => { adult = !adult; agree.classList.toggle('now', adult); agree.setAttribute('aria-checked', adult); if (adult) joy(agree.firstChild, 8); sync(); };
      input.oninput = sync;
      // Three photos, fanned like cards. They drift a little, and wobble when touched.
      const fan = h('div', { class: 'fan' }, [['hero-talk', 'l'], ['hero-make', 'r'], ['hero-two', 'c']].map(([name, at]) => {
        const card = h('div', { class: 'card ' + at }, photo(name, '', 150, 190));
        card.onclick = () => { D.jelly(card.firstChild); D.pop(card, 8); D.buzz(8); };
        return card;
      }));
      const [before, after] = t('hello_title', { dur: '\u0001' }).split('\u0001');
      const words = (text, cls) => text.split(/(\s+)/).map(w => (w.trim() ? h('span', { class: 'word ' + (cls || ''), text: w }) : w));
      const title = h('h1', {}, words(before), words(dur(v.mins || 15), 'gradtext'), words(after));
      [...title.querySelectorAll('.word')].forEach((w, i) => { w.style.animationDelay = 0.25 + i * 0.07 + 's'; });
      return h('form', { class: 'screen hero', onsubmit: e => {
        e.preventDefault();
        if (!input.value.trim()) { wiggle(input); input.focus(); return D.toast(t('err_name'), 1600); }
        if (!adult) return wiggle(agree);
        D.jelly(go);
        act('/api/join', { name: input.value, lang: D.lang, adult: true });
      } }, fan, title, h('p', { class: 'lead', text: t('hello_lead') }),
        h('div', { class: 'stack' }, input, h('div', {}, agree), go),
        h('p', { class: 'fine', text: t('hello_fine') }));
    },

    ask(v) {
      const a = Object.assign({ langs: [D.lang], mood: '', mode: '', style: '', talk: '', interests: [], smile: '' }, v.answers || {});
      const one = (key, prefix, list) => ({ key, title: t('q_' + key), options: list.map(([id, icon]) => ({ id, icon, label: t(prefix + id) })) });
      const steps = [
        { key: 'langs', title: t('q_langs'), options: [{ id: 'en', mark: 'EN', label: t('l_en') }, { id: 'ja', mark: 'JA', label: t('l_ja') }, { id: 'both', icon: 'globe', label: t('l_both') }] },
        one('mood', 'm_', [['calm', 'moon'], ['playful', 'party-popper'], ['talkative', 'message-circle'], ['tired', 'coffee']]),
        one('mode', 'd_', [['make', 'pencil'], ['play', 'gamepad-2'], ['talk', 'messages-square']]),
        one('style', 's_', [['team', 'handshake'], ['rival', 'swords'], ['either', 'shuffle']]),
        one('talk', 't_', [['listener', 'ear'], ['talker', 'megaphone'], ['both', 'scale']]),
        { key: 'interests' }, { key: 'smile' },
      ];
      const ICONS = { music: 'music', films: 'clapperboard', games: 'gamepad-2', food: 'utensils', travel: 'plane', books: 'book-open', anime: 'sparkles', sports: 'volleyball', art: 'palette', tech: 'laptop', nature: 'leaf', pets: 'paw-print' };
      let at = 0;
      const dots = h('div', { class: 'dots' }, steps.map(() => h('i')));
      const backBtn = h('button', { class: 'backlink', type: 'button', onclick: () => go(at - 1, true) }, D.icon('arrow-left'), h('span', { text: t('back') }));
      const slot = h('div', { class: 'stack' });
      const chosen = s => s.key === 'langs' ? (a.langs.length > 1 ? 'both' : a.langs[0]) : a[s.key];
      const badge = key => photo('q-' + key, 'badge', 76, 76);
      function question(s) {
        if (s.key === 'interests') {
          const next = btn('btn wide', 'arrow-right', t('next'), { onclick: () => go(at + 1) }, true);
          const chips = Object.keys(ICONS).map(id => h('button', { class: 'chip' + (a.interests.includes(id) ? ' now' : ''), type: 'button', onclick: e => {
            const on = a.interests.includes(id), chip = e.currentTarget;
            if (!on && a.interests.length >= 3) return wiggle(chip);
            a.interests = on ? a.interests.filter(x => x !== id) : [...a.interests, id];
            chip.classList.toggle('now', !on);
            if (!on) joy(chip, 8); else D.jelly(chip);
          } }, D.icon(ICONS[id]), h('span', { text: t('i_' + id) })));
          return h('div', { class: 'q' }, badge(s.key), h('h2', { text: t('q_interests') }), h('div', { class: 'grid' }, chips), next);
        }
        if (s.key === 'smile') {
          const box = h('textarea', { class: 'input', maxlength: 140, rows: 3, placeholder: t('smile_placeholder') });
          box.value = a.smile || '';
          const send = () => { a.smile = box.value; act('/api/answers', a); };
          return h('div', { class: 'q' }, badge(s.key), h('h2', { text: t('q_smile') }), box,
            btn('btn wide', 'search', t('find'), { onclick: e => { D.jelly(e.currentTarget); send(); } }),
            h('button', { class: 'link', type: 'button', text: t('skip'), onclick: () => { box.value = ''; send(); } }));
        }
        const own = s.key === 'langs' ? h('button', { class: 'link wordslink', type: 'button', onclick: e => { D.jelly(e.currentTarget); words(); } }, D.icon('pencil'), h('span', { text: t('words_link') })) : null;
        return h('div', { class: 'q' }, badge(s.key), h('h2', { text: s.title }), h('div', { class: 'opts' }, s.options.map(o =>
          h('button', { class: 'opt' + (chosen(s) === o.id ? ' now' : ''), type: 'button', onclick: e => {
            const pressed = e.currentTarget;
            if (s.key === 'langs') a.langs = o.id === 'both' ? ['en', 'ja'] : [o.id]; else a[s.key] = o.id;
            for (const b of pressed.parentNode.children) b.classList.remove('now');
            pressed.classList.add('now');
            joy(pressed.firstChild);
            setTimeout(() => go(at + 1), D.reduced ? 0 : 330);
          } }, h('span', { class: 'seat' }, o.icon ? D.icon(o.icon) : o.mark), h('span', { text: o.label })))), own);
      }
      /* "Say it in your own words": a few sentences in, the taps filled in, shown back to check. */
      const filled = s => s.key === 'interests' ? a.interests.length > 0 : s.key === 'smile' ? false : !!chosen(s);
      let skipFilled = false, note = '';
      function words() {
        const box = h('textarea', { class: 'input', maxlength: 400, rows: 5, placeholder: t('words_placeholder'), 'aria-label': t('words_title') });
        box.value = note;
        const heard = h('div', { class: 'stack heard' });
        const read = btn('btn wide', 'sparkles', t('words_read'), { onclick: async e => {
          const pressed = e.currentTarget;
          note = box.value;
          if (!note.trim()) { wiggle(box); return box.focus(); }
          D.jelly(pressed);
          pressed.classList.add('off');
          heard.replaceChildren(h('div', { class: 'row', style: { alignItems: 'center', justifyContent: 'center' } }, h('i', { class: 'spin small' }), h('span', { class: 'lead', text: t('words_reading') })));
          try {
            const r = await api('/api/understand', { text: note });
            show(r);
          } catch (err) {
            heard.replaceChildren();
            D.toast(t(err.code === 'note' ? 'err_note' : 'err_net'));
          }
          pressed.classList.remove('off');
        } });
        function show(r) {
          const got = r.answers || {};
          Object.assign(a, { langs: got.langs && got.langs.length ? got.langs : a.langs, interests: got.interests && got.interests.length ? got.interests : a.interests });
          for (const k of ['mood', 'mode', 'style', 'talk']) if (got[k]) a[k] = got[k];
          if (got.smile) a.smile = got.smile;
          const chips = [];
          if (got.langs && got.langs.length) chips.push(['languages', got.langs.length > 1 ? t('l_both') : t('l_' + got.langs[0])]);
          const ICON = { mood: { calm: 'moon', playful: 'party-popper', talkative: 'message-circle', tired: 'coffee' }, mode: { make: 'pencil', play: 'gamepad-2', talk: 'messages-square' }, style: { team: 'handshake', rival: 'swords', either: 'shuffle' }, talk: { listener: 'ear', talker: 'megaphone', both: 'scale' } };
          const PREFIX = { mood: 'm_', mode: 'd_', style: 's_', talk: 't_' };
          for (const k of ['mood', 'mode', 'style', 'talk']) if (got[k]) chips.push([ICON[k][got[k]], t(PREFIX[k] + got[k])]);
          for (const i of got.interests || []) chips.push([ICONS[i], t('i_' + i)]);
          // The page's own language stands in for languages the note did not mention.
          const missing = (r.missing || []).filter(k => k !== 'langs');
          if (!chips.length) return heard.replaceChildren(h('p', { class: 'lead', text: t('words_none') }));
          const go_on = missing.length
            ? btn('btn wide', 'arrow-right', t('words_rest', { n: missing.length }), { onclick: () => { skipFilled = true; go(0); } }, true)
            : btn('btn wide', 'search', t('words_go'), { onclick: e => { D.jelly(e.currentTarget); act('/api/answers', a); } });
          const list = h('div', { class: 'grid' }, chips.map(([icon, label], i) => {
            const c = h('span', { class: 'chip now', style: { animationDelay: i * 0.06 + 's' } }, D.icon(icon), h('span', { text: label }));
            return c;
          }));
          heard.replaceChildren(h('p', { class: 'heardtitle', text: t('words_heard') }), list, go_on,
            h('button', { class: 'link', type: 'button', text: t('words_change'), onclick: () => { skipFilled = false; go(0); } }));
          setTimeout(() => D.pop(list, 10), D.reduced ? 0 : 200);
        }
        const pane = h('div', { class: 'q' }, badge('smile'), h('h2', { text: t('words_title') }), h('p', { class: 'fine', text: t('words_lead') }), box, read, heard);
        const old = slot.firstElementChild;
        pane.classList.add('in');
        backBtn.style.visibility = 'visible';
        at = -1;
        [...dots.children].forEach(d => { d.className = ''; });
        if (!old) return slot.replaceChildren(pane);
        old.classList.add('out');
        setTimeout(() => { slot.replaceChildren(pane); box.focus(); }, D.reduced ? 0 : 150);
      }
      function go(to, back) {
        if (at === -1 && back) to = 0;
        // After a note was read, the taps it filled are passed over; "Back" still shows them.
        while (skipFilled && !back && to < steps.length - 1 && filled(steps[to])) to++;
        if (to < 0 || to >= steps.length) return;
        const old = slot.firstElementChild, next = question(steps[to]);
        at = to;
        [...dots.children].forEach((d, i) => { d.className = i < at ? 'done' : i === at ? 'now' : ''; });
        backBtn.style.visibility = at ? 'visible' : 'hidden';
        next.classList.add(back ? 'inback' : 'in');
        if (!old) return slot.replaceChildren(next);
        old.classList.add(back ? 'outback' : 'out');
        setTimeout(() => slot.replaceChildren(next), D.reduced ? 0 : 150);
      }
      go(0);
      return h('div', { class: 'screen' }, h('div', { class: 'row', style: { justifyContent: 'space-between', alignItems: 'center' } }, backBtn, dots, h('span', { style: { width: '64px' } })), slot);
    },

    wait(v) {
      const count = h('p', { class: 'count' }), notes = h('div', { class: 'stack' });
      const tipPhoto = photo('tip-1', '', 270, 160), tipText = h('figcaption');
      const tip = h('figure', { class: 'tipcard' }, tipPhoto, tipText);
      const el = h('div', { class: 'screen' },
        h('div', { class: 'orbit' }, h('i', { class: 'sweep' }), h('div', { class: 'moon' }, h('i')), D.face(v.me.name, '#ffb84d', true)),
        h('h2', { text: t('wait_title') }), count, tip, notes,
        h('button', { class: 'link', type: 'button', text: t('wait_leave'), onclick: () => act('/api/queue', { on: false }) }));
      el._set = v => {
        count.textContent = v.here > 1 ? t('wait_others', { n: v.here }) : t('wait_alone');
        notes.replaceChildren(...[note(v)].filter(Boolean));
      };
      el._set(v);
      // A card that turns over every few seconds: what is about to happen, with a picture.
      let n = 0, last = -1;
      const show = at => { tipPhoto.src = 'img/tip-' + (at + 1) + '.webp'; tipText.textContent = t('tip_' + (at + 1)); tip.style.setProperty('--tilt', (at % 2 ? 1.500 : -1.500) + 'deg'); };
      tick(() => {
        const at = Math.floor(n++ / 22) % 4;
        if (at === last) return;
        const first = last < 0;
        last = at;
        if (first || D.reduced) return show(at);
        tip.classList.add('turn');
        setTimeout(() => { show(at); tip.classList.remove('turn'); }, 260);
      }, 250);
      return el;
    },

    build(v) {
      const why = h('p', { class: 'why' }), about = h('div', { class: 'stack', style: { alignItems: 'center' } }), bar = h('i'), status = h('p', { class: 'lead' });
      const faces = pair(v);
      const el = h('div', { class: 'screen' }, faces, h('h2', { text: t('build_title', { name: v.partner.name }) }), about, why, h('div', { class: 'progress' }, bar), status);
      setTimeout(() => D.pop(faces.querySelector('b'), 14), D.reduced ? 0 : 650);
      let hadWhy = false, hadActivity = false;
      whyAt = 0;
      el._set = v => {
        const line = D.pick(v.why);
        if (line && !hadWhy) { hadWhy = true; whyAt = performance.now(); why.textContent = line; D.anim(why, [{ opacity: 0, transform: 'translateY(10px)' }, { opacity: 1, transform: 'none' }], 500); }
        if (v.activity && !hadActivity) {
          hadActivity = true;
          about.replaceChildren(...[v.long && h('span', { class: 'chip now', text: t('build_long', { mins: Math.round(v.mins) }) }), activityCard(v.activity)].filter(Boolean));
        }
        // How far along the room is, as a bar. The details belong on the admin page, not here.
        const done = v.marks.filter(m => m.done).length, total = Math.max(v.marks.length, 2);
        const part = v.marks.length ? Math.min(0.94, (done + 0.6) / (total + 0.6)) : 0.12;
        bar.style.transform = `scaleX(${part})`;
        status.textContent = t(part > 0.6 ? 'build_2' : 'build_1');
      };
      el._set(v);
      return el;
    },

    room(v, from) {
      if (from && v.url && !(cameBack && from === 'room')) {
        // Paired while this page was open: go in. The faces and the reason stay up for a moment
        // first: a room built ahead is ready at once, and the pairing screen may have only flashed.
        const shownFor = from === 'build' && whyAt ? performance.now() - whyAt : 0;
        const seen = shownFor >= 2200, faces = pair(v);
        setTimeout(() => { if (view.screen === 'room') location.assign(view.url); }, D.reduced ? 150 : seen ? 900 : Math.max(1400, 2600 - shownFor));
        if (!seen) setTimeout(() => D.pop(faces.querySelector('b'), 14), D.reduced ? 0 : 650);
        return h('div', { class: 'screen' }, faces,
          h('h2', { text: seen ? t('opening') : t('build_title', { name: v.partner.name }) }), seen ? null : activityCard(v.activity), seen ? null : h('p', { class: 'why', text: D.pick(v.why) }),
          h('div', { class: 'row', style: { alignItems: 'center' } }, h('i', { class: 'spin small' }), h('span', { class: 'lead', text: t('opening') })));
      }
      if (cameBack) {
        // Back from the room that just ended: ask the lobby to look at it now.
        api('/api/back').catch(() => {});
        return h('div', { class: 'screen' }, h('i', { class: 'spin' }), h('h2', { text: t('room_closing') }));
      }
      return h('div', { class: 'screen' }, pair(v), h('h2', { text: t('room_title') }), h('a', { class: 'btn', href: v.url }, h('span', { text: t('room_enter') }), D.icon('arrow-right')));
    },

    vote(v) {
      let liked = null, again = null;
      const activity = v.activity ? D.pick(v.activity.title) : '';
      const over = v.over && !v.over.mine && (v.over.why === 'left' ? t('over_left_them', { name: v.partner.name }) : v.over.why === 'gone' ? t('over_gone_them', { name: v.partner.name }) : v.over.why === 'ended' ? t('over_ended') : '');
      const send = btn('btn wide off', 'arrow-right', t('v_send'), { onclick: e => { if (liked != null && again != null) { D.jelly(e.currentTarget); act('/api/vote', { liked, again }); } } }, true);
      // Yes is the white button, no is the black one.
      const choice = (set, yes, no) => {
        const row = h('div', { class: 'row' });
        const make = (value, icon, label) => btn('pick ' + (value ? 'yes' : 'no'), icon, label, { onclick: e => {
          set(value);
          for (const b of row.children) b.classList.remove('now');
          e.currentTarget.classList.add('now');
          row.classList.add('chosen');
          joy(e.currentTarget, value ? 12 : 6);
          send.classList.toggle('off', liked == null || again == null);
        } });
        row.append(make(true, yes[0], yes[1]), make(false, no[0], no[1]));
        return row;
      };
      const form = h('div', { class: 'stack' },
        h('div', { class: 'card ask' }, h('p', { text: t('v_liked', { activity }) }), choice(x => { liked = x; }, ['thumbs-up', t('v_yes')], ['thumbs-down', t('v_no')])),
        h('div', { class: 'card ask' }, h('p', { text: t('v_again', { name: v.partner.name }) }), choice(x => { again = x; }, ['check', t('v_again_yes')], ['x', t('v_again_no')]), h('span', { class: 'fine', style: { maxWidth: 'none' }, text: t('v_private') })),
        send);
      const left = h('span', { class: 'fine' });
      const waiting = h('div', { class: 'stack', style: { alignItems: 'center' } }, h('i', { class: 'spin' }), h('p', { class: 'lead', text: t('v_waiting', { name: v.partner.name }) }), left);
      const body = h('div', { class: 'stack' });
      const el = h('div', { class: 'screen' }, h('h2', { text: t('gone_title') }), h('p', { class: 'lead', text: t('gone_sub') }), over ? h('p', { class: 'lead', text: over }) : null, body);
      let was = null;
      el._set = v => { if (v.voted !== was) { was = v.voted; body.replaceChildren(v.voted ? waiting : form); } };
      el._set(v);
      tick(() => { left.textContent = t('v_secs', { n: Math.max(0, Math.ceil((view.deadline_ms - now()) / 1000)) }); }, 500);
      return el;
    },

    result(v) {
      const again = btn('btn wide', 'users', t('meet_new'), { onclick: e => { D.jelly(e.currentTarget); act('/api/queue', { on: true }); } });
      const done = h('button', { class: 'link', type: 'button', text: t('done'), onclick: () => act('/api/queue', { on: false }) });
      const picture = (name, cls) => h('div', { class: 'snap ' + cls }, photo(name, '', 280, 180), h('span', { html: D.markSvg('') }));
      if (!v.mutual) {
        return h('div', { class: 'screen' }, picture('no', 'calm'), h('h2', { text: t('r_no_title') }), h('p', { class: 'lead', text: t('r_no_lead') }), h('div', { class: 'stack' }, again, done));
      }
      setTimeout(() => D.confetti(90), D.reduced ? 0 : 350);
      D.buzz([40, 60, 40]);
      const res = v.reservation;
      const quote = h('p', { class: 'notecard' });
      const el = h('div', { class: 'screen' }, picture('yes', 'glad'), h('h2', { class: 'gradtext', text: t('r_yes_title') }), quote,
        res ? h('p', { class: 'when', text: t('r_yes_when', { when: when(res.at_ms) }) }) : null,
        res ? h('p', { class: 'lead', text: t('r_yes_lead', { mins: res.mins }) }) : null,
        h('div', { class: 'row' }, res ? h('a', { class: 'btn quiet', href: '/api/calendar.ics', download: 'duet.ics' }, D.icon('calendar-plus'), h('span', { text: t('calendar') })) : null,
          btn('btn quiet', 'link', t('copy_link'), { onclick: copyLink })),
        h('div', { class: 'stack' }, again, done));
      // The note can be rewritten a moment later, so it is set in place.
      let said = null;
      el._set = v => {
        const line = v.reservation && D.pick(v.reservation.note);
        if (!line || line === said) return;
        const first = said == null;
        said = line;
        quote.textContent = line;
        quote.style.display = '';
        if (!first) D.anim(quote, [{ opacity: 0.2, transform: 'scale(.97)' }, { opacity: 1, transform: 'none' }], 420);
      };
      quote.style.display = 'none';
      el._set(v);
      return el;
    },

    home(v) {
      const next = h('div', { class: 'stack' });
      const el = h('div', { class: 'screen' }, h('h2', { text: t('home_title', { name: v.me.name }) }), next,
        h('div', { class: 'stack' },
          btn('btn wide', 'users', t('meet_new'), { onclick: e => { D.jelly(e.currentTarget); act('/api/queue', { on: true }); } }),
          h('div', { class: 'row' },
            btn('btn quiet', 'pencil', t('change_answers'), { onclick: () => act('/api/queue', { on: false }) }),
            btn('btn quiet', 'link', t('copy_link'), { onclick: copyLink }))),
        h('button', { class: 'link', type: 'button', text: t('forget'), onclick: async () => {
          if (!confirm(t('forget_q'))) return;
          try { apply(await api('/api/forget')); D.toast(t('forgotten')); } catch { D.toast(t('err_net')); }
        } }));
      const cd = h('p', { class: 'cd' }), line = h('p', { class: 'lead' });
      let sig = null;
      el._set = v => {
        const r = v.reservation, now_sig = JSON.stringify([r && r.with, r && r.open, r && r.here, r && r.note, v.note]);
        if (now_sig === sig) return;
        sig = now_sig;
        if (!r) return next.replaceChildren(...[note(v), h('p', { class: 'lead', text: t('home_lead') })].filter(Boolean));
        line.textContent = r.open ? t(r.here ? 'home_starting' : 'home_waiting', { name: r.with }) : when(r.at_ms);
        next.replaceChildren(...[note(v), h('div', { class: 'card lit ask', style: { alignItems: 'center', textAlign: 'center' } }, h('p', { text: t('home_next', { name: r.with }) }), cd, line, r.note ? h('span', { class: 'fine', style: { maxWidth: 'none' }, text: D.pick(r.note) }) : null)].filter(Boolean));
      };
      el._set(v);
      tick(() => { const r = view.reservation; if (r) cd.textContent = r.open ? t('home_time') : span(r.at_ms - now()); }, 500);
      return el;
    },
  };

  /* A screen that is already showing is changed in place, so nothing re-animates. */
  // Screens kept as they are when the lobby changes. The questions are among them: a redraw there
  // would throw away the picks not sent yet whenever someone else joined or a room was built. So is
  // the door to the room: a redraw would start its pause again and could keep people outside.
  const update = Object.fromEntries(['ask', 'wait', 'build', 'room', 'vote', 'home', 'result'].map(name => [name, v => cur && cur._set && cur._set(v)]));

  // A page restored from the browser's back button shows a stale screen: start over.
  addEventListener('pageshow', e => { if (e.persisted) location.reload(); });

  fetch('/api/state?v=0', { cache: 'no-store' }).then(r => r.json()).then(apply).catch(() => { apply({ screen: 'hello', v: 0 }); D.toast(t('err_net')); });
})();
