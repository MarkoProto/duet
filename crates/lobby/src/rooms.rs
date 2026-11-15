//! Where a pair's room runs: inside this process (a local room), or in its own Daytona sandbox.
//!
//! Both are driven the same way: prepare an idle room, arm it with the pair's plan, read its
//! status while it runs, copy the chat out if someone reported it, destroy it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use duet_room::{Room, RoomConfig};
use serde::Serialize;
use serde_json::Value;

use crate::daytona::DaytonaHost;

pub type LocalRooms = Arc<Mutex<HashMap<String, Arc<Room>>>>;

/// Rooms in Daytona sandboxes that people reach through the lobby, by the room's path id: the
/// sandbox address the lobby passes their requests on to. People never see Daytona's address, so
/// they never meet its preview warning page, which would also drop the key after the `#`.
pub type Proxied = Arc<Mutex<HashMap<String, String>>>;

/// A room that exists and waits for its plan.
#[derive(Clone, Debug)]
pub struct Ready {
    /// `daytona` or `local`.
    pub kind: &'static str,
    /// The sandbox id, or the local room id.
    pub id: String,
    /// What the lobby shows the room to prove it is the lobby.
    pub secret: String,
    /// For a sandbox: the signed address the lobby reaches the room on.
    pub control: Option<String>,
}

/// Proof of how a room ended.
#[derive(Clone, Debug, Serialize)]
pub struct Receipt {
    pub kind: String,
    pub id: String,
    /// How long the delete took.
    pub ms: u64,
    /// Whether the room was checked afterwards and found gone.
    pub gone: bool,
}

/// Called as each step of building a room starts (`false`) and finishes (`true`).
pub type OnStage<'a> = &'a (dyn Fn(&'static str, bool) + Send + Sync);

pub enum Host {
    Local {
        rooms: LocalRooms,
        public_url: String,
    },
    Daytona(Box<DaytonaHost>),
}

impl Host {
    pub fn kind(&self) -> &'static str {
        match self {
            Host::Local { .. } => "local",
            Host::Daytona(_) => "daytona",
        }
    }

    /// Brings up an idle room that will live at most `ttl_ms`.
    pub async fn prepare(
        &self,
        room_id: &str,
        ttl_ms: u64,
        on: OnStage<'_>,
    ) -> Result<Ready, String> {
        match self {
            Host::Local { .. } => {
                on("create", false);
                on("create", true);
                Ok(Ready {
                    kind: "local",
                    id: room_id.to_string(),
                    secret: duet_room::text::token(18),
                    control: None,
                })
            }
            Host::Daytona(d) => d.prepare(room_id, ttl_ms, on).await,
        }
    }

    /// Hands the room its plan. Returns each person's own link.
    pub async fn arm(&self, ready: &Ready, cfg: RoomConfig) -> Result<[String; 2], String> {
        match self {
            Host::Local { rooms, public_url } => {
                let keys = [cfg.people[0].key.clone(), cfg.people[1].key.clone()];
                let room = Room::new(cfg.checked()?, ready.secret.clone());
                room.run();
                rooms
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(ready.id.clone(), room);
                Ok(keys.map(|k| format!("{public_url}/r/{}/#k={k}", ready.id)))
            }
            Host::Daytona(d) => d.arm(ready, cfg).await,
        }
    }

    fn local(&self, ready: &Ready) -> Option<Arc<Room>> {
        match self {
            Host::Local { rooms, .. } => rooms
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(&ready.id)
                .cloned(),
            Host::Daytona(_) => None,
        }
    }

    /// Numbers and state. Never words.
    pub async fn status(&self, ready: &Ready) -> Result<Value, String> {
        match self {
            Host::Local { .. } => self
                .local(ready)
                .map(|r| r.status())
                .ok_or_else(|| "the room is gone".to_string()),
            Host::Daytona(d) => d.status(ready).await,
        }
    }

    /// The chat, which a room gives up only after a report.
    pub async fn export(&self, ready: &Ready) -> Result<Option<Value>, String> {
        match self {
            Host::Local { .. } => Ok(self.local(ready).and_then(|r| r.export())),
            Host::Daytona(d) => d.export(ready).await,
        }
    }

    /// Ends the session early, for when the lobby's own clock says it is over.
    pub async fn end(&self, ready: &Ready) {
        match self {
            Host::Local { .. } => {
                if let Some(room) = self.local(ready) {
                    room.end("time");
                }
            }
            Host::Daytona(d) => d.end(ready).await,
        }
    }

    /// Destroys the room and says how that went.
    pub async fn destroy(&self, ready: &Ready) -> Receipt {
        let started = Instant::now();
        match self {
            Host::Local { rooms, .. } => {
                let gone = {
                    let mut rooms = rooms
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    rooms.remove(&ready.id);
                    !rooms.contains_key(&ready.id)
                };
                Receipt {
                    kind: "local".into(),
                    id: ready.id.clone(),
                    ms: started.elapsed().as_millis() as u64,
                    gone,
                }
            }
            Host::Daytona(d) => d.destroy(ready).await,
        }
    }
}
