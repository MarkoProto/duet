//! The lobby: who is here, who is waiting, who is in a room, who said yes.
//!
//! All of it sits behind one lock that is never held across a wait. Each person's page holds one
//! request open and is answered the moment their own screen changes.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use duet_room::text::{clean_name, clean_text, token};
use duet_room::{Bi, Person, RoomConfig, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use crate::daytona::DaytonaHost;
use crate::llm::Llm;
use crate::matcher::{PAIR_BONUS, Weights, solve};
use crate::planner::{self, Pair, Plan};
use crate::profile::{Answers, Profile, bar, fit, patience};
use crate::rooms::{Host, LocalRooms, Proxied, Ready, Receipt};
use crate::settings::{RoomsMode, Settings};
use crate::{notes, reader};

const COLORS: [&str; 2] = ["#ffb84d", "#ff4f8b"];
/// A page is "here" while it holds a request open, or made one this recently.
const HERE_MS: u64 = 9_000;
/// Someone waiting whose page has been gone this long is taken out of the queue.
const GONE_MS: u64 = 25_000;
const MAX_USERS: usize = 20_000;
/// The most rooms the admin can ask to have built ahead.
pub const MAX_WARM: usize = 8;

/// How long each step of building a room took, by step name (`None`: not reached).
type StepTimes = Vec<(&'static str, Option<u64>)>;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Has a name, has not answered the questions.
    Ask,
    /// Answered, not doing anything right now.
    Home,
    /// In the queue.
    Wait,
    Build(String),
    Room(String),
    Vote(String),
    Result(String),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Met {
    pub times: u32,
    /// They did not both say yes, or one reported the other. Never paired again.
    pub closed: bool,
    pub mutual: bool,
}

pub struct User {
    pub id: String,
    pub token: String,
    pub name: String,
    pub lang: String,
    pub answers: Option<Answers>,
    pub profile: Option<Profile>,
    pub stage: Stage,
    pub queued_ms: u64,
    pub left_out: u32,
    pub met: HashMap<String, Met>,
    pub liked: Vec<(String, i32)>,
    pub last_activity: Option<String>,
    pub created_ms: u64,
    pub seen_ms: u64,
    /// Requests this person's pages are holding open right now.
    pub polls: u32,
    pub ver: u64,
    pub wake: watch::Sender<u64>,
    /// Something to say once on the next screen: `noshow`, `missed`, `failed`, `expired`.
    pub note: Option<&'static str>,
}

impl User {
    fn bump(&mut self) {
        self.ver += 1;
        self.wake.send_replace(self.ver);
    }

    fn here(&self, now: u64) -> bool {
        self.polls > 0 || now.saturating_sub(self.seen_ms) < HERE_MS
    }

    fn idle(&self) -> bool {
        matches!(self.stage, Stage::Home | Stage::Result(_)) && self.profile.is_some()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Vote {
    pub liked: bool,
    pub again: bool,
}

struct Mark {
    key: &'static str,
    started: Instant,
    ms: Option<u64>,
}

#[expect(
    clippy::struct_excessive_bools,
    reason = "independent facts about one session, not a state machine in disguise"
)]
pub struct Session {
    pub id: String,
    pub users: [String; 2],
    pub long: bool,
    pub created_ms: u64,
    born: Instant,
    marks: Vec<Mark>,
    pub kind: Option<String>,
    pub title: Option<Bi>,
    pub why: Option<Bi>,
    pub by_model: bool,
    pub runtime: Option<(String, String)>,
    pub urls: Option<[String; 2]>,
    pub live: bool,
    pub ends_ms: u64,
    pub over: Option<(String, Option<u8>)>,
    pub votes: [Option<Vote>; 2],
    pub vote_deadline_ms: u64,
    pub receipt: Option<Receipt>,
    pub ready_ms: Option<u64>,
    /// Whether its room was one built ahead.
    pub warm: bool,
    /// Woken when something about this session should be looked at now rather than soon.
    pub check: Arc<Notify>,
}

/// A room built before anyone needs it, so that a pair's room opens at once. Golden Sea built
/// seven sandboxes at the same time; these are built the same way, a batch at once.
pub struct Warm {
    pub slot: String,
    pub batch: u64,
    pub ready: Option<Ready>,
    failed: Option<(String, Instant)>,
    born: Instant,
    ready_at: Option<Instant>,
    marks: Vec<Mark>,
}

/// One "build these at the same time": when it started and when the last one was ready.
struct Batch {
    id: u64,
    n: usize,
    started: Instant,
    done_ms: Option<u64>,
    failed: usize,
}

/// Records a step starting or finishing. Returns how long it took when it just finished.
fn step(marks: &mut Vec<Mark>, key: &'static str, done: bool) -> Option<u64> {
    match marks.iter_mut().find(|m| m.key == key) {
        Some(m) if done && m.ms.is_none() => {
            let ms = m.started.elapsed().as_millis() as u64;
            m.ms = Some(ms);
            Some(ms)
        }
        Some(_) => None,
        None => {
            marks.push(Mark {
                key,
                started: Instant::now(),
                ms: done.then_some(0),
            });
            None
        }
    }
}

fn marks_view(marks: &[Mark]) -> Vec<Value> {
    marks
        .iter()
        .map(|m| json!({"k": m.key, "done": m.ms.is_some(), "ms": m.ms}))
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reservation {
    pub id: String,
    pub users: [String; 2],
    pub at_ms: u64,
    pub ms: u64,
    /// What they did the first time, so the second time is something else.
    pub activity: String,
    /// What both see: what they did together and what is next. Written from the record, never
    /// from the chat.
    #[serde(default)]
    pub note: Option<Bi>,
    #[serde(default)]
    pub note_by_model: bool,
}

/// What is kept of a finished session: what was done and how it ended. Never what was said.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Closed {
    pub id: String,
    pub at_ms: u64,
    pub users: [String; 2],
    pub activity: String,
    pub long: bool,
    pub why: String,
    pub secs: u64,
    pub mutual: Option<bool>,
    pub liked: [Option<bool>; 2],
    pub by_model: bool,
    pub ready_ms: Option<u64>,
    pub receipt: Option<Value>,
    /// The activity's own record, such as rounds played and how many matched.
    pub summary: Value,
    pub counts: Value,
    pub reported: bool,
}

#[derive(Default)]
struct Samples(VecDeque<u64>);

impl Samples {
    fn push(&mut self, v: u64) {
        if self.0.len() == 256 {
            self.0.pop_front();
        }
        self.0.push_back(v);
    }

    fn view(&self) -> Value {
        if self.0.is_empty() {
            return Value::Null;
        }
        let mut sorted: Vec<u64> = self.0.iter().copied().collect();
        sorted.sort_unstable();
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        json!({"n": sorted.len(), "p50": at(0.5), "p95": at(0.95), "last": self.0.back()})
    }
}

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Totals {
    pub sessions: u64,
    pub mutual: u64,
    pub reports: u64,
    pub people: u64,
}

#[derive(Default)]
struct Stats {
    ready_ms: Samples,
    plan_ms: Samples,
    solve_us: Samples,
    destroy_ms: Samples,
    model_plans: u64,
    rule_plans: u64,
    fallbacks: u64,
    /// Reading a person's own words into the taps.
    read_ms: Samples,
    model_reads: u64,
    rule_reads: u64,
    /// Writing the note after both said yes.
    note_ms: Samples,
    model_notes: u64,
    rule_notes: u64,
    /// Each step of building a room, as Golden Sea timed its sandboxes.
    step_ms: HashMap<&'static str, Samples>,
    /// Sessions that got a room built ahead.
    warm_taken: u64,
}

#[derive(Default)]
pub struct State {
    pub users: HashMap<String, User>,
    tokens: HashMap<String, String>,
    pub sessions: HashMap<String, Session>,
    pub reservations: Vec<Reservation>,
    pub history: VecDeque<Closed>,
    pub totals: Totals,
    stats: Stats,
    pub warm: Vec<Warm>,
    /// Set by the admin; until then the setting decides.
    warm_target: Option<usize>,
    last_batch: Option<Batch>,
    batches: u64,
}

#[derive(Serialize, Deserialize)]
struct SavedUser {
    id: String,
    token: String,
    name: String,
    lang: String,
    answers: Option<Answers>,
    #[serde(default)]
    left_out: u32,
    #[serde(default)]
    met: HashMap<String, Met>,
    #[serde(default)]
    liked: Vec<(String, i32)>,
    #[serde(default)]
    last_activity: Option<String>,
    created_ms: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    #[serde(default)]
    users: Vec<SavedUser>,
    #[serde(default)]
    reservations: Vec<Reservation>,
    #[serde(default)]
    history: Vec<Closed>,
    #[serde(default)]
    totals: Totals,
}

pub struct App {
    pub cfg: Settings,
    pub state: Mutex<State>,
    pub rooms: LocalRooms,
    /// Daytona rooms reached through the lobby (see `rooms::Proxied`).
    pub proxied: Proxied,
    local: Host,
    remote: Option<Host>,
    pub llm: Option<Llm>,
    /// Wakes the loop that forms pairs.
    pub round: Notify,
    /// Wakes the loop that keeps rooms ready ahead.
    warm_wake: Notify,
    dirty: AtomicBool,
}

/// What a page's request needs to answer: either a screen now, or something to wait on.
pub enum Poll {
    Nobody,
    Now(Value),
    Wait(watch::Receiver<u64>),
}

fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map_or_else(|| "?".into(), |c| c.to_uppercase().collect())
}

impl State {
    pub fn user_of(&self, token: &str) -> Option<&str> {
        self.tokens.get(token).map(String::as_str)
    }

    fn insert(&mut self, user: User) {
        self.tokens.insert(user.token.clone(), user.id.clone());
        self.users.insert(user.id.clone(), user);
    }

    fn bump(&mut self, id: &str) {
        if let Some(u) = self.users.get_mut(id) {
            u.bump();
        }
    }

    fn waiting(&self) -> usize {
        self.users
            .values()
            .filter(|u| u.stage == Stage::Wait)
            .count()
    }

    /// Everyone in the queue sees how many are there, so they are told when that changes.
    fn bump_waiting(&mut self) {
        for u in self.users.values_mut().filter(|u| u.stage == Stage::Wait) {
            u.bump();
        }
    }

    fn reservation_of(&self, id: &str) -> Option<&Reservation> {
        self.reservations
            .iter()
            .find(|r| r.users.iter().any(|u| u == id))
    }

    fn reservation_view(&self, cfg: &Settings, id: &str, now: u64) -> Value {
        let Some(r) = self.reservation_of(id) else {
            return Value::Null;
        };
        let other = r
            .users
            .iter()
            .find(|u| *u != id)
            .and_then(|u| self.users.get(u));
        json!({
            "with": other.map(|u| u.name.clone()),
            "at_ms": r.at_ms,
            "mins": r.ms / 60_000,
            "note": r.note,
            "open": now + cfg.rematch_early_ms >= r.at_ms,
            "here": other.is_some_and(|u| u.here(now) && u.idle()),
        })
    }

    /// The screen this person should be looking at, with everything it shows.
    pub fn view(&self, cfg: &Settings, sponsors: &Value, id: &str) -> Value {
        let now = now_ms();
        let Some(u) = self.users.get(id) else {
            return json!({"screen": "hello", "v": 0});
        };
        let partner = |s: &Session| {
            let i = usize::from(s.users[0] == id);
            let name = self
                .users
                .get(&s.users[i])
                .map_or("?".to_string(), |p| p.name.clone());
            (json!({"name": name, "color": COLORS[i]}), COLORS[1 - i])
        };
        let mut v = match &u.stage {
            Stage::Ask => json!({"screen": "ask", "answers": u.answers}),
            Stage::Home => {
                json!({"screen": "home", "reservation": self.reservation_view(cfg, id, now)})
            }
            Stage::Wait => {
                json!({"screen": "wait", "since_ms": u.queued_ms, "here": self.waiting(), "reservation": self.reservation_view(cfg, id, now)})
            }
            Stage::Build(sid) | Stage::Room(sid) | Stage::Vote(sid) | Stage::Result(sid) => {
                match self.sessions.get(sid) {
                    None => {
                        json!({"screen": "home", "reservation": self.reservation_view(cfg, id, now)})
                    }
                    Some(s) => {
                        let (other, my_color) = partner(s);
                        let me_at = usize::from(s.users[1] == id);
                        let activity = s
                            .kind
                            .as_ref()
                            .map(|k| json!({"kind": k, "title": s.title}));
                        let runtime = s
                            .runtime
                            .as_ref()
                            .map(|(kind, rid)| json!({"kind": kind, "id": rid}));
                        match &u.stage {
                            Stage::Build(_) => json!({
                                "screen": "build", "partner": other, "color": my_color, "why": s.why, "activity": activity, "runtime": runtime,
                                "marks": s.marks.iter().map(|m| json!({"k": m.key, "done": m.ms.is_some(), "ms": m.ms})).collect::<Vec<_>>(),
                                "long": s.long, "by_model": s.by_model, "mins": (if s.long { cfg.long_session_ms } else { cfg.session_ms }) as f64 / 60_000.0,
                            }),
                            Stage::Room(_) => json!({
                                "screen": "room", "partner": other, "color": my_color, "url": s.urls.as_ref().map(|u| u[me_at].clone()), "why": s.why,
                                "activity": activity, "runtime": runtime, "long": s.long, "ends_ms": s.ends_ms,
                            }),
                            Stage::Vote(_) => json!({
                                "screen": "vote", "partner": other, "color": my_color, "activity": activity, "receipt": s.receipt,
                                "deadline_ms": s.vote_deadline_ms, "voted": s.votes[me_at].is_some(),
                                "over": s.over.as_ref().map(|(why, by)| json!({"why": why, "mine": by.map(|b| b as usize == me_at)})),
                            }),
                            _ => {
                                let mutual = s.votes.iter().all(|v| v.is_some_and(|v| v.again));
                                json!({
                                    "screen": "result", "mutual": mutual, "color": my_color,
                                    // Who the other person was is repeated only when both said yes.
                                    "partner": if mutual { other } else { Value::Null },
                                    "reservation": self.reservation_view(cfg, id, now),
                                })
                            }
                        }
                    }
                }
            }
        };
        let obj = v.as_object_mut().expect("views are objects");
        obj.insert("v".into(), u.ver.into());
        obj.insert("now".into(), now.into());
        obj.insert(
            "me".into(),
            json!({"id": u.id, "name": u.name, "lang": u.lang, "ready": u.profile.is_some()}),
        );
        obj.insert(
            "link".into(),
            format!("{}/me/{}", cfg.public_url, u.token).into(),
        );
        obj.insert("note".into(), u.note.into());
        obj.insert("sponsors".into(), sponsors.clone());
        v
    }

    fn open_session(&mut self, a: &str, b: &str, long: bool) -> String {
        let id = format!("s_{}", token(6));
        let session = Session {
            id: id.clone(),
            users: [a.to_string(), b.to_string()],
            long,
            created_ms: now_ms(),
            born: Instant::now(),
            marks: Vec::new(),
            kind: None,
            title: None,
            why: None,
            by_model: false,
            runtime: None,
            urls: None,
            live: false,
            ends_ms: 0,
            over: None,
            votes: [None, None],
            vote_deadline_ms: 0,
            receipt: None,
            ready_ms: None,
            warm: false,
            check: Arc::new(Notify::new()),
        };
        self.sessions.insert(id.clone(), session);
        for uid in [a, b] {
            if let Some(u) = self.users.get_mut(uid) {
                u.stage = Stage::Build(id.clone());
                u.left_out = 0;
                u.note = None;
                u.bump();
            }
        }
        id
    }

    /// Forms this round's pairs, if it is time. Returns the sessions it opened.
    fn round(&mut self, cfg: &Settings) -> Vec<String> {
        let now = now_ms();
        // A page that went away does not hold a place in the queue.
        let mut changed = false;
        for u in self.users.values_mut() {
            if u.stage == Stage::Wait && !u.here(now) && now.saturating_sub(u.seen_ms) > GONE_MS {
                u.stage = Stage::Home;
                u.bump();
                changed = true;
            }
        }
        let mut ids: Vec<String> = self
            .users
            .values()
            .filter(|u| u.stage == Stage::Wait && u.profile.is_some() && u.here(now))
            .map(|u| u.id.clone())
            .collect();
        if ids.len() < 2 {
            if changed {
                self.bump_waiting();
            }
            return Vec::new();
        }
        ids.sort_by_key(|id| (self.users[id].queued_ms, id.clone()));
        let oldest = self.users[&ids[0]].queued_ms;
        let newest = self.users[ids.last().expect("two or more")].queued_ms;
        // Wait for arrivals to pause, so people who come together are solved together.
        if now.saturating_sub(newest) < cfg.settle_ms
            && now.saturating_sub(oldest) < cfg.max_gather_ms
        {
            return Vec::new();
        }
        let started = Instant::now();
        let mut weights = Weights::new(ids.len());
        for i in 0..ids.len() {
            for j in i + 1..ids.len() {
                let (a, b) = (&self.users[&ids[i]], &self.users[&ids[j]]);
                let history = a.met.get(&b.id);
                if history.is_some_and(|m| m.closed)
                    || self
                        .reservations
                        .iter()
                        .any(|r| r.users.contains(&a.id) && r.users.contains(&b.id))
                {
                    continue;
                }
                let (Some(pa), Some(pb)) = (&a.profile, &b.profile) else {
                    continue;
                };
                let Some(score) = fit(pa, pb) else { continue };
                let (wa, wb) = (
                    now.saturating_sub(a.queued_ms),
                    now.saturating_sub(b.queued_ms),
                );
                if score < bar(wa.max(wb)) {
                    continue;
                }
                let waited = f32::midpoint(patience(wa, a.left_out), patience(wb, b.left_out));
                let again = 0.3 * history.map_or(0, |m| m.times.min(2)) as f32;
                weights.set(
                    i,
                    j,
                    PAIR_BONUS + ((score + waited - again) * 10_000.0) as i64,
                );
            }
        }
        let pairs = solve(&weights);
        self.stats
            .solve_us
            .push(started.elapsed().as_micros() as u64);
        let mut opened = Vec::new();
        let mut paired = vec![false; ids.len()];
        for (i, j) in pairs {
            paired[i] = true;
            paired[j] = true;
            opened.push(self.open_session(&ids[i], &ids[j], false));
        }
        for (i, id) in ids.iter().enumerate() {
            if !paired[i]
                && let Some(u) = self.users.get_mut(id)
            {
                // Counted once per round they were passed over, and it raises their priority.
                u.left_out += 1;
            }
        }
        self.bump_waiting();
        opened
    }

    /// Starts the longer sessions whose time has come and whose two people are both here.
    fn due(&mut self, cfg: &Settings) -> Vec<String> {
        let now = now_ms();
        let mut opened = Vec::new();
        let mut keep = Vec::new();
        for r in std::mem::take(&mut self.reservations) {
            let people: Vec<&User> = r.users.iter().filter_map(|u| self.users.get(u)).collect();
            if people.len() < 2 || now > r.at_ms + cfg.rematch_window_ms {
                for uid in &r.users {
                    if let Some(u) = self.users.get_mut(uid) {
                        u.note = Some("expired");
                        u.bump();
                    }
                }
                continue;
            }
            let ready = now + cfg.rematch_early_ms >= r.at_ms
                && people.iter().all(|u| u.here(now) && u.idle());
            if ready {
                let sid = self.open_session(&r.users[0], &r.users[1], true);
                // The second time is something else than the first.
                for uid in &r.users {
                    if let Some(u) = self.users.get_mut(uid) {
                        u.last_activity = Some(r.activity.clone());
                    }
                }
                opened.push(sid);
            } else {
                keep.push(r);
            }
        }
        self.reservations = keep;
        opened
    }
}

impl App {
    pub fn new(cfg: Settings) -> Arc<Self> {
        let rooms: LocalRooms = Arc::default();
        let local = Host::Local {
            rooms: rooms.clone(),
            public_url: cfg.public_url.clone(),
        };
        let remote = match (cfg.rooms, &cfg.daytona) {
            (RoomsMode::Local, _) | (_, None) => None,
            (_, Some(d)) => {
                // A short, stable name for this lobby, so its clean-up leaves other lobbies alone.
                let name = cfg
                    .public_url
                    .bytes()
                    .fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
                        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
                    });
                Some(Host::Daytona(Box::new(DaytonaHost::new(
                    d,
                    cfg.room_bin_dir.clone(),
                    format!("{name:016x}"),
                ))))
            }
        };
        let llm = cfg
            .llm
            .as_ref()
            .map(|l| Llm::new(&l.url, l.model.clone(), l.key.clone()));
        Self::with(cfg, rooms, local, remote, llm)
    }

    /// For tests: the same lobby with its outside parts supplied.
    pub fn with(
        cfg: Settings,
        rooms: LocalRooms,
        local: Host,
        remote: Option<Host>,
        llm: Option<Llm>,
    ) -> Arc<Self> {
        let mut state = State::default();
        if let Ok(bytes) = std::fs::read(cfg.data_dir.join("state.json")) {
            match serde_json::from_slice::<Saved>(&bytes) {
                Ok(saved) => {
                    for s in saved.users {
                        let profile = s.answers.as_ref().map(Answers::profile);
                        let first = if profile.is_some() {
                            Stage::Home
                        } else {
                            Stage::Ask
                        };
                        state.insert(User {
                            id: s.id,
                            token: s.token,
                            name: s.name,
                            lang: s.lang,
                            answers: s.answers,
                            profile,
                            stage: first,
                            queued_ms: 0,
                            left_out: s.left_out,
                            met: s.met,
                            liked: s.liked,
                            last_activity: s.last_activity,
                            created_ms: s.created_ms,
                            seen_ms: 0,
                            polls: 0,
                            ver: 1,
                            wake: watch::channel(1).0,
                            note: None,
                        });
                    }
                    state.reservations = saved.reservations;
                    state.history = saved.history.into();
                    state.totals = saved.totals;
                }
                Err(e) => {
                    let aside = cfg
                        .data_dir
                        .join(format!("state.json.unreadable-{}", now_ms()));
                    let _ = std::fs::rename(cfg.data_dir.join("state.json"), &aside);
                    eprintln!(
                        "duet: could not read saved state ({e}); kept it as {} and started empty",
                        aside.display()
                    );
                }
            }
        }
        Arc::new(Self {
            cfg,
            state: Mutex::new(state),
            rooms,
            proxied: Proxied::default(),
            local,
            remote,
            llm,
            round: Notify::new(),
            warm_wake: Notify::new(),
            dirty: AtomicBool::new(false),
        })
    }

    pub fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn host(&self) -> &Host {
        self.remote.as_ref().unwrap_or(&self.local)
    }

    pub fn daytona(&self) -> Option<&DaytonaHost> {
        match &self.remote {
            Some(Host::Daytona(d)) => Some(d),
            _ => None,
        }
    }

    /// Which sponsor parts are really in use, shown on every screen.
    pub fn sponsors(&self) -> Value {
        json!({"rooms": self.host().kind(), "ai": if self.llm.is_some() { "nosana" } else { "rules" }})
    }

    fn touch(&self) {
        self.dirty.store(true, Ordering::Relaxed);
    }

    // ---- what a page can ask for ---------------------------------------------------------

    /// A new person, or a new name for one who is already known. Returns their secret.
    pub fn join(
        &self,
        known: Option<&str>,
        name: &str,
        lang: &str,
    ) -> Result<String, &'static str> {
        let name = clean_name(name).ok_or("name")?;
        let lang = if lang == "ja" { "ja" } else { "en" };
        let mut st = self.lock();
        if let Some(id) = known.and_then(|t| st.user_of(t)).map(str::to_string) {
            let u = st.users.get_mut(&id).expect("known user");
            u.name = name;
            u.lang = lang.to_string();
            u.bump();
            let secret = u.token.clone();
            drop(st);
            self.touch();
            return Ok(secret);
        }
        if st.users.len() >= MAX_USERS {
            return Err("full");
        }
        let user = User {
            id: format!("u_{}", token(6)),
            token: token(24),
            name,
            lang: lang.to_string(),
            answers: None,
            profile: None,
            stage: Stage::Ask,
            queued_ms: 0,
            left_out: 0,
            met: HashMap::new(),
            liked: Vec::new(),
            last_activity: None,
            created_ms: now_ms(),
            seen_ms: now_ms(),
            polls: 0,
            ver: 1,
            wake: watch::channel(1).0,
            note: None,
        };
        let secret = user.token.clone();
        st.insert(user);
        st.totals.people += 1;
        drop(st);
        self.touch();
        Ok(secret)
    }

    pub fn set_lang(&self, token: &str, lang: &str) {
        let mut st = self.lock();
        if let Some(id) = st.user_of(token).map(str::to_string) {
            let u = st.users.get_mut(&id).expect("known user");
            u.lang = if lang == "ja" {
                "ja".into()
            } else {
                "en".into()
            };
            u.bump();
            self.touch();
        }
    }

    /// The answers to the questions. Puts the person in the queue.
    pub fn answer(&self, token: &str, answers: Answers) -> Result<(), &'static str> {
        let answers = answers.checked().ok_or("answers")?;
        let mut st = self.lock();
        let id = st.user_of(token).ok_or("nobody")?.to_string();
        let u = st.users.get_mut(&id).expect("known user");
        if !matches!(u.stage, Stage::Ask | Stage::Home | Stage::Result(_)) {
            return Err("busy");
        }
        u.profile = Some(answers.profile());
        u.answers = Some(answers);
        u.stage = Stage::Wait;
        u.queued_ms = now_ms();
        u.note = None;
        st.bump_waiting();
        drop(st);
        self.touch();
        self.round.notify_one();
        Ok(())
    }

    /// "Say it in your own words": the taps, read from a short note. The note itself is not kept;
    /// the page shows what was read, and the person sends the taps as usual.
    pub async fn understand(&self, token: &str, note: &str) -> Result<Value, &'static str> {
        {
            let st = self.lock();
            let id = st.user_of(token).ok_or("nobody")?;
            let u = st.users.get(id).ok_or("nobody")?;
            if !matches!(u.stage, Stage::Ask | Stage::Home | Stage::Result(_)) {
                return Err("busy");
            }
        }
        let note = clean_text(&note.replace('\n', " "), reader::MAX_CHARS).ok_or("note")?;
        let rules = reader::rules(&note);
        let Some(llm) = &self.llm else {
            self.lock().stats.rule_reads += 1;
            return Ok(rules.view());
        };
        let (system, user) = reader::question(&note);
        let budget = Duration::from_millis(self.cfg.plan_budget_ms.min(10_000));
        let guess = match llm.ask_at(&system, &user, 300, budget, 0.0).await {
            Ok(answer) => {
                let guess = reader::merge(rules, &answer.json);
                let mut st = self.lock();
                st.stats.read_ms.push(answer.ms);
                if guess.by_model > 0 {
                    st.stats.model_reads += 1;
                } else {
                    st.stats.rule_reads += 1;
                }
                guess
            }
            Err(e) => {
                eprintln!("duet: the model did not read this note: {e}");
                self.lock().stats.rule_reads += 1;
                rules
            }
        };
        Ok(guess.view())
    }

    /// Into the queue (`true`) or out of it.
    pub fn queue(&self, token: &str, on: bool) -> Result<(), &'static str> {
        let mut st = self.lock();
        let id = st.user_of(token).ok_or("nobody")?.to_string();
        let u = st.users.get_mut(&id).expect("known user");
        match (on, &u.stage) {
            (true, Stage::Home | Stage::Result(_)) if u.profile.is_some() => {
                u.stage = Stage::Wait;
                u.queued_ms = now_ms();
                u.note = None;
            }
            // Back to the questions: to wait again, or to change the answers (the old ones stay filled in).
            (true, Stage::Home | Stage::Result(_)) | (false, Stage::Home) => u.stage = Stage::Ask,
            (false, Stage::Wait | Stage::Result(_)) => u.stage = Stage::Home,
            _ => return Err("busy"),
        }
        st.bump_waiting();
        st.bump(&id);
        drop(st);
        self.round.notify_one();
        Ok(())
    }

    /// The two questions after a session.
    pub fn vote(&self, token: &str, liked: bool, again: bool) -> Result<(), &'static str> {
        let mut st = self.lock();
        let id = st.user_of(token).ok_or("nobody")?.to_string();
        let Stage::Vote(sid) = st.users[&id].stage.clone() else {
            return Err("busy");
        };
        let s = st.sessions.get_mut(&sid).ok_or("gone")?;
        let at = usize::from(s.users[1] == id);
        if s.votes[at].is_none() {
            s.votes[at] = Some(Vote { liked, again });
        }
        s.check.notify_one();
        st.bump(&id);
        Ok(())
    }

    /// The page came back from the room: look at that room now, not at the next tick.
    pub fn back(&self, token: &str) {
        let st = self.lock();
        if let Some(Stage::Room(sid)) = st
            .user_of(token)
            .and_then(|id| st.users.get(id))
            .map(|u| &u.stage)
            && let Some(s) = st.sessions.get(sid)
        {
            s.check.notify_one();
        }
    }

    /// Removes a person and everything kept about them. Refused in the middle of a session.
    pub fn forget(&self, token: &str) -> Result<(), &'static str> {
        let mut st = self.lock();
        let id = st.user_of(token).ok_or("nobody")?.to_string();
        if matches!(
            st.users[&id].stage,
            Stage::Build(_) | Stage::Room(_) | Stage::Vote(_)
        ) {
            return Err("busy");
        }
        st.users.remove(&id);
        st.tokens.remove(token);
        st.reservations.retain(|r| !r.users.contains(&id));
        for u in st.users.values_mut() {
            u.met.remove(&id);
        }
        drop(st);
        self.touch();
        Ok(())
    }

    /// Start of a held request. Marks the person as here.
    pub fn poll(&self, token: &str, seen: u64) -> Poll {
        let mut st = self.lock();
        let Some(id) = st.user_of(token).map(str::to_string) else {
            return Poll::Nobody;
        };
        let sponsors = self.sponsors();
        let u = st.users.get_mut(&id).expect("known user");
        u.seen_ms = now_ms();
        if u.ver != seen {
            return Poll::Now(st.view(&self.cfg, &sponsors, &id));
        }
        u.polls += 1;
        Poll::Wait(u.wake.subscribe())
    }

    /// End of a held request.
    pub fn poll_done(&self, token: &str) -> Value {
        let mut st = self.lock();
        let Some(id) = st.user_of(token).map(str::to_string) else {
            return json!({"screen": "hello", "v": 0});
        };
        let u = st.users.get_mut(&id).expect("known user");
        u.polls = u.polls.saturating_sub(1);
        u.seen_ms = now_ms();
        st.view(&self.cfg, &self.sponsors(), &id)
    }

    pub fn view_of(&self, token: &str) -> Option<Value> {
        let st = self.lock();
        let id = st.user_of(token)?;
        Some(st.view(&self.cfg, &self.sponsors(), id))
    }

    /// The return link: is this secret known?
    pub fn knows(&self, token: &str) -> bool {
        self.lock().user_of(token).is_some()
    }

    /// The calendar entry for a person's next session, if they have one.
    #[expect(
        clippy::many_single_char_names,
        reason = "Howard Hinnant's civil-date algorithm, kept with its usual names"
    )]
    pub fn calendar(&self, token: &str) -> Option<String> {
        let st = self.lock();
        let id = st.user_of(token)?;
        let r = st.reservation_of(id)?;
        let other = r
            .users
            .iter()
            .find(|u| u.as_str() != id)
            .and_then(|u| st.users.get(u))
            .map_or("someone", |u| u.name.as_str());
        let stamp = |ms: u64| {
            let (days, secs) = (ms / 86_400_000, (ms / 1000) % 86_400);
            // Civil date from days since 1970 (Howard Hinnant's algorithm).
            let z = days as i64 + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z.rem_euclid(146_097);
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let (d, m) = (
                doy - (153 * mp + 2) / 5 + 1,
                if mp < 10 { mp + 3 } else { mp - 9 },
            );
            let y = yoe + era * 400 + i64::from(m <= 2);
            format!(
                "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
                secs / 3600,
                (secs / 60) % 60,
                secs % 60
            )
        };
        let link = format!("{}/me/{}", self.cfg.public_url, st.users[id].token);
        Some(format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Duet//EN\r\nBEGIN:VEVENT\r\nUID:{}@duet\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Duet with {}\r\nDESCRIPTION:Open your link a few minutes before: {}\r\nURL:{}\r\nBEGIN:VALARM\r\nTRIGGER:-PT10M\r\nACTION:DISPLAY\r\nDESCRIPTION:Duet in 10 minutes\r\nEND:VALARM\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            r.id,
            stamp(now_ms()),
            stamp(r.at_ms),
            stamp(r.at_ms + r.ms),
            other.replace([',', ';', '\\', '\r', '\n'], " "),
            link,
            link
        ))
    }

    // ---- the loops -----------------------------------------------------------------------

    /// Runs for the life of the lobby: forms pairs, starts due sessions, saves.
    pub fn run(self: &Arc<Self>) {
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    () = app.round.notified() => {}
                    () = tokio::time::sleep(Duration::from_millis(250)) => {}
                }
                let opened = {
                    let mut st = app.lock();
                    let mut opened = st.round(&app.cfg);
                    opened.extend(st.due(&app.cfg));
                    opened
                };
                for sid in opened {
                    tokio::spawn(app.clone().session(sid));
                }
            }
        });
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(800)).await;
                if app.dirty.swap(false, Ordering::Relaxed) {
                    app.save();
                }
            }
        });
    }

    /// Writes what must outlive a restart: people, sessions set for later, and the record of
    /// finished sessions. Written beside the old file and swapped in, so a crash cannot leave half.
    pub fn save(&self) {
        let saved = {
            let st = self.lock();
            Saved {
                users: st
                    .users
                    .values()
                    .map(|u| SavedUser {
                        id: u.id.clone(),
                        token: u.token.clone(),
                        name: u.name.clone(),
                        lang: u.lang.clone(),
                        answers: u.answers.clone(),
                        left_out: u.left_out,
                        met: u.met.clone(),
                        liked: u.liked.clone(),
                        last_activity: u.last_activity.clone(),
                        created_ms: u.created_ms,
                    })
                    .collect(),
                reservations: st.reservations.clone(),
                history: st.history.iter().cloned().collect(),
                totals: st.totals.clone(),
            }
        };
        let dir = &self.cfg.data_dir;
        let write = || -> std::io::Result<()> {
            std::fs::create_dir_all(dir)?;
            let tmp = dir.join("state.json.tmp");
            std::fs::write(&tmp, serde_json::to_vec(&saved)?)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
            }
            std::fs::rename(tmp, dir.join("state.json"))
        };
        if let Err(e) = write() {
            eprintln!("duet: could not save state: {e}");
            self.touch();
        }
    }

    fn mark(&self, sid: &str, key: &'static str, done: bool) {
        let mut st = self.lock();
        let Some(s) = st.sessions.get_mut(sid) else {
            return;
        };
        let took = step(&mut s.marks, key, done);
        let users = s.users.clone();
        if let Some(ms) = took {
            st.stats.step_ms.entry(key).or_default().push(ms);
        }
        for u in &users {
            st.bump(u);
        }
    }

    /// The pair's plan: the rules always, the model's answer on top when it comes in time.
    async fn plan(&self, pair: &Pair<'_>) -> Plan {
        let Some(llm) = &self.llm else {
            self.lock().stats.rule_plans += 1;
            return planner::rules(pair);
        };
        let (system, user) = planner::question(pair);
        match llm
            .ask(
                &system,
                &user,
                1400,
                Duration::from_millis(self.cfg.plan_budget_ms),
            )
            .await
        {
            Ok(answer) => {
                let (plan, taken) = planner::merge(pair, &answer.json);
                let mut st = self.lock();
                st.stats.plan_ms.push(answer.ms);
                if taken > 0 {
                    st.stats.model_plans += 1;
                } else {
                    st.stats.rule_plans += 1;
                }
                plan
            }
            Err(e) => {
                eprintln!("duet: the model did not help with this pair: {e}");
                self.lock().stats.rule_plans += 1;
                planner::rules(pair)
            }
        }
    }

    /// One session from pairing to the last vote.
    async fn session(self: Arc<Self>, sid: String) {
        // What is needed from the two people, copied out so nothing is held while waiting.
        let Some((ids, names, langs, profiles, smiles, avoid, liked, long)) = ({
            let st = self.lock();
            st.sessions.get(&sid).and_then(|s| {
                let a = st.users.get(&s.users[0])?;
                let b = st.users.get(&s.users[1])?;
                // Activities this lobby does not offer are avoided like ones already done.
                let mut avoid: Vec<String> = planner::ACTIVITIES
                    .iter()
                    .filter(|k| !self.cfg.activities.iter().any(|a| a == *k))
                    .map(ToString::to_string)
                    .collect();
                // The one done last time is avoided too, unless it is the only one offered.
                let offered_left = planner::ACTIVITIES
                    .iter()
                    .filter(|k| !avoid.iter().any(|a| a == *k))
                    .count();
                if a.last_activity.is_some()
                    && a.last_activity == b.last_activity
                    && offered_left > 1
                {
                    avoid.extend(a.last_activity.clone());
                }
                Some((
                    s.users.clone(),
                    [a.name.clone(), b.name.clone()],
                    [a.lang.clone(), b.lang.clone()],
                    [a.profile.clone()?, b.profile.clone()?],
                    [
                        a.answers
                            .as_ref()
                            .map(|x| x.smile.clone())
                            .unwrap_or_default(),
                        b.answers
                            .as_ref()
                            .map(|x| x.smile.clone())
                            .unwrap_or_default(),
                    ],
                    avoid,
                    [a.liked.clone(), b.liked.clone()],
                    s.long,
                ))
            })
        }) else {
            return self.abandon(&sid, "failed");
        };
        let session_ms = if long {
            self.cfg.long_session_ms
        } else {
            self.cfg.session_ms
        };
        let ttl_ms = self.cfg.join_wait_ms + session_ms + 180_000;
        let room_id = format!("r_{}", token(8));
        let mut seed_bytes = [0u8; 8];
        let _ = getrandom::fill(&mut seed_bytes);
        let pair = Pair {
            people: [&profiles[0], &profiles[1]],
            smiles: [&smiles[0], &smiles[1]],
            avoid: &avoid,
            liked: [&liked[0], &liked[1]],
            seed: u64::from_le_bytes(seed_bytes),
        };

        // The plan and the room are made at the same time: the model's seconds hide inside the
        // sandbox's seconds.
        let on = |key: &'static str, done: bool| self.mark(&sid, key, done);
        let (plan, prepared) = if let Some((ready, marks)) = self.take_warm() {
            {
                let mut st = self.lock();
                st.stats.warm_taken += 1;
                if let Some(s) = st.sessions.get_mut(&sid) {
                    s.warm = true;
                    s.marks.extend(marks.into_iter().map(|(key, ms)| Mark {
                        key,
                        started: Instant::now(),
                        ms,
                    }));
                }
            }
            self.warm_wake.notify_one();
            (self.plan(&pair).await, Ok(ready))
        } else {
            tokio::join!(self.plan(&pair), self.host().prepare(&room_id, ttl_ms, &on))
        };
        {
            let mut st = self.lock();
            if let Some(s) = st.sessions.get_mut(&sid) {
                s.kind = Some(plan.activity.kind.clone());
                s.title = Some(plan.activity.title.clone());
                s.why = Some(plan.host.why.clone());
                s.by_model = plan.by_model;
            }
            for u in &ids {
                st.bump(u);
            }
        }
        let mut host = self.host();
        let mut ready = match prepared {
            Ok(ready) => ready,
            Err(e) => {
                // The sponsor's sandbox did not come up: a room inside the lobby, and say so.
                eprintln!(
                    "duet: {} room failed ({e}); using a local room",
                    host.kind()
                );
                self.lock().stats.fallbacks += 1;
                host = &self.local;
                match host.prepare(&room_id, ttl_ms, &on).await {
                    Ok(ready) => ready,
                    Err(_) => return self.abandon(&sid, "failed"),
                }
            }
        };
        let keys = [token(18), token(18)];
        let alternatives = planner::alternatives(&pair, &plan.activity.kind);
        let config = |ready: &Ready| RoomConfig {
            room_id: room_id.clone(),
            people: [0, 1].map(|i| Person {
                key: keys[i].clone(),
                name: names[i].clone(),
                color: COLORS[i].to_string(),
                lang: langs[i].clone(),
            }),
            activity: plan.activity.clone(),
            host: plan.host.clone(),
            session_ms,
            join_wait_ms: self.cfg.join_wait_ms,
            away_ms: 90_000.min(session_ms),
            nudge_after_ms: 75_000.min(session_ms / 3),
            return_url: format!("{}/#back", self.cfg.public_url),
            runtime: Runtime {
                kind: ready.kind.to_string(),
                id: ready.id.clone(),
            },
            long,
            alternatives: alternatives.clone(),
        };
        self.mark(&sid, "arm", false);
        let urls = match host.arm(&ready, config(&ready)).await {
            Ok(urls) => urls,
            Err(e) => {
                eprintln!(
                    "duet: {} room refused its plan ({e}); using a local room",
                    host.kind()
                );
                let _ = host.destroy(&ready).await;
                self.lock().stats.fallbacks += 1;
                host = &self.local;
                let Ok(fresh) = host.prepare(&room_id, ttl_ms, &on).await else {
                    return self.abandon(&sid, "failed");
                };
                ready = fresh;
                match host.arm(&ready, config(&ready)).await {
                    Ok(urls) => urls,
                    Err(_) => return self.abandon(&sid, "failed"),
                }
            }
        };
        self.mark(&sid, "arm", true);
        // A sandbox is reached through the lobby: Daytona's preview links show a warning page to
        // browsers, and that page drops the key after the `#`.
        let urls = match (&ready.control, ready.kind) {
            (Some(control), "daytona") => {
                self.proxied
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(room_id.clone(), control.trim_end_matches('/').to_string());
                keys.clone()
                    .map(|k| format!("{}/r/{room_id}/#k={k}&p=1", self.cfg.public_url))
            }
            _ => urls,
        };
        let check = {
            let mut st = self.lock();
            let Some(s) = st.sessions.get_mut(&sid) else {
                return;
            };
            s.runtime = Some((ready.kind.to_string(), ready.id.clone()));
            s.urls = Some(urls);
            s.live = true;
            s.ends_ms = now_ms() + session_ms;
            let took = s.born.elapsed().as_millis() as u64;
            s.ready_ms = Some(took);
            let check = s.check.clone();
            st.stats.ready_ms.push(took);
            for u in &ids {
                if let Some(user) = st.users.get_mut(u) {
                    user.stage = Stage::Room(sid.clone());
                    user.bump();
                }
            }
            check
        };

        // Watch the room: numbers only, unless someone reports.
        let hard_stop =
            Instant::now() + Duration::from_millis(self.cfg.join_wait_ms + session_ms + 20_000);
        let mut last = Value::Null;
        let mut exported = false;
        let mut misses = 0;
        loop {
            tokio::select! {
                () = check.notified() => {}
                () = tokio::time::sleep(Duration::from_millis(1_500.min(session_ms / 4).max(150))) => {}
            }
            if let Ok(status) = host.status(&ready).await {
                misses = 0;
                if status["reported"] == true
                    && !exported
                    && let Ok(Some(chat)) = host.export(&ready).await
                {
                    exported = self.keep_report(&sid, &ids, &chat);
                }
                let over = status["phase"] == "over";
                last = status;
                if over {
                    break;
                }
            } else {
                misses += 1;
                if misses >= 6 {
                    break; // the room is unreachable: treat it as ended
                }
            }
            if Instant::now() >= hard_stop {
                host.end(&ready).await;
                if let Ok(status) = host.status(&ready).await {
                    last = status;
                }
                break;
            }
        }

        // The bell. Destroy the room, then ask the two questions.
        self.proxied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&room_id);
        let receipt = host.destroy(&ready).await;
        let why = last["why"].as_str().unwrap_or("ended").to_string();
        let by = last["by"].as_u64().map(|b| b as u8);
        let joined = [last["joined"][0] == true, last["joined"][1] == true];
        let met = joined == [true, true];
        {
            let mut st = self.lock();
            st.stats.destroy_ms.push(receipt.ms);
            if let Some(s) = st.sessions.get_mut(&sid) {
                s.live = false;
                s.over = Some((why.clone(), by));
                s.receipt = Some(receipt.clone());
                s.vote_deadline_ms = now_ms() + self.cfg.vote_wait_ms;
            }
            for (i, u) in ids.iter().enumerate() {
                let Some(user) = st.users.get_mut(u) else {
                    continue;
                };
                if met {
                    user.stage = Stage::Vote(sid.clone());
                } else if joined[i] {
                    // They came and the other did not: straight back to the front of the queue.
                    user.stage = Stage::Wait;
                    user.queued_ms = now_ms();
                    user.seen_ms = now_ms(); // they were in the room, not gone
                    user.left_out += 1;
                    user.note = Some("noshow");
                } else {
                    user.stage = Stage::Home;
                    user.note = Some("missed");
                }
                user.bump();
            }
            st.bump_waiting();
        }
        self.round.notify_one();
        if met {
            let deadline = Instant::now() + Duration::from_millis(self.cfg.vote_wait_ms);
            loop {
                let both = self
                    .lock()
                    .sessions
                    .get(&sid)
                    .is_none_or(|s| s.votes.iter().all(Option::is_some));
                if both || Instant::now() >= deadline {
                    break;
                }
                tokio::select! {
                    () = check.notified() => {}
                    () = tokio::time::sleep(Duration::from_millis(400)) => {}
                }
            }
        }

        // Close the book on it.
        let mut st = self.lock();
        let Some(s) = st.sessions.get(&sid) else {
            return;
        };
        let votes = s.votes;
        let mutual = met && votes.iter().all(|v| v.is_some_and(|v| v.again));
        let kind = s.kind.clone().unwrap_or_default();
        let created = s.created_ms;
        let closed = Closed {
            id: sid.clone(),
            at_ms: now_ms(),
            users: ids.clone(),
            activity: kind.clone(),
            long,
            why,
            secs: last["elapsed_ms"].as_u64().unwrap_or(0) / 1000,
            mutual: met.then_some(mutual),
            liked: [votes[0].map(|v| v.liked), votes[1].map(|v| v.liked)],
            by_model: s.by_model,
            ready_ms: s.ready_ms,
            receipt: serde_json::to_value(&receipt).ok(),
            summary: last["summary"].clone(),
            counts: last["counts"].clone(),
            reported: exported,
        };
        st.totals.sessions += 1;
        if mutual {
            st.totals.mutual += 1;
        }
        for (i, u) in ids.iter().enumerate() {
            let other = ids[1 - i].clone();
            let Some(user) = st.users.get_mut(u) else {
                continue;
            };
            if met {
                let m = user.met.entry(other).or_default();
                m.times += 1;
                m.mutual = mutual;
                m.closed = !mutual || exported;
                user.last_activity = Some(kind.clone());
                if let Some(v) = votes[i] {
                    let delta = if v.liked { 1 } else { -1 };
                    match user.liked.iter_mut().find(|(k, _)| *k == kind) {
                        Some((_, n)) => *n = (*n + delta).clamp(-3, 3),
                        None => user.liked.push((kind.clone(), delta)),
                    }
                }
                if user.stage == Stage::Vote(sid.clone()) {
                    user.stage = Stage::Result(sid.clone());
                }
            }
            user.bump();
        }
        // Every mutual yes sets the next round, also after a second or third one.
        let mut note_job = None;
        if mutual {
            let at = (created + self.cfg.rematch_after_ms)
                .max(now_ms() + self.cfg.rematch_after_ms.min(60_000));
            st.reservations
                .retain(|r| !(r.users.contains(&ids[0]) && r.users.contains(&ids[1])));
            // The activity they finished, which is not the planned one if they switched.
            let played = last["summary"]["kind"]
                .as_str()
                .filter(|k| planner::ACTIVITIES.contains(k))
                .map_or_else(|| kind.clone(), ToString::to_string);
            let facts = notes::facts(
                &played,
                &last["summary"],
                last["elapsed_ms"].as_u64().unwrap_or(0) / 1000,
                self.cfg.long_session_ms.div_ceil(60_000),
            );
            let rid = format!("t_{}", token(6));
            st.reservations.push(Reservation {
                id: rid.clone(),
                users: ids.clone(),
                at_ms: at,
                ms: self.cfg.long_session_ms,
                activity: kind,
                note: Some(notes::rules(&facts)),
                note_by_model: false,
            });
            note_job = Some((rid, facts));
            for u in &ids {
                st.bump(u);
            }
        }
        if st.history.len() == 500 {
            st.history.pop_front();
        }
        st.history.push_back(closed);
        // The session itself stays in memory while someone may still be looking at its result.
        let old: Vec<String> = st
            .sessions
            .iter()
            .filter(|(_, s)| {
                !s.live
                    && s.receipt.is_some()
                    && now_ms().saturating_sub(s.created_ms) > 6 * 3600 * 1000
            })
            .map(|(k, _)| k.clone())
            .collect();
        for k in old {
            st.sessions.remove(&k);
        }
        drop(st);
        self.touch();
        if let Some((rid, facts)) = note_job {
            let app = self.clone();
            tokio::spawn(async move { app.write_note(&rid, &ids, &facts).await });
        }
    }

    /// The model's note for a pair who both said yes, in place of the rules' one when it holds up.
    /// The rules' note is already showing, so nobody waits for this.
    async fn write_note(&self, rid: &str, ids: &[String; 2], facts: &Value) {
        let Some(llm) = &self.llm else {
            self.lock().stats.rule_notes += 1;
            return;
        };
        let (system, user) = notes::question(facts);
        let budget = Duration::from_millis(self.cfg.plan_budget_ms);
        let note = match llm.ask_at(&system, &user, 400, budget, 0.4).await {
            Ok(answer) => {
                self.lock().stats.note_ms.push(answer.ms);
                notes::checked(facts, &answer.json)
            }
            Err(e) => {
                eprintln!("duet: the model did not write this pair's note: {e}");
                None
            }
        };
        let mut st = self.lock();
        let Some(note) = note else {
            st.stats.rule_notes += 1;
            return;
        };
        let Some(r) = st.reservations.iter_mut().find(|r| r.id == rid) else {
            return;
        };
        r.note = Some(note);
        r.note_by_model = true;
        st.stats.model_notes += 1;
        for u in ids {
            st.bump(u);
        }
        drop(st);
        self.touch();
    }

    /// A session that could not be built: both go back to the queue.
    fn abandon(&self, sid: &str, note: &'static str) {
        let mut st = self.lock();
        let Some(s) = st.sessions.remove(sid) else {
            return;
        };
        for u in &s.users {
            if let Some(user) = st.users.get_mut(u) {
                user.stage = if user.profile.is_some() {
                    Stage::Wait
                } else {
                    Stage::Ask
                };
                user.queued_ms = now_ms();
                user.note = Some(note);
                user.bump();
            }
        }
    }

    /// Stores a reported session's chat. This is the only place words from a room are kept.
    fn keep_report(&self, sid: &str, ids: &[String; 2], chat: &Value) -> bool {
        let dir = self.cfg.data_dir.join("reports");
        let body = json!({"session": sid, "at_ms": now_ms(), "users": ids, "room": chat});
        let written = std::fs::create_dir_all(&dir)
            .and_then(|()| std::fs::write(dir.join(format!("{sid}.json")), body.to_string()));
        match written {
            Ok(()) => {
                let mut st = self.lock();
                st.totals.reports += 1;
                // Reported pairs are closed to each other from this moment.
                for (i, u) in ids.iter().enumerate() {
                    if let Some(user) = st.users.get_mut(u) {
                        user.met.entry(ids[1 - i].clone()).or_default().closed = true;
                    }
                }
                drop(st);
                self.touch();
                true
            }
            Err(e) => {
                eprintln!("duet: could not store a report: {e}");
                false
            }
        }
    }

    // ---- rooms built ahead ------------------------------------------------------------------

    fn warm_target(&self, st: &State) -> usize {
        st.warm_target.unwrap_or(self.cfg.warm_rooms).min(MAX_WARM)
    }

    /// Starts keeping rooms ready ahead, when rooms run in Daytona. Call it after leftovers from
    /// an earlier run were swept, or the sweep would remove the new ones too.
    pub fn keep_warm(self: &Arc<Self>) {
        if self.daytona().is_none() {
            return;
        }
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                app.tend_warm();
                tokio::select! {
                    () = app.warm_wake.notified() => {}
                    () = tokio::time::sleep(Duration::from_secs(5)) => {}
                }
            }
        });
    }

    /// Retires rooms that waited too long, and starts building what is missing, all at once.
    fn tend_warm(self: &Arc<Self>) {
        let (old, build) = {
            let mut st = self.lock();
            let max = Duration::from_millis(self.cfg.warm_max_age_ms);
            let mut old = Vec::new();
            st.warm.retain(|w| match (&w.ready, w.ready_at) {
                (Some(r), Some(at)) if at.elapsed() > max => {
                    old.push(r.clone());
                    false
                }
                _ => true,
            });
            // A failure is shown for half a minute, and nothing new is built meanwhile, so a
            // broken key does not turn into a stream of failed sandboxes.
            st.warm.retain(|w| {
                w.failed
                    .as_ref()
                    .is_none_or(|(_, at)| at.elapsed() < Duration::from_secs(30))
            });
            let failing = st.warm.iter().any(|w| w.failed.is_some());
            let want = self.warm_target(&st);
            let n = if failing {
                0
            } else {
                want.saturating_sub(st.warm.len())
            };
            let mut build = Vec::new();
            if n > 0 {
                st.batches += 1;
                let batch = st.batches;
                st.last_batch = Some(Batch {
                    id: batch,
                    n,
                    started: Instant::now(),
                    done_ms: None,
                    failed: 0,
                });
                for _ in 0..n {
                    let slot = format!("w_{}", token(6));
                    st.warm.push(Warm {
                        slot: slot.clone(),
                        batch,
                        ready: None,
                        failed: None,
                        born: Instant::now(),
                        ready_at: None,
                        marks: Vec::new(),
                    });
                    build.push(slot);
                }
            }
            (old, build)
        };
        for ready in old {
            let app = self.clone();
            tokio::spawn(async move {
                let receipt = app.host().destroy(&ready).await;
                app.lock().stats.destroy_ms.push(receipt.ms);
            });
        }
        for slot in build {
            tokio::spawn(self.clone().build_warm(slot));
        }
    }

    async fn build_warm(self: Arc<Self>, slot: String) {
        let ttl_ms =
            self.cfg.warm_max_age_ms + self.cfg.join_wait_ms + self.cfg.long_session_ms + 180_000;
        let on = |key: &'static str, done: bool| {
            let mut st = self.lock();
            let took = st
                .warm
                .iter_mut()
                .find(|w| w.slot == slot)
                .and_then(|w| step(&mut w.marks, key, done));
            if let Some(ms) = took {
                st.stats.step_ms.entry(key).or_default().push(ms);
            }
        };
        let result = self.host().prepare(&slot, ttl_ms, &on).await;
        let orphan = {
            let mut st = self.lock();
            match st.warm.iter_mut().find(|w| w.slot == slot) {
                // Replaced while it was being built: it is not wanted any more.
                None => result.ok(),
                Some(w) => {
                    match result {
                        Ok(ready) => {
                            w.ready = Some(ready);
                            w.ready_at = Some(Instant::now());
                        }
                        Err(e) => {
                            eprintln!("duet: a room built ahead failed: {e}");
                            w.failed = Some((e.chars().take(200).collect(), Instant::now()));
                        }
                    }
                    let (batch, failed) = (w.batch, w.failed.is_some());
                    let all_done = st
                        .warm
                        .iter()
                        .filter(|w| w.batch == batch)
                        .all(|w| w.ready.is_some() || w.failed.is_some());
                    if let Some(b) = st.last_batch.as_mut().filter(|b| b.id == batch) {
                        b.failed += usize::from(failed);
                        if all_done && b.done_ms.is_none() {
                            b.done_ms = Some(b.started.elapsed().as_millis() as u64);
                        }
                    }
                    None
                }
            }
        };
        if let Some(ready) = orphan {
            let _ = self.host().destroy(&ready).await;
        }
    }

    /// The oldest room that is ready and still young enough, with how long each step took.
    fn take_warm(&self) -> Option<(Ready, StepTimes)> {
        self.daytona()?;
        let mut st = self.lock();
        let max = Duration::from_millis(self.cfg.warm_max_age_ms);
        let at = st
            .warm
            .iter()
            .position(|w| w.ready.is_some() && w.ready_at.is_some_and(|t| t.elapsed() < max))?;
        let w = st.warm.remove(at);
        let marks = w.marks.iter().map(|m| (m.key, m.ms)).collect();
        Some((w.ready?, marks))
    }

    /// The admin's "build them now": the rooms waiting ahead are replaced by `n` new ones, all
    /// built at the same time.
    pub fn rebuild_warm(self: &Arc<Self>, n: usize) -> Result<(), &'static str> {
        if self.daytona().is_none() {
            return Err("no_daytona");
        }
        let old: Vec<Ready> = {
            let mut st = self.lock();
            if st
                .warm
                .iter()
                .any(|w| w.ready.is_none() && w.failed.is_none())
            {
                return Err("building");
            }
            st.warm_target = Some(0); // nothing new until the old ones are gone
            st.warm.drain(..).filter_map(|w| w.ready).collect()
        };
        let app = self.clone();
        tokio::spawn(async move {
            let host = app.host();
            let receipts =
                futures_util::future::join_all(old.iter().map(|r| host.destroy(r))).await;
            {
                let mut st = app.lock();
                for receipt in receipts {
                    st.stats.destroy_ms.push(receipt.ms);
                }
                st.warm_target = Some(n.min(MAX_WARM));
            }
            app.warm_wake.notify_one();
        });
        Ok(())
    }

    /// Removes every room waiting ahead, for when the lobby stops. Their own lifetime would remove
    /// them later anyway.
    pub async fn close_warm(&self) {
        let ready: Vec<Ready> = {
            let mut st = self.lock();
            st.warm_target = Some(0);
            st.warm.drain(..).filter_map(|w| w.ready).collect()
        };
        let host = self.host();
        let all = futures_util::future::join_all(ready.iter().map(|r| host.destroy(r)));
        let _ = tokio::time::timeout(Duration::from_secs(10), all).await;
    }

    /// The numbers for the projector. Initials only, and nothing anyone typed.
    pub fn hood(&self) -> Value {
        let st = self.lock();
        let now = now_ms();
        let initials = |ids: &[String; 2]| {
            ids.iter()
                .map(|u| {
                    st.users
                        .get(u)
                        .map_or("?".to_string(), |u| initial(&u.name))
                })
                .collect::<Vec<_>>()
        };
        let mut rooms: Vec<Value> = st
            .sessions
            .values()
            .filter(|s| s.receipt.is_none())
            .map(|s| {
                json!({
                    "id": s.id, "phase": if s.live { "live" } else { "building" }, "pair": initials(&s.users), "activity": s.kind, "long": s.long,
                    "runtime": s.runtime.as_ref().map(|(k, id)| json!({"kind": k, "id": id})),
                    "marks": marks_view(&s.marks),
                    "left_ms": if s.live { s.ends_ms.saturating_sub(now) } else { 0 }, "ready_ms": s.ready_ms, "by_model": s.by_model, "created_ms": s.created_ms, "warm": s.warm,
                })
            })
            .collect();
        rooms.sort_by_key(|r| std::cmp::Reverse(r["created_ms"].as_u64()));
        let recent: Vec<Value> = st
            .history
            .iter()
            .rev()
            .take(12)
            .map(|c| json!({"id": c.id, "at_ms": c.at_ms, "activity": c.activity, "secs": c.secs, "why": c.why, "mutual": c.mutual, "receipt": c.receipt, "ready_ms": c.ready_ms, "by_model": c.by_model, "long": c.long, "summary": c.summary}))
            .collect();
        json!({
            "now": now,
            "sponsors": self.sponsors(),
            "model": self.llm.as_ref().map(|l| l.url().to_string()),
            "daytona_api": self.daytona().map(|d| d.api.api_name()),
            "people": {"here": st.users.values().filter(|u| u.here(now)).count(), "waiting": st.waiting(), "known": st.users.len()},
            "rooms": rooms,
            "recent": recent,
            "set_for_later": st.reservations.len(),
            "totals": st.totals,
            "timings": {
                "room_ready_ms": st.stats.ready_ms.view(), "model_ms": st.stats.plan_ms.view(),
                "pairing_us": st.stats.solve_us.view(), "destroy_ms": st.stats.destroy_ms.view(),
                "read_ms": st.stats.read_ms.view(), "note_ms": st.stats.note_ms.view(),
                "steps": st.stats.step_ms.iter().map(|(k, v)| (k.to_string(), v.view())).collect::<serde_json::Map<_, _>>(),
            },
            "plans": {"model": st.stats.model_plans, "rules": st.stats.rule_plans, "local_fallbacks": st.stats.fallbacks},
            "ai": {
                "reads": {"model": st.stats.model_reads, "rules": st.stats.rule_reads},
                "plans": {"model": st.stats.model_plans, "rules": st.stats.rule_plans},
                "notes": {"model": st.stats.model_notes, "rules": st.stats.rule_notes},
            },
            "warm": {
                "on": self.daytona().is_some(),
                "target": self.warm_target(&st),
                "max_secs": self.cfg.warm_max_age_ms / 1000,
                "taken": st.stats.warm_taken,
                "rooms": st.warm.iter().map(|w| json!({
                    "slot": w.slot, "batch": w.batch,
                    "state": if w.failed.is_some() { "failed" } else if w.ready.is_some() { "ready" } else { "building" },
                    "id": w.ready.as_ref().map(|r| r.id.clone()),
                    "error": w.failed.as_ref().map(|(e, _)| e.clone()),
                    "marks": marks_view(&w.marks),
                    "age_ms": w.born.elapsed().as_millis() as u64,
                    "built_ms": w.ready_at.map(|t| t.duration_since(w.born).as_millis() as u64),
                    "waiting_ms": w.ready_at.map(|t| t.elapsed().as_millis() as u64),
                })).collect::<Vec<_>>(),
                "batch": st.last_batch.as_ref().map(|b| json!({
                    "id": b.id, "n": b.n, "done_ms": b.done_ms, "failed": b.failed,
                    "running_ms": b.started.elapsed().as_millis() as u64,
                })),
            },
            "session_secs": self.cfg.session_ms / 1000,
        })
    }
}
