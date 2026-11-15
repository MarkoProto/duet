/* This or That: two options, each person picks one in private, the room shows both picks at once.
   The pairs come from the plan (written for the two by the model, or from the built-in bank). */
(() => {
  const { h, t } = D;
  D.acts = D.acts || {};

  const FALLBACK = [
    ['Morning', '朝型', 'Night', '夜型'],
    ['Beach', '海', 'Mountains', '山'],
    ['Call', '電話', 'Text', 'メッセージ'],
    ['Summer', '夏', 'Winter', '冬'],
    ['Sweet', '甘いもの', 'Salty', 'しょっぱいもの'],
    ['City', '都会', 'Countryside', '田舎'],
    ['Coffee', 'コーヒー', 'Tea', 'お茶'],
    ['Cats', '猫', 'Dogs', '犬'],
  ].map(([ae, aj, be, bj]) => ({ a: { en: ae, ja: aj }, b: { en: be, ja: bj } }));

  D.acts.thisorthat = (root, ctx) => {
    const given = ctx.data && Array.isArray(ctx.data.pairs) ? ctx.data.pairs.filter(p => p && p.a && p.b && (p.a.en || p.a.ja) && (p.b.en || p.b.ja)) : [];
    const pairs = given.length >= 3 ? given : FALLBACK;
    const { you, other } = ctx;
    const them = () => ctx.people[other].name;
    let round = 0, played = 0, same = 0, resumed = 0;
    const picked = {}, revealed = {};
    // "Next" and "Go again" move on only when both have pressed: [Ana, Ben] by round.
    const ready = {}, again = {};
    const both = (m, r, by) => { const x = (m[r] ||= [false, false]); x[by] = true; return x[0] && x[1]; };
    const waitText = m => m[round]?.[you] ? t('wl_wait_next', { name: them() }) : m[round]?.[other] ? t('wl_they_next', { name: them() }) : '';
    const slot = r => 't' + r;
    let mine = {}; // what this person picked, by round, so the choice shows while waiting
    let status = null; // the line under the options or the picks, changed alone when the other acts
    let sealTimer = 0;
    const box = h('div', { class: 'actbox tt' });
    root.replaceChildren(box);

    const head = () => h('div', { class: 'hud' },
      h('span', { class: 'pill', text: t('wl_round', { i: (round % pairs.length) + 1, n: pairs.length }) }),
      h('span', { class: 'pill' + (same ? ' good' : '') }, D.icon('sparkles'), t('tt_score', { n: same })));

    function render() {
      const s = slot(round);
      if (round > 0 && round % pairs.length === 0 && resumed < round && !revealed[s]) {
        const waiting = !!again[round]?.[you];
        return box.replaceChildren(h('p', { class: 'acttitle', text: D.pick(ctx.title) }),
          h('p', { class: 'prompt', text: t('tt_done', { n: same, total: played }) }),
          h('button', { class: 'btn' + (waiting ? ' quiet' : ''), type: 'button', disabled: waiting, text: t('wl_more'), onclick: () => ctx.send({ k: 'again', r: round }) }),
          h('p', { class: 'status', text: waitText(again) }));
      }
      const pair = pairs[round % pairs.length];
      const option = (key, chosen) => h('button', { class: 'opt ' + key + (chosen ? ' now' : ''), type: 'button', disabled: !!revealed[s], onclick: e => pick(key, e) },
        h('b', { text: D.pick(pair[key]) }));
      if (revealed[s]) {
        const v = revealed[s], match = v[0] === v[1];
        const card = i => h('div', { class: 'ans' + (match ? ' sync' : ''), vars: { '--who': ctx.people[i].color } }, h('small', { text: ctx.people[i].name }), h('b', { text: D.pick(pair[v[i]] || pair.a) }));
        return box.replaceChildren(head(), h('p', { class: 'vs' }, option('a', false), h('span', { text: t('tt_or') }), option('b', false)),
          h('div', { class: 'pair' }, card(you), card(other)),
          h('p', { class: 'verdict' + (match ? ' gradtext' : ''), text: t(match ? 'tt_same' : 'tt_diff') }),
          h('button', { class: 'btn' + (ready[round]?.[you] ? ' quiet' : ''), type: 'button', disabled: !!ready[round]?.[you], onclick: e => { D.jelly(e.currentTarget); ctx.send({ k: 'next', r: round }); } }, h('span', { text: t('wl_next') }), D.icon('arrow-right')),
          status = h('p', { class: 'status', text: waitText(ready) }));
      }
      let mineNow = mine[round];
      if (!mineNow && picked[s]?.[you]) { try { mineNow = mine[round] = sessionStorage.getItem('duet-tt' + round) || undefined; } catch {} }
      const theirs = picked[s]?.[other];
      box.replaceChildren(head(), h('p', { class: 'vs' }, option('a', mineNow === 'a'), h('span', { text: t('tt_or') }), option('b', mineNow === 'b')),
        status = h('p', { class: 'status', text: mineNow ? t('tt_waiting', { name: them() }) : theirs ? t('tt_they_picked', { name: them() }) : t('tt_choosing', { name: them() }) }));
    }

    function pick(key, e) {
      const s = slot(round);
      if (revealed[s]) return;
      mine[round] = key;
      try { sessionStorage.setItem('duet-tt' + round, key); } catch {}
      (picked[s] ||= [false, false])[you] = true;
      // Until both are in, a pick can still be changed. Quick re-taps send only the last one.
      clearTimeout(sealTimer);
      sealTimer = setTimeout(() => { if (!revealed[s]) ctx.seal(s, mine[round] || key); }, 250);
      D.pop(e.currentTarget, 10);
      render();
    }

    render();
    return {
      onSealed(by, s) {
        (picked[s] ||= [false, false])[by] = true;
        if (s !== slot(round) || by !== other || revealed[s]) return;
        // Only the line changes, so an option being pressed stays put.
        if (status?.isConnected) status.textContent = mine[round] ? t('tt_waiting', { name: them() }) : t('tt_they_picked', { name: them() });
        else render();
      },
      onReveal(s, values, replay) {
        if (revealed[s]) return;
        const v = values.map(x => (x === 'b' ? 'b' : 'a'));
        revealed[s] = v;
        played++;
        if (v[0] === v[1]) same++;
        ctx.score({ kind: 'thisorthat', rounds: played, same });
        if (s !== slot(round)) return;
        render();
        if (!replay && v[0] === v[1]) { D.confetti(50); D.buzz(40); }
      },
      onAct(by, d) {
        if (!d || typeof d.r !== 'number' || d.r !== round) return;
        if (d.k === 'next' && revealed[slot(round)]) {
          if (both(ready, round, by)) { round++; render(); }
          else if (by === other && status?.isConnected) status.textContent = waitText(ready);
          else render();
        }
        else if (d.k === 'again') { if (both(again, round, by)) resumed = round; render(); }
      },
    };
  };
})();
