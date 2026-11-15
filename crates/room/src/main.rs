//! `duet-room`: one pair's room, running alone inside a sandbox.
//!
//! It starts idle. The lobby arms it with the pair's plan (`POST /internal/arm`), reads its
//! status while the session runs, and deletes the whole sandbox at the end. The room holds no
//! keys and calls nobody.

use std::net::SocketAddr;
use std::time::Duration;

use duet_room::standalone::{Standalone, router};
use duet_room::{Phase, RoomConfig};

fn arg(name: &str) -> Option<String> {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == name {
            return args.next();
        }
        if let Some(v) = a.strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
            return Some(v.to_string());
        }
    }
    None
}

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("duet-room {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(3000);
    let host = arg("--host").unwrap_or_else(|| "0.0.0.0".into());
    let secret = arg("--secret")
        .or_else(|| std::env::var("DUET_ROOM_SECRET").ok())
        .unwrap_or_default();
    if secret.len() < 16 {
        eprintln!(
            "duet-room: a secret of at least 16 characters is required (--secret or DUET_ROOM_SECRET)"
        );
        std::process::exit(2);
    }
    let app = Standalone::new(secret);

    // A plan on disk arms the room at once. Otherwise it waits for the lobby.
    if let Some(path) = arg("--config") {
        let plan = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<RoomConfig>(&b).map_err(|e| e.to_string()))
            .and_then(RoomConfig::checked);
        if let Err(why) = plan.and_then(|cfg| app.arm(cfg).map_err(str::to_string)) {
            eprintln!("duet-room: {path}: {why}");
            std::process::exit(2);
        }
    }

    // Ten minutes after the session is over nobody needs this process any more.
    let watched = app.clone();
    tokio::spawn(async move {
        let mut over_for = 0u32;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            match watched.room() {
                Some(room) if room.phase() == Phase::Over => over_for += 5,
                _ => over_for = 0,
            }
            if over_for >= 600 {
                std::process::exit(0);
            }
        }
    });

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .expect("--host and --port must form an address");
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .unwrap_or_else(|e| {
            eprintln!("duet-room: cannot listen on {addr}: {e}");
            std::process::exit(1);
        });
    println!("duet-room listening on {}", listener.local_addr().unwrap());
    let stop = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    axum::serve(listener, router(app))
        .with_graceful_shutdown(stop)
        .await
        .unwrap();
}
