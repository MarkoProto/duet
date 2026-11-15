//! One pair's room. The same code runs as its own program inside a sandbox (`duet-room`) and
//! inside the lobby for local rooms.

pub mod assets;
pub mod config;
pub mod http;
pub mod limit;
pub mod standalone;
pub mod state;
pub mod text;

pub use config::{Activity, Bi, HostScript, Person, RoomConfig, Runtime};
pub use state::{ClientMsg, Frame, Phase, Room};
