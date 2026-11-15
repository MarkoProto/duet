/* Same Wavelength: both answer one prompt in private, the room reveals both at once.
   The room holds each answer until the other is in, so neither screen ever has the other's early. */
(() => {
  const { h, t } = D;
  D.acts = D.acts || {};

  const FALLBACK = [
    { en: 'A food you could eat every day for a week', ja: '1週間毎日食べても飽きないもの' },
    { en: 'The best season', ja: '一番好きな季節' },
    { en: 'A sound that calms you down', ja: '落ち着く音' },
    { en: 'Something you always carry', ja: 'いつも持ち歩いているもの' },
    { en: 'A perfect Sunday, in one word', ja: '理想の日曜日を一言で' },
    { en: 'An animal you would be', ja: '自分を動物にたとえると' },
    { en: 'The first thing you do after waking up', ja: '起きて最初にすること' },
    { en: 'A place you would go right now', ja: '今すぐ行きたい場所' },
  ];

  /* Compare what was meant, not what was typed: case, width, spaces and punctuation do not count,
     and one slip of the finger is forgiven in a word of four letters or more. */
  const kata = s => s.replace(/[\u3041-\u3096]/g, c => String.fromCharCode(c.charCodeAt(0) + 0x60));
  const norm = s => { const n = kata(String(s).normalize('NFKC').toLowerCase().replace(/[\s\p{P}\p{S}]+/gu, '')); return n || String(s).trim(); };
  function distance(a, b) {
    a = [...a]; b = [...b];
    let prev = Array.from({ length: b.length + 1 }, (_, i) => i);
    for (let i = 1; i <= a.length; i++) {
      const row = [i];
      for (let j = 1; j <= b.length; j++) row[j] = Math.min(prev[j] + 1, row[j - 1] + 1, prev[j - 1] + (a[i - 1] === b[j - 1] ? 0 : 1));
      prev = row;
    }
    return prev[b.length];
  }
  function verdict(a, b) {
    const x = norm(a), y = norm(b);
    if (!x || !y) return 'diff';
    if (x === y) return 'sync';
    const len = Math.max([...x].length, [...y].length), d = distance(x, y);
    const sameStart = [...x][0] === [...y][0];
    if (sameStart && ((len >= 5 && d <= 1) || (len >= 8 && d <= 2))) return 'sync';
    if ((x.length >= 3 && y.includes(x)) || (y.length >= 3 && x.includes(y))) return 'close';
    return 'diff';
  }
  D.wavelengthVerdict = verdict;

  D.acts.wavelength = (root, ctx) => {
    const given = ctx.data && Array.isArray(ctx.data.prompts) ? ctx.data.prompts.filter(p => p && (p.en || p.ja)) : [];
    const prompts = given.length >= 3 ? given : FALLBACK;
    const { you, other } = ctx;
    let round = 0, played = 0, synced = 0, resumed = 0;
    const sealed = {}, revealed = {};
    // "Next" and "Go again" move on only when both have pressed, so one person's tap never pulls
    // the other's screen away while they are still reading. Kept by round: [Ana, Aoi].
    const ready = {}, again = {};
    const both = (m, r, by) => { const x = (m[r] ||= [false, false]); x[by] = true; return x[0] && x[1]; };
    const waitText = m => m[round]?.[you] ? t('wl_wait_next', { name: ctx.people[other].name }) : m[round]?.[other] ? t('wl_they_next', { name: ctx.people[other].name }) : '';
    const slot = r => 'w' + r;
    const mineKey = r => 'duet-' + slot(r);
    const box = h('div', { class: 'actbox wl' });
    root.replaceChildren(box);
    // The answer being typed: its round, its field and the line under it. Kept so that the other
    // person locking in changes only that line, and a redraw never loses what is being typed.
    let typing = null;
    let revealStatus = null; // the line under the revealed answers

    const head = () => h('div', { class: 'hud' },
      h('span', { class: 'pill', text: t('wl_round', { i: (round % prompts.length) + 1, n: prompts.length }) }),
      h('span', { class: 'pill' + (synced ? ' good' : '') }, D.icon('sparkles'), t('wl_score', { n: synced })));

    function render() {
      const s = slot(round);
      if (round > 0 && round % prompts.length === 0 && resumed < round && !revealed[s]) {
        const waitingMore = !!again[round]?.[you];
        return box.replaceChildren(h('p', { class: 'acttitle', text: D.pick(ctx.title) }),
          h('p', { class: 'prompt', text: t('wl_done', { n: synced, total: played }) }),
          h('button', { class: 'btn' + (waitingMore ? ' quiet' : ''), type: 'button', disabled: waitingMore, text: t('wl_more'), onclick: () => ctx.send({ k: 'again', r: round }) }),
          h('p', { class: 'status', text: waitText(again) }));
      }
      const prompt = h('p', { class: 'prompt', text: D.pick(prompts[round % prompts.length]) });
      if (revealed[s]) {
        const v = revealed[s], kind = verdict(v[0], v[1]);
        const card = i => h('div', { class: 'ans' + (kind === 'sync' ? ' sync' : ''), vars: { '--who': ctx.people[i].color } }, h('small', { text: ctx.people[i].name }), h('b', { text: String(v[i]) }));
        return box.replaceChildren(head(), prompt, h('div', { class: 'pair' }, card(you), card(other)),
          h('p', { class: 'verdict' + (kind === 'sync' ? ' gradtext' : ''), text: t(kind === 'sync' ? 'wl_sync' : kind === 'close' ? 'wl_close' : 'wl_diff') }),
          h('button', { class: 'btn' + (ready[round]?.[you] ? ' quiet' : ''), type: 'button', disabled: !!ready[round]?.[you], onclick: e => { D.jelly(e.currentTarget); ctx.send({ k: 'next', r: round }); } }, h('span', { text: t('wl_next') }), D.icon('arrow-right')),
          revealStatus = h('p', { class: 'status', text: waitText(ready) }));
      }
      const mine = sealed[s]?.[you], theirs = sealed[s]?.[other];
      const draft = !mine && typing && typing.s === s ? typing.input.value : '';
      const focused = !mine && typing && typing.s === s && document.activeElement === typing.input;
      let saved = '';
      try { saved = sessionStorage.getItem(mineKey(round)) || ''; } catch {}
      const input = h('input', { class: 'input', maxlength: 40, placeholder: t('wl_placeholder'), value: mine ? saved : '', enterkeyhint: 'done', autocomplete: 'off', autocapitalize: 'off' });
      if (mine) input.value = saved || '•••';
      else if (draft) input.value = draft;
      input.disabled = !!mine;
      input.addEventListener('focus', () => D.$('app').classList.add('acting'));
      input.addEventListener('blur', () => D.$('app').classList.remove('acting'));
      const form = h('form', { onsubmit: e => {
        e.preventDefault();
        if (mine) return; // locked is final: the room already holds this answer
        const v = input.value.trim();
        if (!v) { input.classList.remove('wrong'); void input.offsetWidth; input.classList.add('wrong'); return; }
        try { sessionStorage.setItem(mineKey(round), v); } catch {}
        (sealed[s] ||= [false, false])[you] = true;
        ctx.seal(s, v.slice(0, 40));
        D.pop(e.submitter || input, 12);
        input.blur();
        render();
      } }, input, h('button', { class: 'btn' + (mine ? ' quiet' : ''), type: 'submit', disabled: !!mine, onpointerdown: e => e.preventDefault() }, D.icon('lock'), h('span', { text: t('wl_lock') })));
      const status = h('p', { class: 'status', text: mine ? t('wl_locked', { name: ctx.people[other].name }) : theirs ? t('wl_they_locked', { name: ctx.people[other].name }) : t('wl_thinking', { name: ctx.people[other].name }) });
      box.replaceChildren(head(), prompt, form, status);
      typing = { s, input, status };
      if (focused) { input.focus(); input.setSelectionRange(input.value.length, input.value.length); }
    }

    render();
    return {
      onSealed(by, s) {
        (sealed[s] ||= [false, false])[by] = true;
        if (s !== slot(round)) return;
        // The other person locked in while this one is still typing: say so, and leave the field alone.
        if (by === other && !sealed[s][you] && !revealed[s] && typing && typing.s === s && typing.input.isConnected) {
          typing.status.textContent = t('wl_they_locked', { name: ctx.people[other].name });
          return;
        }
        render();
      },
      onReveal(s, values, replay) {
        if (revealed[s]) return;
        revealed[s] = values;
        played++;
        const kind = verdict(values[0], values[1]);
        if (kind === 'sync') synced++;
        ctx.score({ kind: 'wavelength', rounds: played, synced });
        if (s !== slot(round)) return;
        render();
        if (!replay && kind === 'sync') { D.confetti(60); D.buzz(40); }
      },
      onAct(by, d) {
        if (!d || typeof d.r !== 'number' || d.r !== round) return;
        if (d.k === 'next' && revealed[slot(round)]) {
          if (both(ready, round, by)) { round++; render(); }
          // The other pressed first: only the line changes, so a button being pressed stays put.
          else if (by === other && revealStatus?.isConnected) revealStatus.textContent = waitText(ready);
          else render();
        }
        else if (d.k === 'again') { if (both(again, round, by)) resumed = round; render(); }
      },
    };
  };
})();
