//! From a few taps to a profile, and from two profiles to a score.

use serde::{Deserialize, Serialize};

use duet_room::text::clean_text;

pub const INTERESTS: [&str; 12] = [
    "music", "films", "games", "food", "travel", "books", "anime", "sports", "art", "tech",
    "nature", "pets",
];

/// What the page sends after the questions. Every field is one of a fixed set of words, except
/// `smile`, which is free text and optional.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Answers {
    /// `en`, `ja`, or both.
    #[serde(default)]
    pub langs: Vec<String>,
    /// `calm`, `playful`, `talkative` or `tired`.
    #[serde(default)]
    pub mood: String,
    /// `make`, `play` or `talk`.
    #[serde(default)]
    pub mode: String,
    /// `team`, `rival` or `either`.
    #[serde(default)]
    pub style: String,
    /// `listener`, `talker` or `both`.
    #[serde(default)]
    pub talk: String,
    #[serde(default)]
    pub interests: Vec<String>,
    #[serde(default)]
    pub smile: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Make,
    Play,
    Talk,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Profile {
    pub en: bool,
    pub ja: bool,
    /// 0 calm, 1 lively.
    pub energy: f32,
    /// 0 plays as a team, 1 likes a contest.
    pub rivalry: f32,
    /// 0 listens, 1 talks.
    pub talk: f32,
    pub mode: Mode,
    /// One bit per entry of [`INTERESTS`].
    pub interests: u16,
    /// Extra words read from the free text by the model, when one is connected.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl Answers {
    /// Keeps only known words and cleans the free text. Returns `None` if nothing usable is left.
    pub fn checked(mut self) -> Option<Self> {
        self.langs.retain(|l| l == "en" || l == "ja");
        self.langs.sort();
        self.langs.dedup();
        if self.langs.is_empty() {
            return None;
        }
        let one_of = |v: &str, allowed: &[&str]| allowed.contains(&v);
        if !one_of(&self.mood, &["calm", "playful", "talkative", "tired"])
            || !one_of(&self.mode, &["make", "play", "talk"])
            || !one_of(&self.style, &["team", "rival", "either"])
            || !one_of(&self.talk, &["listener", "talker", "both"])
        {
            return None;
        }
        self.interests.retain(|i| INTERESTS.contains(&i.as_str()));
        self.interests.sort();
        self.interests.dedup();
        self.interests.truncate(3);
        self.smile = clean_text(&self.smile.replace('\n', " "), 140).unwrap_or_default();
        Some(self)
    }

    pub fn profile(&self) -> Profile {
        let energy = match self.mood.as_str() {
            "tired" => 0.1,
            "calm" => 0.25,
            "talkative" => 0.65,
            _ => 0.85,
        };
        let mut talk: f32 = match self.talk.as_str() {
            "listener" => 0.15,
            "talker" => 0.85,
            _ => 0.5,
        };
        if self.mood == "talkative" {
            talk = (talk + 0.15).min(1.0);
        }
        Profile {
            en: self.langs.iter().any(|l| l == "en"),
            ja: self.langs.iter().any(|l| l == "ja"),
            energy,
            rivalry: match self.style.as_str() {
                "team" => 0.0,
                "rival" => 1.0,
                _ => 0.5,
            },
            talk,
            mode: match self.mode.as_str() {
                "make" => Mode::Make,
                "play" => Mode::Play,
                _ => Mode::Talk,
            },
            interests: self
                .interests
                .iter()
                .filter_map(|i| INTERESTS.iter().position(|k| k == i))
                .fold(0, |bits, at| bits | (1 << at)),
            tags: Vec::new(),
        }
    }
}

impl Profile {
    pub fn interest_names(&self) -> Vec<&'static str> {
        INTERESTS
            .iter()
            .enumerate()
            .filter(|(i, _)| self.interests & (1 << i) != 0)
            .map(|(_, n)| *n)
            .collect()
    }
}

/// The language two people will be greeted in when they share more than one: English first.
pub fn shared_language(a: &Profile, b: &Profile) -> Option<&'static str> {
    if a.en && b.en {
        Some("en")
    } else if a.ja && b.ja {
        Some("ja")
    } else {
        None
    }
}

/// How well two people fit, from 0 to 1. `None` when they share no language.
///
/// Likeness counts for interests, energy and play style. For talking it is the other way round:
/// a talker fits a listener, and two people in the middle fit each other.
pub fn fit(a: &Profile, b: &Profile) -> Option<f32> {
    shared_language(a, b)?;
    let both = (a.interests & b.interests).count_ones() as f32;
    let either = (a.interests | b.interests).count_ones() as f32;
    let shared_tags = a.tags.iter().filter(|t| b.tags.contains(t)).count() as f32;
    let interests = if either == 0.0 { 0.3 } else { both / either };
    let interests = (interests + 0.12 * shared_tags).min(1.0);
    let energy = 1.0 - (a.energy - b.energy).abs();
    let rivalry = 1.0 - (a.rivalry - b.rivalry).abs();
    let talk = 1.0 - (a.talk + b.talk - 1.0).abs();
    let mode = if a.mode == b.mode {
        1.0
    } else if a.mode != Mode::Talk && b.mode != Mode::Talk {
        0.6
    } else {
        0.45
    };
    Some(0.32 * interests + 0.20 * energy + 0.13 * rivalry + 0.20 * talk + 0.15 * mode)
}

/// The lowest fit two people may be paired at. It falls the longer the more patient of the two
/// has waited: a poor match now beats a good one that may never come.
pub fn bar(longest_wait_ms: u64) -> f32 {
    0.30 * (-(longest_wait_ms as f32) / 30_000.0).exp()
}

/// Extra weight for having waited, so the same person is not left out twice.
pub fn patience(wait_ms: u64, times_left_out: u32) -> f32 {
    0.25 * (1.0 - (-(wait_ms as f32) / 60_000.0).exp()) + 0.2 * times_left_out.min(3) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answers(
        langs: &[&str],
        mood: &str,
        mode: &str,
        style: &str,
        talk: &str,
        interests: &[&str],
    ) -> Answers {
        Answers {
            langs: langs.iter().map(ToString::to_string).collect(),
            mood: mood.into(),
            mode: mode.into(),
            style: style.into(),
            talk: talk.into(),
            interests: interests.iter().map(ToString::to_string).collect(),
            smile: String::new(),
        }
    }

    #[test]
    fn answers_are_checked() {
        let good = answers(
            &["en", "xx", "en"],
            "calm",
            "make",
            "team",
            "listener",
            &["music", "hacking", "food", "art", "pets"],
        )
        .checked()
        .unwrap();
        assert_eq!(good.langs, ["en"]);
        assert_eq!(
            good.interests,
            ["art", "food", "music"],
            "unknown ones dropped, at most three kept"
        );
        assert!(
            answers(&[], "calm", "make", "team", "listener", &[])
                .checked()
                .is_none()
        );
        assert!(
            answers(&["en"], "furious", "make", "team", "listener", &[])
                .checked()
                .is_none()
        );
        let mut long = answers(&["ja"], "calm", "talk", "either", "both", &[]);
        long.smile = "a\nb ".repeat(200);
        let long = long.checked().unwrap();
        assert!(long.smile.chars().count() <= 140 && !long.smile.contains('\n'));
    }

    #[test]
    fn no_shared_language_means_no_match() {
        let a = answers(&["en"], "calm", "make", "team", "listener", &["music"]).profile();
        let b = answers(&["ja"], "calm", "make", "team", "talker", &["music"]).profile();
        let c = answers(&["en", "ja"], "calm", "make", "team", "talker", &["music"]).profile();
        assert_eq!(fit(&a, &b), None);
        assert!(fit(&a, &c).is_some());
        assert_eq!(shared_language(&b, &c), Some("ja"));
        assert_eq!(shared_language(&a, &c), Some("en"));
    }

    #[test]
    fn a_talker_fits_a_listener_better_than_another_talker() {
        let talker = answers(&["en"], "playful", "play", "team", "talker", &["games"]).profile();
        let listener =
            answers(&["en"], "playful", "play", "team", "listener", &["games"]).profile();
        assert!(fit(&talker, &listener) > fit(&talker, &talker));
        assert!(fit(&talker, &listener) > fit(&listener, &listener));
    }

    #[test]
    fn shared_interests_and_energy_raise_the_fit() {
        let a = answers(
            &["en"],
            "calm",
            "make",
            "team",
            "both",
            &["art", "music", "nature"],
        )
        .profile();
        let alike = answers(
            &["en"],
            "calm",
            "make",
            "team",
            "both",
            &["art", "music", "books"],
        )
        .profile();
        let unlike = answers(
            &["en"],
            "playful",
            "play",
            "rival",
            "both",
            &["sports", "tech"],
        )
        .profile();
        let (near, far) = (fit(&a, &alike).unwrap(), fit(&a, &unlike).unwrap());
        assert!(near > far + 0.3, "{near} vs {far}");
        assert!((0.0..=1.0).contains(&near) && (0.0..=1.0).contains(&far));
        assert_eq!(
            fit(&a, &alike),
            fit(&alike, &a),
            "the score does not depend on who asks"
        );
    }

    #[test]
    fn waiting_lowers_the_bar_and_raises_priority() {
        assert!(bar(0) > bar(30_000));
        assert!(bar(120_000) < 0.01);
        assert!(patience(60_000, 0) > patience(1_000, 0));
        assert!(
            patience(0, 1) > patience(60_000, 0),
            "being left out once counts for more than a minute of waiting"
        );
    }
}
