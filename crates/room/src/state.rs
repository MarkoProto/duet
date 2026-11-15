//! One pair's session, held in memory and nowhere else.
//!
//! The room gives every stored event a number, so both screens see the same things in the same
//! order and a page that reconnects can ask for exactly what it missed. Momentary things (typing,
//! a pen in motion, a floating reaction) are passed on without a number and are never stored.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use axum::extract::ws::Utf8Bytes;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};

use crate::config::RoomConfig;
use crate::limit::Limits;
use crate::text::{clean_mark, clean_text, same_secret};

const MAX_EVENTS: usize = 20_000;
const MAX_CHAT_CHARS: usize = 600;
const MAX_ACT_BYTES: usize = 32 * 1024;
const MAX_LIVE_BYTES: usize = 4 * 1024;
const MAX_SEAL_BYTES: usize = 400;
const MAX_SLOTS: usize = 500;
const MAX_MARKS_EACH: usize = 12;
const LIVE_KEPT: usize = 64;
/// A dropped connection is mentioned to the other person after this long.
const AWAY_SAY_MS: u64 = 8_000;
/// A held request counts as presence for this long after it was made.
const POLL_PRESENCE_MS: u64 = 35_000;
const LAST_MINUTE_MS: u64 = 60_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Open, but the second person has not arrived.
    Waiting,
    Live,
    /// The last minute.
    Ending,
    Over,
}

impl Phase {
    pub fn name(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting",
            Phase::Live => "live",
            Phase::Ending => "ending",
            Phase::Over => "over",
        }
    }

    fn running(self) -> bool {
        matches!(self, Phase::Live | Phase::Ending)
    }
}

/// What travels to every connection.
#[derive(Clone, Debug)]
pub enum Frame {
    /// Stored and numbered.
    Event { seq: u64, json: Utf8Bytes },
    /// Passed on once. `from` is skipped when sending, so nobody hears their own echo.
    Live { from: Option<u8>, json: Utf8Bytes },
}

/// What a page may send.
#[derive(Debug, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMsg {
    Chat {
        mid: String,
        text: String,
        #[serde(default)]
        re: Option<String>,
    },
    /// Add or take back a reaction on a message.
    Mreact {
        id: String,
        e: String,
        on: bool,
    },
    /// A move in the activity. The room only orders it; the page modules give it meaning.
    Act {
        #[serde(default)]
        mid: Option<String>,
        d: Value,
    },
    /// A private answer, held until the other person has sealed the same slot.
    Seal {
        slot: String,
        v: Value,
    },
    /// Something momentary: `typing`, `float`, `nudge`, `pen`, `ping`.
    Live {
        k: String,
        #[serde(default)]
        d: Value,
    },
    /// The activity's own record of how far the pair got. Kept after the room is gone.
    Score {
        d: Value,
    },
    Ping {
        c: f64,
    },
    Report {
        reason: String,
        #[serde(default)]
        note: String,
    },
    Leave,
}

struct MessageMeta {
    marks: Vec<(u8, String)>,
}

struct Seal {
    values: [Option<Value>; 2],
    revealed: bool,
}

#[derive(Clone)]
struct Report {
    by: u8,
    at_ms: u64,
    reason: String,
    note: String,
}

#[derive(Default)]
struct Counts {
    chat: [u32; 2],
    acts: u32,
    marks: u32,
    reveals: u32,
    floats: u32,
}

struct Inner {
    phase: Phase,
    why: Option<&'static str>,
    over_by: Option<u8>,
    started_ms: Option<u64>,
    ends_ms: Option<u64>,
    over_ms: Option<u64>,
    first_join_ms: Option<u64>,
    /// `log[n]` is event number `n + 1`.
    log: Vec<Utf8Bytes>,
    live: VecDeque<(u64, Option<u8>, Utf8Bytes)>,
    next_live: u64,
    joined: [bool; 2],
    sockets: [u32; 2],
    poll_seen_ms: [u64; 2],
    last_seen_ms: [u64; 2],
    away_said: [bool; 2],
    pres_sent: [bool; 2],
    messages: HashMap<String, MessageMeta>,
    seen_mids: [VecDeque<String>; 2],
    seals: HashMap<String, Seal>,
    limits: [Limits; 2],
    counts: Counts,
    summary: Value,
    report: Option<Report>,
    transcript: Vec<(u64, u8, String)>,
    last_chat_ms: u64,
    nudges_sent: usize,
}

pub struct Room {
    pub cfg: RoomConfig,
    secret: String,
    boot: Instant,
    inner: Mutex<Inner>,
    tx: broadcast::Sender<Frame>,
    /// Bumped on every frame, so held requests know to look again.
    wake: watch::Sender<u64>,
}

fn text_of(v: &Value) -> Utf8Bytes {
    Utf8Bytes::from(v.to_string())
}

impl Room {
    /// `secret` is what the lobby must show to read status or the export.
    pub fn new(cfg: RoomConfig, secret: String) -> Arc<Self> {
        let (tx, _) = broadcast::channel(1024);
        let (wake, _) = watch::channel(0);
        Arc::new(Self {
            cfg,
            secret,
            boot: Instant::now(),
            tx,
            wake,
            inner: Mutex::new(Inner {
                phase: Phase::Waiting,
                why: None,
                over_by: None,
                started_ms: None,
                ends_ms: None,
                over_ms: None,
                first_join_ms: None,
                log: Vec::with_capacity(512),
                live: VecDeque::with_capacity(LIVE_KEPT),
                next_live: 1,
                joined: [false; 2],
                sockets: [0; 2],
                poll_seen_ms: [0; 2],
                last_seen_ms: [0; 2],
                away_said: [false; 2],
                pres_sent: [false; 2],
                messages: HashMap::new(),
                seen_mids: [VecDeque::new(), VecDeque::new()],
                seals: HashMap::new(),
                limits: [Limits::default(); 2],
                counts: Counts::default(),
                summary: Value::Null,
                report: None,
                transcript: Vec::new(),
                last_chat_ms: 0,
                nudges_sent: 0,
            }),
        })
    }

    /// Starts the clock task. It stops by itself when the room is dropped.
    pub fn run(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut every = tokio::time::interval(std::time::Duration::from_millis(250));
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                every.tick().await;
                match weak.upgrade() {
                    Some(room) => room.tick(),
                    None => break,
                }
            }
        });
    }

    pub fn now_ms(&self) -> u64 {
        self.boot.elapsed().as_millis() as u64
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock must not take the room down with it.
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Which person a link key belongs to.
    pub fn who(&self, key: &str) -> Option<u8> {
        let a = same_secret(key, &self.cfg.people[0].key);
        let b = same_secret(key, &self.cfg.people[1].key);
        match (a, b) {
            (true, _) => Some(0),
            (_, true) => Some(1),
            _ => None,
        }
    }

    pub fn is_lobby(&self, secret: &str) -> bool {
        same_secret(secret, &self.secret)
    }

    pub fn wake(&self) -> watch::Receiver<u64> {
        self.wake.subscribe()
    }

    pub fn phase(&self) -> Phase {
        self.lock().phase
    }

    // ---- sending -------------------------------------------------------------------------

    fn emit(&self, st: &mut Inner, mut ev: Value) -> u64 {
        let seq = st.log.len() as u64 + 1;
        let obj = ev.as_object_mut().expect("events are objects");
        obj.insert("s".into(), seq.into());
        obj.insert("ts".into(), self.now_ms().into());
        let json = text_of(&ev);
        st.log.push(json.clone());
        let _ = self.tx.send(Frame::Event { seq, json });
        self.wake.send_modify(|n| *n += 1);
        seq
    }

    fn pass(&self, st: &mut Inner, from: Option<u8>, v: &Value) {
        let json = text_of(v);
        if st.live.len() == LIVE_KEPT {
            st.live.pop_front();
        }
        st.live.push_back((st.next_live, from, json.clone()));
        st.next_live += 1;
        let _ = self.tx.send(Frame::Live { from, json });
        self.wake.send_modify(|n| *n += 1);
    }

    fn present(st: &Inner, i: usize, now: u64) -> bool {
        st.sockets[i] > 0
            || (st.poll_seen_ms[i] > 0 && now.saturating_sub(st.poll_seen_ms[i]) < POLL_PRESENCE_MS)
    }

    fn sync_presence(&self, st: &mut Inner) {
        let now = self.now_ms();
        let pres = [Self::present(st, 0, now), Self::present(st, 1, now)];
        if pres != st.pres_sent {
            st.pres_sent = pres;
            self.pass(st, None, &json!({"t": "pres", "on": pres}));
        }
    }

    fn finish(&self, st: &mut Inner, why: &'static str, by: Option<u8>) {
        if st.phase == Phase::Over {
            return;
        }
        st.phase = Phase::Over;
        st.why = Some(why);
        st.over_by = by;
        st.over_ms = Some(self.now_ms());
        self.emit(st, json!({"t": "phase", "p": "over", "why": why, "by": by}));
    }

    fn arrive(&self, st: &mut Inner, who: usize) {
        let now = self.now_ms();
        st.last_seen_ms[who] = now;
        if !st.joined[who] {
            st.joined[who] = true;
            st.first_join_ms.get_or_insert(now);
            self.emit(st, json!({"t": "sys", "k": "join", "by": who}));
            if st.joined == [true, true] && st.phase == Phase::Waiting {
                let ends = now + self.cfg.session_ms;
                st.phase = Phase::Live;
                st.started_ms = Some(now);
                st.ends_ms = Some(ends);
                st.last_chat_ms = now;
                self.emit(
                    st,
                    json!({"t": "phase", "p": "live", "starts": now, "ends": ends}),
                );
                if !self.cfg.host.why.is_empty() {
                    self.emit(
                        st,
                        json!({"t": "host", "k": "why", "text": self.cfg.host.why}),
                    );
                }
                if !self.cfg.host.opener.is_empty() {
                    self.emit(
                        st,
                        json!({"t": "host", "k": "opener", "text": self.cfg.host.opener}),
                    );
                }
            }
        } else if st.away_said[who] {
            st.away_said[who] = false;
            if st.phase != Phase::Over {
                self.emit(st, json!({"t": "sys", "k": "back", "by": who}));
            }
        }
        self.sync_presence(st);
    }

    // ---- connections ---------------------------------------------------------------------

    /// A socket opens. Returns the greeting (with everything after `after`), the number of the
    /// last event in it, and the feed of what follows. Taken under one lock, so nothing is
    /// missed and nothing arrives twice.
    pub fn attach(&self, who: u8, after: u64) -> (Utf8Bytes, u64, broadcast::Receiver<Frame>) {
        let mut st = self.lock();
        let feed = self.tx.subscribe();
        st.sockets[who as usize] += 1;
        self.arrive(&mut st, who as usize);
        let hello = self.hello(&st, who, after);
        (Utf8Bytes::from(hello), st.log.len() as u64, feed)
    }

    pub fn detach(&self, who: u8) {
        let mut st = self.lock();
        let i = who as usize;
        st.sockets[i] = st.sockets[i].saturating_sub(1);
        st.last_seen_ms[i] = self.now_ms();
        self.sync_presence(&mut st);
    }

    /// The greeting for a page that cannot hold a socket open.
    pub fn hello_for_poll(&self, who: u8, after: u64) -> String {
        let mut st = self.lock();
        st.poll_seen_ms[who as usize] = self.now_ms();
        self.arrive(&mut st, who as usize);
        self.hello(&st, who, after)
    }

    /// What is new for a page that holds requests instead of a socket, or `None` if nothing is.
    pub fn poll(&self, who: u8, after: u64, after_live: u64) -> Option<String> {
        let mut st = self.lock();
        st.poll_seen_ms[who as usize] = self.now_ms();
        self.arrive(&mut st, who as usize);
        let total = st.log.len() as u64;
        let from = after.min(total) as usize;
        let lives: Vec<&str> = st
            .live
            .iter()
            .filter(|(id, sender, _)| *id > after_live && *sender != Some(who))
            .map(|(_, _, j)| j.as_str())
            .collect();
        if from as u64 == total && lives.is_empty() {
            return None;
        }
        let events: Vec<&str> = st.log[from..].iter().map(Utf8Bytes::as_str).collect();
        Some(format!(
            "{{\"seq\":{},\"la\":{},\"ev\":[{}],\"lv\":[{}]}}",
            total,
            st.next_live - 1,
            events.join(","),
            lives.join(",")
        ))
    }

    /// Everything stored after event number `after`, for a socket that fell behind.
    pub fn events_after(&self, after: u64) -> Vec<(u64, Utf8Bytes)> {
        let st = self.lock();
        let from = after.min(st.log.len() as u64) as usize;
        st.log[from..]
            .iter()
            .enumerate()
            .map(|(i, j)| ((from + i) as u64 + 1, j.clone()))
            .collect()
    }

    fn hello(&self, st: &Inner, who: u8, after: u64) -> String {
        let now = self.now_ms();
        let cfg = &self.cfg;
        let people: Vec<Value> = cfg
            .people
            .iter()
            .map(|p| json!({"name": p.name, "color": p.color, "lang": p.lang}))
            .collect();
        let head = json!({
            "t": "hello",
            "you": who,
            "room": {
                "id": cfg.room_id,
                "people": people,
                "activity": cfg.activity,
                "runtime": cfg.runtime,
                "session_ms": cfg.session_ms,
                "long": cfg.long,
                "return_url": cfg.return_url,
                "alternatives": cfg.alternatives,
            },
            "phase": st.phase.name(),
            "why": st.why,
            "by": st.over_by,
            "now": now,
            "starts": st.started_ms,
            "ends": st.ends_ms,
            "seq": st.log.len(),
            "la": st.next_live - 1,
            "pres": [Self::present(st, 0, now), Self::present(st, 1, now)],
        });
        // The stored events are already JSON text: paste them in rather than encode them again.
        let mut out = head.to_string();
        out.pop();
        out.push_str(",\"log\":[");
        // A number from some other room, or from the future, means "send everything".
        let from = if after > st.log.len() as u64 {
            0
        } else {
            after as usize
        };
        for (i, ev) in st.log[from..].iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str(ev.as_str());
        }
        out.push_str("]}");
        out
    }

    // ---- the clock -----------------------------------------------------------------------

    pub fn tick(&self) {
        let mut st = self.lock();
        let now = self.now_ms();
        let cfg = &self.cfg;
        self.sync_presence(&mut st);
        match st.phase {
            Phase::Over => return,
            Phase::Waiting => {
                match st.first_join_ms {
                    Some(first) if now.saturating_sub(first) >= cfg.join_wait_ms => {
                        self.finish(&mut st, "noshow", None);
                    }
                    None if now >= cfg.join_wait_ms * 2 + 60_000 => {
                        self.finish(&mut st, "empty", None);
                    }
                    _ => {}
                }
                return;
            }
            Phase::Live | Phase::Ending => {}
        }
        let ends = st.ends_ms.unwrap_or(now);
        if now >= ends {
            self.finish(&mut st, "time", None);
            return;
        }
        if st.phase == Phase::Live && ends - now <= LAST_MINUTE_MS.min(cfg.session_ms / 3) {
            st.phase = Phase::Ending;
            self.emit(&mut st, json!({"t": "phase", "p": "ending", "ends": ends}));
            if !cfg.host.last_minute.is_empty() {
                self.emit(
                    &mut st,
                    json!({"t": "host", "k": "last", "text": cfg.host.last_minute}),
                );
            }
        }
        for i in 0..2 {
            if Self::present(&st, i, now) {
                st.last_seen_ms[i] = now;
                continue;
            }
            let gone = now.saturating_sub(st.last_seen_ms[i]);
            if gone >= cfg.away_ms {
                self.emit(&mut st, json!({"t": "sys", "k": "left", "by": i}));
                self.finish(&mut st, "gone", Some(i as u8));
                return;
            }
            if gone >= AWAY_SAY_MS.min(cfg.away_ms / 2) && !st.away_said[i] {
                st.away_said[i] = true;
                self.emit(&mut st, json!({"t": "sys", "k": "away", "by": i}));
            }
        }
        let quiet = now.saturating_sub(st.last_chat_ms);
        if st.phase == Phase::Live
            && st.nudges_sent < cfg.host.nudges.len()
            && quiet >= cfg.nudge_after_ms
            && ends - now > cfg.nudge_after_ms
        {
            let line = cfg.host.nudges[st.nudges_sent].clone();
            st.nudges_sent += 1;
            st.last_chat_ms = now;
            self.emit(&mut st, json!({"t": "host", "k": "nudge", "text": line}));
        }
    }

    // ---- what people send ----------------------------------------------------------------

    /// Handles one message. The return value, if any, goes back to the sender alone.
    pub fn handle(&self, who: u8, msg: ClientMsg) -> Option<String> {
        let w = who as usize;
        let now = self.now_ms();
        let refuse = |code: &str, mid: Option<&str>| {
            Some(json!({"t": "err", "code": code, "mid": mid}).to_string())
        };
        let mut st = self.lock();
        match msg {
            ClientMsg::Ping { c } => Some(json!({"t": "pong", "c": c, "now": now}).to_string()),

            ClientMsg::Chat { mid, text, re } => {
                if st.phase == Phase::Over {
                    return refuse("closed", Some(&mid));
                }
                if mid.is_empty() || mid.len() > 40 {
                    return refuse("bad", None);
                }
                if st.seen_mids[w].contains(&mid) {
                    return None; // a retry of something that already arrived
                }
                if !st.limits[w].chat.take(now) {
                    return refuse("slow", Some(&mid));
                }
                if st.log.len() >= MAX_EVENTS {
                    return refuse("full", Some(&mid));
                }
                let Some(text) = clean_text(&text, MAX_CHAT_CHARS) else {
                    return refuse("empty", Some(&mid));
                };
                let re = re.filter(|id| st.messages.contains_key(id));
                let id = format!("m{}", st.log.len() + 1);
                if st.seen_mids[w].len() == 64 {
                    st.seen_mids[w].pop_front();
                }
                st.seen_mids[w].push_back(mid.clone());
                st.messages
                    .insert(id.clone(), MessageMeta { marks: Vec::new() });
                st.counts.chat[w] += 1;
                st.last_chat_ms = now;
                st.transcript.push((now, who, text.clone()));
                self.emit(
                    &mut st,
                    json!({"t": "chat", "by": who, "id": id, "mid": mid, "text": text, "re": re}),
                );
                None
            }

            ClientMsg::Mreact { id, e, on } => {
                if st.phase == Phase::Over || !st.limits[w].react.take(now) {
                    return None;
                }
                let mark = clean_mark(&e)?.to_string();
                let meta = st.messages.get_mut(&id)?;
                let at = meta
                    .marks
                    .iter()
                    .position(|(by, m)| *by == who && *m == mark);
                match (on, at) {
                    (true, None)
                        if meta.marks.iter().filter(|(by, _)| *by == who).count()
                            < MAX_MARKS_EACH =>
                    {
                        meta.marks.push((who, mark.clone()));
                    }
                    (false, Some(i)) => {
                        meta.marks.remove(i);
                    }
                    _ => return None,
                }
                st.counts.marks += 1;
                self.emit(
                    &mut st,
                    json!({"t": "mreact", "by": who, "id": id, "e": mark, "on": on}),
                );
                None
            }

            ClientMsg::Act { mid, d } => {
                if !st.phase.running() {
                    return refuse("closed", mid.as_deref());
                }
                if !st.limits[w].act.take(now) {
                    return refuse("slow", mid.as_deref());
                }
                if st.log.len() >= MAX_EVENTS {
                    return refuse("full", mid.as_deref());
                }
                if d.to_string().len() > MAX_ACT_BYTES {
                    return refuse("big", mid.as_deref());
                }
                // A move re-sent after a reconnect must not be played twice.
                if let Some(mid) = &mid {
                    if mid.len() > 40 || st.seen_mids[w].contains(mid) {
                        return None;
                    }
                    if st.seen_mids[w].len() == 64 {
                        st.seen_mids[w].pop_front();
                    }
                    st.seen_mids[w].push_back(mid.clone());
                }
                st.counts.acts += 1;
                self.emit(&mut st, json!({"t": "act", "by": who, "mid": mid, "d": d}));
                None
            }

            ClientMsg::Seal { slot, v } => {
                if !st.phase.running() || !st.limits[w].seal.take(now) {
                    return refuse("closed", None);
                }
                if slot.is_empty() || slot.len() > 40 || v.to_string().len() > MAX_SEAL_BYTES {
                    return refuse("bad", None);
                }
                if !st.seals.contains_key(&slot) && st.seals.len() >= MAX_SLOTS {
                    return refuse("full", None);
                }
                let seal = st.seals.entry(slot.clone()).or_insert(Seal {
                    values: [None, None],
                    revealed: false,
                });
                if seal.revealed {
                    return None;
                }
                let first_time = seal.values[w].is_none();
                seal.values[w] = Some(v);
                let both = seal.values.iter().all(Option::is_some);
                let values = both.then(|| {
                    seal.revealed = true;
                    seal.values.clone()
                });
                if first_time {
                    self.emit(&mut st, json!({"t": "sealed", "by": who, "slot": slot}));
                }
                if let Some(values) = values {
                    st.counts.reveals += 1;
                    self.emit(&mut st, json!({"t": "reveal", "slot": slot, "v": values}));
                }
                None
            }

            ClientMsg::Live { k, d } => {
                if st.phase == Phase::Over {
                    return None;
                }
                let allowed = match k.as_str() {
                    "nudge" => st.limits[w].nudge.take(now),
                    "typing" | "float" | "pen" | "ping" => st.limits[w].live.take(now),
                    _ => false,
                };
                if !allowed || d.to_string().len() > MAX_LIVE_BYTES {
                    return None;
                }
                if k == "float" {
                    st.counts.floats += 1;
                }
                self.pass(
                    &mut st,
                    Some(who),
                    &json!({"t": "live", "by": who, "k": k, "d": d}),
                );
                None
            }

            ClientMsg::Score { d } => {
                if d.to_string().len() <= 2048 {
                    st.summary = d;
                }
                None
            }

            ClientMsg::Report { reason, note } => {
                if st.report.is_none() {
                    st.report = Some(Report {
                        by: who,
                        at_ms: now,
                        reason: clean_text(&reason, 40).unwrap_or_else(|| "other".into()),
                        note: clean_text(&note, 500).unwrap_or_default(),
                    });
                }
                // The other person sees only that the session ended.
                self.finish(&mut st, "ended", None);
                Some(json!({"t": "reported"}).to_string())
            }

            ClientMsg::Leave => {
                if st.phase != Phase::Over {
                    self.emit(&mut st, json!({"t": "sys", "k": "left", "by": who}));
                    self.finish(&mut st, "left", Some(who));
                }
                None
            }
        }
    }

    // ---- what the lobby reads ------------------------------------------------------------

    /// Ends the session now. Used when the lobby's own clock says the time is up.
    pub fn end(&self, why: &'static str) {
        let mut st = self.lock();
        self.finish(&mut st, why, None);
    }

    /// Numbers and state only. No words anyone typed.
    pub fn status(&self) -> Value {
        let st = self.lock();
        let now = self.now_ms();
        json!({
            "room": self.cfg.room_id,
            "phase": st.phase.name(),
            "why": st.why,
            "by": st.over_by,
            "joined": st.joined,
            "present": [Self::present(&st, 0, now), Self::present(&st, 1, now)],
            "elapsed_ms": st.started_ms.map(|s| st.over_ms.unwrap_or(now).saturating_sub(s)),
            "remaining_ms": if st.phase.running() { st.ends_ms.map(|e| e.saturating_sub(now)) } else { None },
            "reported": st.report.is_some(),
            "activity": self.cfg.activity.kind,
            "counts": {
                "chat": st.counts.chat,
                "acts": st.counts.acts,
                "marks": st.counts.marks,
                "reveals": st.counts.reveals,
                "floats": st.counts.floats,
                "nudges": st.nudges_sent,
            },
            "summary": st.summary,
            "events": st.log.len(),
        })
    }

    /// The chat, released only when someone in this room reported it.
    pub fn export(&self) -> Option<Value> {
        let st = self.lock();
        let report = st.report.as_ref()?;
        let lines: Vec<Value> = st
            .transcript
            .iter()
            .map(|(ts, by, text)| json!({"ts": ts, "by": by, "text": text}))
            .collect();
        Some(json!({
            "room": self.cfg.room_id,
            "names": [self.cfg.people[0].name, self.cfg.people[1].name],
            "activity": self.cfg.activity.kind,
            "report": {"by": report.by, "at_ms": report.at_ms, "reason": report.reason, "note": report.note},
            "chat": lines,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::sample;

    fn room(session_ms: u64) -> Arc<Room> {
        Room::new(sample(session_ms).checked().unwrap(), "lobby-secret".into())
    }

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    fn chat(room: &Room, who: u8, mid: &str, text: &str) -> Option<String> {
        room.handle(
            who,
            ClientMsg::Chat {
                mid: mid.into(),
                text: text.into(),
                re: None,
            },
        )
    }

    fn drain(feed: &mut broadcast::Receiver<Frame>) -> Vec<Value> {
        let mut out = Vec::new();
        while let Ok(frame) = feed.try_recv() {
            match frame {
                Frame::Event { json, .. } | Frame::Live { json, .. } => {
                    out.push(parse(json.as_str()));
                }
            }
        }
        out
    }

    fn kinds(events: &[Value]) -> Vec<String> {
        events
            .iter()
            .map(|e| e["t"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn keys_map_to_people_and_strangers_get_nothing() {
        let r = room(60_000);
        assert_eq!(r.who("key-of-person-zero-000"), Some(0));
        assert_eq!(r.who("key-of-person-one-1111"), Some(1));
        assert_eq!(r.who("nope"), None);
        assert_eq!(r.who(""), None);
        assert!(r.is_lobby("lobby-secret"));
        assert!(!r.is_lobby("key-of-person-zero-000"));
    }

    #[test]
    fn the_clock_starts_only_when_both_are_in() {
        let r = room(60_000);
        let (hello, _, _a) = r.attach(0, 0);
        assert_eq!(parse(hello.as_str())["phase"], "waiting");
        assert_eq!(r.phase(), Phase::Waiting);
        let (hello, seq, _b) = r.attach(1, 0);
        let hello = parse(hello.as_str());
        assert_eq!(hello["phase"], "live");
        assert_eq!(hello["you"], 1);
        assert_eq!(
            hello["ends"].as_u64().unwrap() - hello["starts"].as_u64().unwrap(),
            60_000
        );
        // join, join, phase, why, opener
        assert_eq!(seq, 5);
        assert_eq!(
            kinds(hello["log"].as_array().unwrap()),
            ["sys", "sys", "phase", "host", "host"]
        );
    }

    #[test]
    fn the_other_persons_key_never_leaves_the_room() {
        let r = room(60_000);
        let (hello, _, _a) = r.attach(0, 0);
        assert!(!hello.as_str().contains("key-of-person"));
        assert!(!r.status().to_string().contains("key-of-person"));
    }

    #[test]
    fn chat_is_ordered_cleaned_and_safe_to_retry() {
        let r = room(60_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        drain(&mut a);
        assert!(chat(&r, 0, "a1", "  hello\u{202E}  ").is_none());
        assert!(chat(&r, 0, "a1", "hello").is_none(), "a retry is swallowed");
        assert!(chat(&r, 1, "b1", "hi").is_none());
        let got = drain(&mut a);
        let chats: Vec<&Value> = got.iter().filter(|e| e["t"] == "chat").collect();
        assert_eq!(chats.len(), 2);
        assert_eq!(chats[0]["text"], "hello");
        assert_eq!(chats[0]["by"], 0);
        assert_eq!(chats[1]["by"], 1);
        assert!(chats[0]["s"].as_u64() < chats[1]["s"].as_u64());
        let err = parse(&chat(&r, 0, "a2", "   ").unwrap());
        assert_eq!(err["code"], "empty");
    }

    #[test]
    fn a_flood_is_slowed_down() {
        let r = room(60_000);
        let (_, _, _a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        let refused = (0..20)
            .filter(|i| chat(&r, 0, &format!("m{i}"), "x").is_some())
            .count();
        assert!(
            refused >= 12,
            "only a short burst gets through, refused {refused}"
        );
        assert!(
            chat(&r, 1, "other", "still fine").is_none(),
            "the other person is not affected"
        );
    }

    #[test]
    fn a_reconnect_gets_exactly_what_it_missed() {
        let r = room(60_000);
        let (_, seen, a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        drop(a);
        r.detach(0);
        chat(&r, 1, "b1", "are you there");
        chat(&r, 1, "b2", "hello?");
        let (hello, _, _a) = r.attach(0, seen);
        let hello = parse(hello.as_str());
        let log = hello["log"].as_array().unwrap();
        assert!(log.iter().all(|e| e["s"].as_u64().unwrap() > seen));
        assert_eq!(log.iter().filter(|e| e["t"] == "chat").count(), 2);
        // A number this room never issued means "send everything".
        let (hello, _, _c) = r.attach(0, 99_999);
        assert_eq!(parse(hello.as_str())["log"][0]["s"], 1);
    }

    #[test]
    fn sealed_answers_stay_private_until_both_are_in() {
        let r = room(60_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        drain(&mut a);
        r.handle(
            0,
            ClientMsg::Seal {
                slot: "q1".into(),
                v: json!("ramen"),
            },
        );
        let got = drain(&mut a);
        assert_eq!(kinds(&got), ["sealed"]);
        assert!(
            !got[0].to_string().contains("ramen"),
            "the value is not sent early"
        );
        // Changing your mind before the reveal is allowed and is not announced again.
        r.handle(
            0,
            ClientMsg::Seal {
                slot: "q1".into(),
                v: json!("sushi"),
            },
        );
        assert_eq!(drain(&mut a), [] as [Value; 0]);
        r.handle(
            1,
            ClientMsg::Seal {
                slot: "q1".into(),
                v: json!("curry"),
            },
        );
        let got = drain(&mut a);
        assert_eq!(kinds(&got), ["sealed", "reveal"]);
        assert_eq!(got[1]["v"], json!(["sushi", "curry"]));
        // After the reveal the slot is closed.
        r.handle(
            0,
            ClientMsg::Seal {
                slot: "q1".into(),
                v: json!("late"),
            },
        );
        assert_eq!(drain(&mut a), [] as [Value; 0]);
    }

    #[test]
    fn reactions_add_once_and_come_off() {
        let r = room(60_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        chat(&r, 0, "a1", "hello");
        let id = drain(&mut a)
            .into_iter()
            .find(|e| e["t"] == "chat")
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let mark = |on| ClientMsg::Mreact {
            id: id.clone(),
            e: "❤️".into(),
            on,
        };
        r.handle(1, mark(true));
        r.handle(1, mark(true));
        r.handle(
            1,
            ClientMsg::Mreact {
                id: id.clone(),
                e: "not an emoji".into(),
                on: true,
            },
        );
        r.handle(
            1,
            ClientMsg::Mreact {
                id: "m999".into(),
                e: "❤️".into(),
                on: true,
            },
        );
        assert_eq!(drain(&mut a).len(), 1);
        r.handle(1, mark(false));
        r.handle(1, mark(false));
        let got = drain(&mut a);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["on"], false);
    }

    #[test]
    fn momentary_things_are_not_stored_and_not_echoed() {
        let r = room(60_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (hello, seq_before, _b) = r.attach(1, 0);
        let la = parse(hello.as_str())["la"].as_u64().unwrap();
        drain(&mut a);
        r.handle(
            0,
            ClientMsg::Live {
                k: "typing".into(),
                d: json!(true),
            },
        );
        r.handle(
            0,
            ClientMsg::Live {
                k: "shout".into(),
                d: json!(true),
            },
        );
        assert_eq!(r.events_after(seq_before).len(), 0, "nothing was stored");
        let got = drain(&mut a);
        assert_eq!(got.len(), 1, "the unknown kind was dropped");
        assert_eq!(got[0]["k"], "typing");
        // A page on held requests does not get its own typing back.
        assert!(r.poll(0, seq_before, la).is_none());
        assert!(r.poll(1, seq_before, la).unwrap().contains("typing"));
    }

    #[test]
    fn leaving_ends_it_for_both() {
        let r = room(60_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        drain(&mut a);
        r.handle(1, ClientMsg::Leave);
        let got = drain(&mut a);
        assert_eq!(kinds(&got), ["sys", "phase"]);
        assert_eq!(got[1]["why"], "left");
        assert_eq!(got[1]["by"], 1);
        assert_eq!(
            parse(&chat(&r, 0, "late", "wait").unwrap())["code"],
            "closed"
        );
    }

    #[test]
    fn only_a_reported_room_gives_up_its_chat() {
        let r = room(60_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        chat(&r, 1, "b1", "something unkind");
        assert!(r.export().is_none());
        assert_eq!(r.status()["reported"], false);
        assert!(
            !r.status().to_string().contains("unkind"),
            "status carries no words"
        );
        drain(&mut a);
        let reply = r.handle(
            0,
            ClientMsg::Report {
                reason: "rude".into(),
                note: "see chat".into(),
            },
        );
        assert_eq!(parse(&reply.unwrap())["t"], "reported");
        let got = drain(&mut a);
        assert_eq!(
            got.last().unwrap()["why"],
            "ended",
            "the other side sees a plain ending"
        );
        assert!(!got.iter().any(|e| e.to_string().contains("report")));
        let export = r.export().unwrap();
        assert_eq!(export["chat"][0]["text"], "something unkind");
        assert_eq!(export["report"]["by"], 0);
        assert_eq!(r.status()["reported"], true);
    }

    #[test]
    fn the_activity_record_is_kept_small() {
        let r = room(60_000);
        let (_, _, _a) = r.attach(0, 0);
        r.handle(
            0,
            ClientMsg::Score {
                d: json!({"rounds": 8, "synced": 5}),
            },
        );
        assert_eq!(r.status()["summary"]["synced"], 5);
        r.handle(
            0,
            ClientMsg::Score {
                d: json!("x".repeat(5000)),
            },
        );
        assert_eq!(
            r.status()["summary"]["synced"],
            5,
            "an oversized record is ignored"
        );
    }

    #[tokio::test]
    async fn time_runs_out() {
        let r = room(3_000);
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, _b) = r.attach(1, 0);
        r.run();
        tokio::time::sleep(std::time::Duration::from_millis(2_400)).await;
        assert_eq!(r.phase(), Phase::Ending);
        tokio::time::sleep(std::time::Duration::from_millis(1_000)).await;
        assert_eq!(r.phase(), Phase::Over);
        let got = drain(&mut a);
        let phases: Vec<&Value> = got.iter().filter(|e| e["t"] == "phase").collect();
        assert_eq!(
            phases
                .iter()
                .map(|p| p["p"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["live", "ending", "over"]
        );
        assert_eq!(phases[2]["why"], "time");
        assert!(got.iter().any(|e| e["t"] == "host" && e["k"] == "last"));
        assert!(
            got.iter().any(|e| e["t"] == "host" && e["k"] == "nudge"),
            "the host spoke in the silence"
        );
        assert!(r.status()["elapsed_ms"].as_u64().unwrap() >= 3_000);
    }

    #[tokio::test]
    async fn a_no_show_ends_the_room() {
        let r = room(60_000); // join_wait is 2 s in the sample
        let (_, _, _a) = r.attach(0, 0);
        r.run();
        tokio::time::sleep(std::time::Duration::from_millis(2_600)).await;
        assert_eq!(r.phase(), Phase::Over);
        assert_eq!(r.status()["why"], "noshow");
    }

    #[tokio::test]
    async fn a_dropped_connection_is_forgiven_then_counted_as_gone() {
        let r = room(60_000); // away is 2 s in the sample
        let (_, _, mut a) = r.attach(0, 0);
        let (_, _, b) = r.attach(1, 0);
        r.run();
        drop(b);
        r.detach(1);
        tokio::time::sleep(std::time::Duration::from_millis(1_400)).await;
        assert_eq!(r.phase(), Phase::Live);
        let got = drain(&mut a);
        assert!(got.iter().any(|e| e["t"] == "sys" && e["k"] == "away"));
        // Coming back in time keeps the session going.
        let (_, _, b) = r.attach(1, 0);
        assert!(
            drain(&mut a)
                .iter()
                .any(|e| e["t"] == "sys" && e["k"] == "back")
        );
        drop(b);
        r.detach(1);
        tokio::time::sleep(std::time::Duration::from_millis(2_600)).await;
        assert_eq!(r.phase(), Phase::Over);
        assert_eq!(r.status()["why"], "gone");
        assert_eq!(r.status()["by"], 1);
    }
}
