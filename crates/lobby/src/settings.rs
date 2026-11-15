//! Everything that can be set from outside, read once at start from the environment.

use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomsMode {
    /// Daytona when a key is present, otherwise local.
    Auto,
    Local,
    Daytona,
}

/// Which Daytona API to speak. Daytona released a new one (SDK 2.0.0) on 2026-11-13; the old
/// one is still served. `Auto` asks the new one whether it knows the key, and uses the old one
/// if it does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DaytonaApi {
    #[default]
    Auto,
    V0,
    V2,
}

#[derive(Clone, Debug, Default)]
pub struct DaytonaSettings {
    /// The old API (SDK 0.231): `https://app.daytona.io/api`.
    pub api_url: String,
    /// The new API (SDK 2.0): `https://api.daytona.com`.
    pub api2_url: String,
    pub api: DaytonaApi,
    pub key: String,
    /// A region (old API: target; new API: `region_id`).
    pub target: Option<String>,
    /// A snapshot (new API: an image) that already holds the room binary at
    /// `/usr/local/bin/duet-room`.
    pub snapshot: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LlmSettings {
    pub url: String,
    pub model: Option<String>,
    pub key: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub bind: SocketAddr,
    /// The address people's browsers use, without a trailing slash.
    pub public_url: String,
    pub data_dir: PathBuf,
    pub session_ms: u64,
    pub long_session_ms: u64,
    /// How long after the first session the second one is set.
    pub rematch_after_ms: u64,
    /// How long after that time the two may still start.
    pub rematch_window_ms: u64,
    /// How long before that time they may already start, if both are here.
    pub rematch_early_ms: u64,
    pub vote_wait_ms: u64,
    pub join_wait_ms: u64,
    /// A round fires once arrivals have paused for this long...
    pub settle_ms: u64,
    /// ...or once someone has waited this long, whichever comes first.
    pub max_gather_ms: u64,
    /// How long the model may take over a pair's plan before the rules are used alone.
    pub plan_budget_ms: u64,
    pub rooms: RoomsMode,
    /// How many rooms to keep built ahead when rooms run in Daytona.
    pub warm_rooms: usize,
    /// The activities a pair may be given (`DUET_ACTIVITIES`, comma-separated). All by default.
    pub activities: Vec<String>,
    /// A room built ahead that waited this long is replaced, so it always has its full lifetime.
    pub warm_max_age_ms: u64,
    /// Unlocks the admin page's buttons: `/hood#key=…`. Made up at start when not set.
    pub admin_key: String,
    pub daytona: Option<DaytonaSettings>,
    /// Where the Linux room binaries are (`duet-room-x86_64`, `duet-room-aarch64`).
    pub room_bin_dir: PathBuf,
    pub llm: Option<LlmSettings>,
}

fn var(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn secs(name: &str, default_ms: u64) -> u64 {
    var(name)
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|s| *s > 0.0)
        .map_or(default_ms, |s| (s * 1000.0) as u64)
}

/// A secret from the environment, or from a file named there, or from the usual place under
/// `~/Secrets`. It is never printed.
fn secret(name: &str, usual: &str) -> Option<String> {
    if let Some(v) = var(name) {
        return Some(v);
    }
    let file = var(&format!("{name}_FILE"))
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| PathBuf::from(h).join("Secrets").join(usual)))?;
    std::fs::read_to_string(file)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl Settings {
    pub fn from_env() -> Result<Self, String> {
        let bind: SocketAddr = var("DUET_BIND")
            .unwrap_or_else(|| "127.0.0.1:8787".into())
            .parse()
            .map_err(|e| format!("DUET_BIND: {e}"))?;
        let public_url = var("DUET_PUBLIC_URL")
            .unwrap_or_else(|| format!("http://{bind}"))
            .trim_end_matches('/')
            .to_string();
        let rooms = match var("DUET_ROOMS").as_deref() {
            None | Some("auto") => RoomsMode::Auto,
            Some("local") => RoomsMode::Local,
            Some("daytona") => RoomsMode::Daytona,
            Some(other) => {
                return Err(format!(
                    "DUET_ROOMS must be auto, local or daytona, not {other:?}"
                ));
            }
        };
        let api = match var("DUET_DAYTONA_API").as_deref() {
            None | Some("auto") => DaytonaApi::Auto,
            Some("v0") => DaytonaApi::V0,
            Some("v2") => DaytonaApi::V2,
            Some(other) => {
                return Err(format!(
                    "DUET_DAYTONA_API must be auto, v0 or v2, not {other:?}"
                ));
            }
        };
        let daytona = secret("DAYTONA_API_KEY", "Daytona/key").map(|key| DaytonaSettings {
            api_url: var("DAYTONA_API_URL")
                .unwrap_or_else(|| "https://app.daytona.io/api".into())
                .trim_end_matches('/')
                .to_string(),
            api2_url: var("DAYTONA_V2_API_URL")
                .unwrap_or_else(|| "https://api.daytona.com".into())
                .trim_end_matches('/')
                .to_string(),
            api,
            key,
            target: var("DAYTONA_TARGET"),
            snapshot: var("DUET_DAYTONA_SNAPSHOT"),
        });
        if rooms == RoomsMode::Daytona && daytona.is_none() {
            return Err("DUET_ROOMS=daytona needs DAYTONA_API_KEY".into());
        }
        let llm_key = secret("NOSANA_API_KEY", "Nosana/key");
        let llm = (var("NOSANA_LLM_URL").is_some() || llm_key.is_some()).then(|| LlmSettings {
            url: var("NOSANA_LLM_URL").unwrap_or_else(|| crate::llm::DEFAULT_URL.into()),
            model: var("NOSANA_LLM_MODEL"),
            key: llm_key,
        });
        Ok(Self {
            bind,
            public_url,
            data_dir: var("DUET_DATA").map_or_else(|| PathBuf::from("data"), PathBuf::from),
            session_ms: session_length("DUET_SESSION_SECS", secs("DUET_SESSION_SECS", 900_000)),
            long_session_ms: session_length(
                "DUET_LONG_SESSION_SECS",
                secs("DUET_LONG_SESSION_SECS", 2_700_000),
            ),
            rematch_after_ms: secs("DUET_REMATCH_AFTER_SECS", 86_400_000),
            rematch_window_ms: secs("DUET_REMATCH_WINDOW_SECS", 1_200_000),
            rematch_early_ms: secs("DUET_REMATCH_EARLY_SECS", 120_000),
            vote_wait_ms: secs("DUET_VOTE_SECS", 90_000),
            join_wait_ms: secs("DUET_JOIN_WAIT_SECS", 90_000),
            settle_ms: secs("DUET_SETTLE_SECS", 2_500),
            max_gather_ms: secs("DUET_GATHER_SECS", 9_000),
            plan_budget_ms: secs("DUET_PLAN_SECS", 12_000),
            rooms,
            activities: activities(var("DUET_ACTIVITIES").as_deref()),
            warm_rooms: var("DUET_WARM_ROOMS")
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            warm_max_age_ms: secs("DUET_WARM_MAX_SECS", 1_200_000),
            admin_key: var("DUET_ADMIN_KEY").unwrap_or_else(|| duet_room::text::token(16)),
            daytona,
            room_bin_dir: var("DUET_ROOM_BIN_DIR")
                .map_or_else(|| PathBuf::from("dist"), PathBuf::from),
            llm,
        })
    }

    /// Short clocks and no sponsors. Used by the tests.
    pub fn quick(data_dir: PathBuf) -> Self {
        Self {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_url: "http://test".into(),
            data_dir,
            session_ms: 1_500,
            long_session_ms: 2_000,
            rematch_after_ms: 1_000,
            rematch_window_ms: 20_000,
            rematch_early_ms: 200,
            vote_wait_ms: 1_500,
            join_wait_ms: 1_000,
            settle_ms: 100,
            max_gather_ms: 400,
            plan_budget_ms: 2_000,
            rooms: RoomsMode::Local,
            warm_rooms: 0,
            activities: crate::planner::ACTIVITIES.map(String::from).to_vec(),
            warm_max_age_ms: 60_000,
            admin_key: "test-admin".into(),
            daytona: None,
            room_bin_dir: PathBuf::from("dist"),
            llm: None,
        }
    }
}

/// The activities a lobby offers, from `DUET_ACTIVITIES`: names in any case, unknown ones left out
/// with a warning, and all of them when nothing usable is left.
pub fn activities(setting: Option<&str>) -> Vec<String> {
    let all = || crate::planner::ACTIVITIES.map(String::from).to_vec();
    let Some(setting) = setting else {
        return all();
    };
    let mut out = Vec::new();
    for name in setting
        .split(',')
        .map(|k| k.trim().to_lowercase())
        .filter(|k| !k.is_empty())
    {
        if crate::planner::ACTIVITIES.contains(&name.as_str()) {
            if !out.contains(&name) {
                out.push(name);
            }
        } else {
            eprintln!(
                "duet: DUET_ACTIVITIES: no activity called {name:?} (there are: {})",
                crate::planner::ACTIVITIES.join(", ")
            );
        }
    }
    if out.is_empty() {
        eprintln!("duet: DUET_ACTIVITIES names none that exist, so all are offered");
        return all();
    }
    out
}

/// A room runs between 3 seconds and 3 hours. A setting outside that would make every room refuse
/// its plan, so it is brought into range, and said.
fn session_length(name: &str, ms: u64) -> u64 {
    let fixed = ms.clamp(3_000, 3 * 3600 * 1000);
    if fixed != ms {
        eprintln!(
            "duet: {name} must be between 3 and 10800 seconds; using {}",
            fixed / 1000
        );
    }
    fixed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activities_are_read_whatever_the_case_and_never_left_empty() {
        assert_eq!(
            activities(Some(" Story, thisorthat ,story")),
            ["story", "thisorthat"]
        );
        assert_eq!(activities(Some("Sketch")), ["sketch"]);
        assert_eq!(
            activities(Some("nonsense")).len(),
            crate::planner::ACTIVITIES.len()
        );
        assert_eq!(activities(None).len(), crate::planner::ACTIVITIES.len());
    }

    #[test]
    fn a_session_length_the_room_would_refuse_is_brought_into_range() {
        assert_eq!(session_length("X", 2_000), 3_000);
        assert_eq!(session_length("X", 90_000), 90_000);
        assert_eq!(session_length("X", 99 * 3600 * 1000), 3 * 3600 * 1000);
    }
}
