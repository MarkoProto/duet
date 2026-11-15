/* Tale for Two: a short story written together, one sentence each in turn. The first line and the
   twists that drop in along the way come from the plan (written for the two by the model, or from
   the built-in bank). Whose turn it is follows from the last line in the room's order, so both
   screens agree. New lines are added to the page one by one: the field being typed in is never
   redrawn, so nothing written there is lost when the other's line arrives. */
(() => {
  const { h, t } = D;
  D.acts = D.acts || {};
  const MAX = 140;
  const TWIST_EVERY = 3;
  const FALLBACK_START = { en: 'The last train had already left, and two strangers were still on the platform.', ja: '終電はもう出てしまったのに、ホームにはまだ見知らぬ二人が残っていた。' };
  const FALLBACK_TWISTS = [
    { en: 'Suddenly, the lights go out.', ja: '突然、明かりが消える。' },
    { en: 'A dog appears, carrying a key.', ja: '鍵をくわえた犬が現れる。' },
    { en: 'A door appears where there was none.', ja: 'なかったはずの場所に扉が現れる。' },
  ];

  D.acts.story = (root, ctx) => {
    const { you, other } = ctx;
    const data = ctx.data || {};
    const start = data.start && (data.start.en || data.start.ja) ? data.start : FALLBACK_START;
    const twists = Array.isArray(data.twists) && data.twists.some(x => x && (x.en || x.ja)) ? data.twists.filter(x => x && (x.en || x.ja)) : FALLBACK_TWISTS;
    const lines = [];       // {by, text}, in the room's order
    const known = new Set();
    let sentAndWaiting = null;

    const turnOf = () => (lines.length ? 1 - lines[lines.length - 1].by : null);
    const myTurn = () => !sentAndWaiting && (turnOf() === null || turnOf() === you);

    const tale = h('div', { class: 'tale' }, h('p', { class: 'line host', text: D.pick(start) }));
    const turn = h('span', { class: 'pill turn' });
    const input = h('input', { class: 'input', maxlength: MAX, placeholder: t('st_placeholder'), enterkeyhint: 'send', autocomplete: 'off' });
    const add = h('button', { class: 'btn', type: 'submit', onpointerdown: e => e.preventDefault() }, D.icon('pencil'), h('span', { text: t('st_add') }));
    const form = h('form', { onsubmit: e => {
      e.preventDefault();
      const text = input.value.trim().slice(0, MAX);
      if (!text) { input.classList.remove('wrong'); void input.offsetWidth; input.classList.add('wrong'); return; }
      if (!myTurn()) { turn.classList.remove('nudge'); void turn.offsetWidth; turn.classList.add('nudge'); return; }
      const id = you + '-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 6);
      sentAndWaiting = id;
      ctx.send({ k: 'line', id, text });
      input.value = '';
      D.pop(e.submitter || add, 10);
      showTurn();
    } }, input, add);
    input.addEventListener('focus', () => D.$('app').classList.add('acting'));
    input.addEventListener('blur', () => D.$('app').classList.remove('acting'));
    const box = h('div', { class: 'actbox st' }, h('div', { class: 'hud' }, h('span', { class: 'pill', text: D.pick(ctx.title) }), turn), tale, form);
    root.replaceChildren(box);

    function showTurn() {
      const yours = myTurn();
      turn.classList.toggle('good', yours);
      turn.textContent = turnOf() === null && !sentAndWaiting ? t('st_first') : yours ? t('st_your_turn') : t('sk_their_turn', { name: ctx.people[other].name });
      add.disabled = !yours;
      add.classList.toggle('quiet', !yours);
    }
    function append(el) {
      tale.append(el);
      tale.scrollTop = tale.scrollHeight;
    }
    showTurn();

    return {
      onAct(by, d, replay) {
        if (!d || d.k !== 'line' || typeof d.text !== 'string' || known.has(d.id) || lines.length >= 200) return;
        const text = d.text.trim().slice(0, MAX);
        if (!text) return;
        known.add(d.id);
        lines.push({ by, text });
        if (sentAndWaiting === d.id) sentAndWaiting = null;
        const el = h('p', { class: 'line', vars: { '--who': ctx.people[by].color } }, h('small', { text: ctx.people[by].name }), h('span', { text }));
        if (replay) el.style.animation = 'none';
        append(el);
        // Every few lines, a twist from the plan drops in. Both screens count the same lines.
        if (lines.length % TWIST_EVERY === 0) {
          const twist = twists[lines.length / TWIST_EVERY - 1];
          if (twist) {
            const card = h('p', { class: 'line twist' }, h('small', { text: t('st_twist') }), h('span', { text: D.pick(twist) }));
            if (replay) card.style.animation = 'none';
            append(card);
            if (!replay) D.buzz(20);
          }
        }
        if (!replay) ctx.score({ kind: 'story', lines: lines.length });
        showTurn();
      },
    };
  };
})();
