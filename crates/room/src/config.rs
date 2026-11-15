//! What the lobby hands a room when it arms it. The room knows nothing else about the world.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::text::clean_name;

/// A line in both languages. Each screen shows the one its person chose.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Bi {
    #[serde(default)]
    pub en: String,
    #[serde(default)]
    pub ja: String,
}

impl Bi {
    pub fn new(en: impl Into<String>, ja: impl Into<String>) -> Self {
        Self {
            en: en.into(),
            ja: ja.into(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.en.is_empty() && self.ja.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Person {
    /// The secret half of this person's link. Never sent to the other person.
    pub key: String,
    pub name: String,
    pub color: String,
    /// `en` or `ja`.
    pub lang: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Activity {
    /// Which page module runs: `wavelength`, `sketch`, `signal`, `thisorthat` or `story`.
    pub kind: String,
    pub title: Bi,
    /// The module's own content, such as prompts.
    #[serde(default)]
    pub data: Value,
    /// Both screens build the same levels from this.
    #[serde(default)]
    pub seed: u32,
}

/// What the host says, and when. Written by the lobby before the room opens, so the room needs
/// no key and no network.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HostScript {
    /// Why these two were paired.
    #[serde(default)]
    pub why: Bi,
    #[serde(default)]
    pub opener: Bi,
    /// Used one at a time when the chat goes quiet.
    #[serde(default)]
    pub nudges: Vec<Bi>,
    #[serde(default)]
    pub last_minute: Bi,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Runtime {
    /// `daytona` or `local`. Shown on the page, so a local room is never taken for a sponsor run.
    pub kind: String,
    /// The sandbox id, or the local room id.
    pub id: String,
}

fn default_join_wait() -> u64 {
    90_000
}
fn default_away() -> u64 {
    90_000
}
fn default_nudge() -> u64 {
    75_000
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomConfig {
    pub room_id: String,
    pub people: [Person; 2],
    pub activity: Activity,
    #[serde(default)]
    pub host: HostScript,
    /// Length of the session once both are in.
    pub session_ms: u64,
    /// How long the first person waits for the second.
    #[serde(default = "default_join_wait")]
    pub join_wait_ms: u64,
    /// How long a dropped connection may stay away before the session ends.
    #[serde(default = "default_away")]
    pub away_ms: u64,
    /// Silence in chat before the host speaks.
    #[serde(default = "default_nudge")]
    pub nudge_after_ms: u64,
    /// Where the page goes when the session is over.
    pub return_url: String,
    pub runtime: Runtime,
    /// A second, longer session between two people who both said yes.
    #[serde(default)]
    pub long: bool,
    /// Other activities, ready to play, that the two may agree to switch to.
    #[serde(default)]
    pub alternatives: Vec<Activity>,
}

/// The activities a room can run, one page module each.
pub const KINDS: [&str; 5] = ["wavelength", "sketch", "signal", "thisorthat", "story"];

impl RoomConfig {
    /// Refuses a plan that could not run, and tidies what it accepts.
    pub fn checked(mut self) -> Result<Self, String> {
        if !(3_000..=3 * 3600 * 1000).contains(&self.session_ms) {
            return Err("session_ms must be between 3 seconds and 3 hours".into());
        }
        if self.people[0].key.len() < 16 || self.people[1].key.len() < 16 {
            return Err("each person needs a key of at least 16 characters".into());
        }
        if self.people[0].key == self.people[1].key {
            return Err("the two keys must differ".into());
        }
        if !KINDS.contains(&self.activity.kind.as_str()) {
            return Err(format!("unknown activity {:?}", self.activity.kind));
        }
        // Alternatives: known kinds only, none the same as the activity or each other, four at most.
        let mut seen = vec![self.activity.kind.clone()];
        self.alternatives.retain(|a| {
            let fresh = KINDS.contains(&a.kind.as_str()) && !seen.contains(&a.kind);
            if fresh {
                seen.push(a.kind.clone());
            }
            fresh
        });
        self.alternatives.truncate(4);
        for (i, p) in self.people.iter_mut().enumerate() {
            p.name = clean_name(&p.name).unwrap_or_else(|| {
                if i == 0 {
                    "Guest 1".into()
                } else {
                    "Guest 2".into()
                }
            });
            if p.lang != "ja" {
                p.lang = "en".into();
            }
            let hex = p.color.strip_prefix('#').unwrap_or("");
            if !(hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit())) {
                p.color = if i == 0 {
                    "#ffb84d".into()
                } else {
                    "#ff4f8b".into()
                };
            }
        }
        self.host.nudges.truncate(6);
        Ok(self)
    }
}

#[cfg(test)]
pub fn sample(session_ms: u64) -> RoomConfig {
    RoomConfig {
        room_id: "r_test".into(),
        people: [
            Person {
                key: "key-of-person-zero-000".into(),
                name: "Ana".into(),
                color: "#ffb84d".into(),
                lang: "en".into(),
            },
            Person {
                key: "key-of-person-one-1111".into(),
                name: "Aoi".into(),
                color: "#ff4f8b".into(),
                lang: "ja".into(),
            },
        ],
        activity: Activity {
            kind: "wavelength".into(),
            title: Bi::new("Same Wavelength", "以心伝心"),
            data: Value::Null,
            seed: 7,
        },
        host: HostScript {
            why: Bi::new("You both like music.", "二人とも音楽が好き。"),
            opener: Bi::new("Say hi.", "あいさつしよう。"),
            nudges: vec![Bi::new("Quiet?", "静か？")],
            last_minute: Bi::new("One minute left.", "あと1分。"),
        },
        session_ms,
        join_wait_ms: 2_000,
        away_ms: 2_000,
        nudge_after_ms: 600,
        return_url: "/".into(),
        runtime: Runtime {
            kind: "local".into(),
            id: "r_test".into(),
        },
        long: false,
        alternatives: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternatives_are_known_and_never_repeat_the_activity() {
        let mut cfg = sample(60_000);
        let alt = |kind: &str| Activity {
            kind: kind.into(),
            title: Bi::new("x", "x"),
            data: Value::Null,
            seed: 1,
        };
        cfg.alternatives = vec![
            alt("story"),
            alt(&cfg.activity.kind.clone()),
            alt("chess"),
            alt("story"),
            alt("signal"),
        ];
        let cfg = cfg.checked().unwrap();
        let kinds: Vec<&str> = cfg.alternatives.iter().map(|a| a.kind.as_str()).collect();
        assert_eq!(kinds, ["story", "signal"]);
    }

    #[test]
    fn a_good_plan_passes_and_is_tidied() {
        let mut cfg = sample(60_000);
        cfg.people[0].name = "  \n ".into();
        cfg.people[1].color = "red".into();
        cfg.people[1].lang = "fr".into();
        let cfg = cfg.checked().unwrap();
        assert_eq!(cfg.people[0].name, "Guest 1");
        assert_eq!(cfg.people[1].color, "#ff4f8b");
        assert_eq!(cfg.people[1].lang, "en");
    }

    #[test]
    fn bad_plans_are_refused() {
        let mut same = sample(60_000);
        same.people[1].key = same.people[0].key.clone();
        assert!(same.checked().is_err());
        assert!(sample(10).checked().is_err());
        let mut odd = sample(60_000);
        odd.activity.kind = "roulette".into();
        assert!(odd.checked().is_err());
    }
}
