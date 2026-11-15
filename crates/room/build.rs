use duet_embed::build::{Mount, pack};
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../web");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let shared = root.join("shared");
    pack(
        &out,
        "room",
        &[
            Mount {
                dir: &root.join("room"),
                prefix: "",
            },
            Mount {
                dir: &shared.join("fonts"),
                prefix: "/fonts",
            },
        ],
        &shared,
    );
}
