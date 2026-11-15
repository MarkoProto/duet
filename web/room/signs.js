/* Signs, from Duet the watch-party app: a message that is nothing but a typed face, a mark or a laugh
   (<3, :), T_T, haha, hi...) becomes a small moving picture. Inside a sentence they stay plain text. */
(() => {
  const HEART_PATH = 'M32 58C11 42 4 30 4 20 4 11 11 5 19 5c6 0 11 3 13 8 2-5 7-8 13-8 8 0 15 6 15 15 0 10-7 22-28 38Z';
  const FACE = 'fill="url(#sg-face)" stroke="#fff" stroke-width="2.5"', INK = 'fill="#1a0b1e"', LINE = 'stroke="#1a0b1e" stroke-width="3.500" stroke-linecap="round" fill="none"';
  const ROUND = `<circle cx="32" cy="32" r="27" ${FACE}/>`, DOT = INK, SHUT = 'stroke="#1a0b1e" stroke-width="3.500" stroke-linecap="round" fill="none"';
  const DROP = (x, y) => `<path class="sweat" d="M${x} ${y}c2 3 4 5 4 7a4 4 0 0 1-8 0c0-2 2-4 4-7Z" fill="#9db4ff" stroke="#fff" stroke-width="1"/>`;
  const STAR = (cx, cy, cls) => `<path class="twink ${cls}" transform="translate(${cx - 9} ${cy - 9})" d="M9 0l2.600 6 6.400.5-4.900 4.200 1.500 6.300L9 13.700 3.400 17l1.500-6.300L0 6.500 6.400 6Z" fill="#ff4f8b" stroke="#fff" stroke-width="1"/>`;
  const SIGN_ART = {
    cry: `${ROUND}<path d="M16 25c3 3 9 3 12 0M36 25c3 3 9 3 12 0" ${SHUT}/><rect class="stream s1" x="18" y="28" width="7" height="24" rx="3" fill="#7fb4ff" stroke="#fff" stroke-width="1"/><rect class="stream s2" x="39" y="28" width="7" height="24" rx="3" fill="#7fb4ff" stroke="#fff" stroke-width="1"/><path d="M26 46c3-5 9-5 12 0" ${SHUT}/>`,
    flat: `${ROUND}<path d="M16 26h13M35 26h13M23 43h18" ${SHUT}/>${DROP(49, 12)}`,
    unsure: `${ROUND}<circle cx="23" cy="26" r="3.500" ${DOT}/><circle cx="41" cy="26" r="3.500" ${DOT}/><path d="M21 45 44 37" ${SHUT}/>`,
    wow: `${ROUND}<circle class="pop" cx="22" cy="24" r="5" ${DOT}/><circle class="pop" cx="42" cy="24" r="5" ${DOT}/><ellipse class="gape" cx="32" cy="43" rx="7" ry="8" ${DOT}/>`,
    confused: `${ROUND}<circle cx="22" cy="26" r="3" ${DOT}/><circle cx="42" cy="25" r="7" fill="#fff" stroke="#1a0b1e" stroke-width="2.500"/><circle cx="42" cy="25" r="3" ${DOT}/><path d="M24 44c5-3 11 2 17-2" ${SHUT}/><text class="ask" x="54" y="14">?</text>`,
    happy: `${ROUND}<path d="M16 28c2-6 10-6 12 0M36 28c2-6 10-6 12 0" ${SHUT}/><path d="M21 38c3 8 19 8 22 0" ${SHUT}/><ellipse cx="15" cy="37" rx="4" ry="2.500" fill="#ff7fab"/><ellipse cx="49" cy="37" rx="4" ry="2.500" fill="#ff7fab"/>`,
    cute: `${ROUND}<circle cx="21" cy="28" r="3.500" ${DOT}/><circle cx="43" cy="28" r="3.500" ${DOT}/><path d="M23 38c2 5 7 5 9 0 2 5 7 5 9 0" ${SHUT}/><ellipse class="cheek" cx="14" cy="37" rx="5" ry="3" fill="#ff5f8f"/><ellipse class="cheek" cx="50" cy="37" rx="5" ry="3" fill="#ff5f8f"/>`,
    angry: `<circle cx="32" cy="32" r="27" fill="url(#sg-heart)" stroke="#fff" stroke-width="2.500"/><path d="M15 20l13 6M49 20l-13 6" ${SHUT}/><circle cx="23" cy="30" r="3.500" ${DOT}/><circle cx="41" cy="30" r="3.500" ${DOT}/><path d="M21 46c3-7 19-7 22 0" ${SHUT}/><circle class="steam" cx="10" cy="9" r="4" fill="#fff"/><circle class="steam" cx="54" cy="9" r="4" fill="#fff"/>`,
    pain: `${ROUND}<path d="M16 21l11 6-11 6M48 21l-11 6 11 6" ${SHUT}/><path d="M22 44h20" ${SHUT}/><path d="M27 41v6M32 41v6M37 41v6" stroke="#1a0b1e" stroke-width="2"/>`,
    dead: `${ROUND}<path d="M16 20l10 10M26 20 16 30M38 20l10 10M48 20 38 30" ${SHUT}/><path d="M23 42h18" ${SHUT}/><path d="M34 42h9v6a4.500 4.500 0 0 1-9 0Z" fill="#ff4f8b" stroke="#1a0b1e" stroke-width="2"/>`,
    kiss: `${ROUND}<path d="M16 27c3 3 9 3 12 0M36 27c3 3 9 3 12 0" ${SHUT}/><path d="M28 37c5-2 5 3 1 4 4 1 4 6-1 4" ${SHUT}/><path class="puff" d="M44 34c-3-3-7 1-3 5l3 3 3-3c4-4 0-8-3-5Z" fill="url(#sg-heart)" stroke="#fff" stroke-width="1"/>`,
    cool: `${ROUND}<path d="M10 22h44v3c0 8-4 12-11 12s-10-5-11-10c-1 5-4 10-11 10s-11-4-11-12Z" fill="#1a0b1e" stroke="#fff" stroke-width="1"/><rect class="glint" x="13" y="23" width="6" height="12" rx="3" fill="#fff"/><path d="M23 44c5 5 14 5 18-1" ${SHUT}/>`,
    shock: `${ROUND}<circle cx="22" cy="23" r="3" ${DOT}/><circle cx="42" cy="23" r="3" ${DOT}/><path d="M18 50c0-20 28-20 28 0Z" ${DOT}/><path d="M12 12v8M52 12v8" stroke="#7fb4ff" stroke-width="3.500" stroke-linecap="round"/>`,
    dizzy: `${ROUND}<g class="whirl"><path d="M23 19a7 7 0 1 1-7 7 4.500 4.500 0 1 1 4.500 4.500" ${SHUT}/></g><g class="whirl"><path d="M41 19a7 7 0 1 1-7 7 4.500 4.500 0 1 1 4.500 4.500" ${SHUT}/></g><path d="M23 45c3-3 6 3 9 0s6 3 9 0" ${SHUT}/>`,
    stars: `${ROUND}${STAR(22, 26, 't1')}${STAR(42, 26, 't2')}<path d="M21 40c3 8 19 8 22 0Z" ${INK}/>`,
    money: `${ROUND}<text class="small" x="22" y="32">$</text><text class="small" x="42" y="32">$</text><path d="M20 39c3 8 21 8 24 0" ${SHUT}/>`,
    sleepy: `${ROUND}<path d="M16 28c3 3 9 3 12 0M36 28c3 3 9 3 12 0" ${SHUT}/><ellipse cx="32" cy="43" rx="4" ry="3" ${DOT}/><text class="z" x="50" y="16">z</text><text class="z z2" x="42" y="12">z</text>`,
    think: `${ROUND}<circle cx="24" cy="27" r="3.500" ${DOT}/><circle cx="42" cy="24" r="3.500" ${DOT}/><path d="M24 44c5-3 11-3 16-1" ${SHUT}/><circle class="bub" cx="48" cy="12" r="2.500" fill="#fff"/><circle class="bub b2" cx="54" cy="7" r="3.500" fill="#fff"/><circle class="bub b3" cx="62" cy="1" r="4.500" fill="#fff"/>`,
    cheer: `<path class="arms" d="M16 34 3 10M48 34 61 10" stroke="#ffb84d" stroke-width="6" stroke-linecap="round"/><circle cx="32" cy="36" r="24" ${FACE}/><path d="M18 31c2-5 9-5 11 0M35 31c2-5 9-5 11 0" ${SHUT}/><path d="M20 40h24c0 10-7 13-12 13s-12-3-12-13Z" ${INK}/>`,
    wave: `<circle cx="28" cy="34" r="24" ${FACE}/><circle cx="20" cy="29" r="3" ${DOT}/><circle cx="36" cy="29" r="3" ${DOT}/><path d="M17 39c3 7 19 7 22 0" ${SHUT}/><g class="arm"><path d="M49 34 60 10" stroke="#ffb84d" stroke-width="6" stroke-linecap="round"/><circle cx="60" cy="9" r="5" fill="#ffd978" stroke="#fff" stroke-width="1.500"/></g>`,
    sweaty: `${ROUND}<path d="M16 28c2-6 10-6 12 0M36 28c2-6 10-6 12 0" ${SHUT}/><path d="M21 38c3 8 19 8 22 0" ${SHUT}/>${DROP(50, 12)}`,
    touched: `${ROUND}<path d="M16 28c2-6 10-6 12 0M36 28c2-6 10-6 12 0" ${SHUT}/><path d="M21 38c3 8 19 8 22 0" ${SHUT}/><path class="tear" d="M17 31c2 3 4 5 4 7a4 4 0 0 1-8 0c0-2 2-4 4-7Z" fill="#9db4ff" stroke="#fff" stroke-width="1"/>`,
    heart: `<path d="${HEART_PATH}" fill="url(#sg-heart)" stroke="#fff" stroke-width="2.5"/><ellipse cx="19" cy="17" rx="6" ry="4" fill="#fff" opacity=".6" transform="rotate(-28 19 17)"/>`,
    broken: `<g class="hl"><path d="M32 13C30 8 25 5 19 5 11 5 4 11 4 20c0 10 7 22 28 38l-6-15 8-9-8-9Z" fill="url(#sg-heart)" stroke="#fff" stroke-width="2.5" stroke-linejoin="round"/></g><g class="hr"><path d="M32 13c2-5 7-8 13-8 8 0 15 6 15 15 0 10-7 22-28 38l-6-15 8-9-8-9Z" fill="url(#sg-heart)" stroke="#fff" stroke-width="2.5" stroke-linejoin="round"/></g>`,
    question: '<text x="32" y="53">?</text>',
    bang: '<text x="32" y="53">!</text>',
    wat: '<text class="w1" x="22" y="53">?</text><text class="w2" x="47" y="53">!</text>',
    smile: `<circle cx="32" cy="32" r="27" ${FACE}/><circle cx="23" cy="26" r="3.500" ${INK}/><circle cx="41" cy="26" r="3.500" ${INK}/><path d="M20 37c3 8 21 8 24 0" ${LINE}/>`,
    haha: `<circle cx="30" cy="34" r="25" ${FACE}/><path d="M16 29c2-5 8-5 10 0M34 29c2-5 8-5 10 0" ${LINE}/><path d="M16 37h28c0 11-7 15-14 15s-14-4-14-15Z" ${INK}/><path d="M22 46c4-3 12-3 16 0-2 4-5 5-8 5s-6-1-8-5Z" fill="#ff4f8b"/><text class="pop p1" x="52" y="10">HA</text><text class="pop p2" x="60" y="27">HA</text>`,
    hihi: `${ROUND}<path d="M16 28c2-5 9-5 11 0M37 28c2-5 9-5 11 0" ${SHUT}/><ellipse class="cheek" cx="13" cy="36" rx="5" ry="3" fill="#ff5f8f"/><ellipse class="cheek" cx="51" cy="36" rx="5" ry="3" fill="#ff5f8f"/><g class="paw"><ellipse cx="45" cy="48" rx="8" ry="9" fill="#ffe9a8"/><rect x="20" y="36" width="27" height="6.500" rx="3.250" fill="#ffe9a8"/><rect x="21.500" y="41.500" width="26" height="6.500" rx="3.250" fill="#ffe9a8"/><rect x="23.500" y="47" width="23" height="6.500" rx="3.250" fill="#ffe9a8"/></g><text class="pop p1" x="58" y="11">hi</text><text class="pop p2" x="7" y="9">hi</text>`,
    hoho: `<circle cx="30" cy="34" r="26" ${FACE}/><path d="M15 28c2-5 9-5 11 0M34 28c2-5 9-5 11 0" ${SHUT}/><ellipse cx="12" cy="39" rx="5" ry="3" fill="#ff7fab"/><ellipse cx="48" cy="39" rx="5" ry="3" fill="#ff7fab"/><g class="oh"><ellipse cx="30" cy="46" rx="7.500" ry="8" ${INK}/><ellipse cx="30" cy="50" rx="4.500" ry="2.800" fill="#ff4f8b"/></g><text class="pop p1" x="54" y="11">HO</text><text class="pop p2" x="61" y="29">HO</text>`,
    laugh: `<circle cx="32" cy="32" r="27" ${FACE}/><path d="M17 27c2-5 8-5 10 0M37 27c2-5 8-5 10 0" ${LINE}/><path d="M17 35h30c0 12-8 17-15 17s-15-5-15-17Z" ${INK}/><path d="M24 45c4-3 12-3 16 0-2 4-5 6-8 6s-6-2-8-6Z" fill="#ff4f8b"/>`,
    sad: `<circle cx="32" cy="32" r="27" ${FACE}/><circle cx="23" cy="27" r="3.500" ${INK}/><circle cx="41" cy="27" r="3.500" ${INK}/><path d="M21 46c3-7 19-7 22 0" ${LINE}/><path class="tear" d="M15 32c2 3 4 5 4 7a4 4 0 0 1-8 0c0-2 2-4 4-7Z" fill="#9db4ff"/>`,
    wink: `<circle cx="32" cy="32" r="27" ${FACE}/><circle cx="23" cy="26" r="3.500" ${INK}/><ellipse class="lid" cx="41" cy="26" rx="4" ry="3.500" ${INK}/><path d="M20 38c5 7 19 7 24-2" ${LINE}/>`,
    tongue: `<circle cx="32" cy="32" r="27" ${FACE}/><circle cx="23" cy="25" r="3.500" ${INK}/><circle cx="41" cy="25" r="3.500" ${INK}/><path d="M20 37h24" ${LINE}/><path class="lick" d="M26 38h13v8a6.500 6.500 0 0 1-13 0Z" fill="#ff4f8b" stroke="#1a0b1e" stroke-width="2"/>`,
    dots: '<circle cx="12" cy="36" r="7" fill="url(#sg-sun)" stroke="#fff" stroke-width="2"/><circle cx="32" cy="36" r="7" fill="url(#sg-sun)" stroke="#fff" stroke-width="2"/><circle cx="52" cy="36" r="7" fill="url(#sg-sun)" stroke="#fff" stroke-width="2"/>',
  };
  const SIGNS = [
    [/^(<3+|\u2764\ufe0f?|\u2665\ufe0f?)$/u, 'heart'], [/^<[\/\\]3+$/, 'broken'], [/^\?+$/, 'question'], [/^!+$/, 'bang'], [/^(\?+!+|!+\?+)[?!]*$/, 'wat'],
    [/^(t[_\-.^]t|;[_\-.];|[:=]'-?\(+|q[_.]?q|qaq|tat)$/i, 'cry'], [/^[:=]'-?[)\]]+$/, 'touched'],
    [/^(\^[_\-.]?\^|n_n)['';]+$/, 'sweaty'], [/^(\^[_\-.]?\^|n_n)$/, 'happy'], [/^(:3|=3|>w<|>\.<w|u[wv]u|o[wv]o)$/i, 'cute'],
    [/^(>:[(\[\/]+|d:<|>:o)$/i, 'angry'], [/^(>[_.\-]?<)$/, 'pain'], [/^(x[_.]x|rip)$/i, 'dead'], [/^(:-?\*+|;-?\*+|xoxo)$/i, 'kiss'], [/^[b8]-?\)+$/i, 'cool'],
    [/^(d:|d;|dx)$/i, 'shock'], [/^@[_.\-]?@$/, 'dizzy'], [/^\*[_.\-]\*$/, 'stars'], [/^\$[_.\-]\$$/, 'money'], [/^(z{3,}|-_-z+)$/i, 'sleepy'], [/^(hm{1,}|:-?\?)$/i, 'think'],
    [/^(o[_.\-]0|0[_.\-]o)$/i, 'confused'], [/^(o[_.]o)$/, 'wow'], [/^(O[_.]o|o[_.]O)$/, 'confused'], [/^(O[_.]O|0[_.]0|[:=]-?[oO0]+|omg+)$/i, 'wow'],
    [/^(\\[o0]\/|yay+)$/i, 'cheer'], [/^([o0]\/|\\[o0])$/i, 'wave'], [/^(-[_.]-|\._\.|[:=]-?\|+)$/, 'flat'], [/^[:=]-?[\/\\sS]+$/, 'unsure'],
    [/^([:=]-?[)\]}]+|\(:|\(=)$/, 'smile'], [/^a?(h+a+\s*){2,}h*!*$/i, 'haha'], [/^(h+i+\s*){2,}h*!*$/i, 'hihi'], [/^(h+o+\s*){2,}h*!*$/i, 'hoho'],
    [/^(h+i+|he+y+|hell+o+|hiya|bye+( ?bye+)?|[c\u0107\u010d]ao+|zdravo)!*$/i, 'wave'], [/^([:=]-?D+|x-?d+|lo+l+|lmf?ao+|rofl|(he){2,}h?)$/i, 'laugh'], [/^([:=]-?[(\[c]+|\):|\)=)$/, 'sad'],
    [/^;-?[)\]D]+$/, 'wink'], [/^[:;=]-?[pP]+$/, 'tongue'], [/^(\.{3,}|\u2026)$/, 'dots'],
  ];
  const signFor = text => (SIGNS.find(([match]) => match.test(text.trim())) || [])[1];
  D.signFor = signFor;
  D.signArt = sign => '<svg viewBox="0 0 64 64" aria-hidden="true">' + SIGN_ART[sign] + '</svg>';
})();
