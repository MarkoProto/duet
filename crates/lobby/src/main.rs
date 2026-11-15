//! `duet`: the lobby.
//!
//!     duet            run the lobby
//!     duet doctor     test the sponsor connections, step by step
//!     duet --version

use std::time::Duration;

use duet::lobby::App;
use duet::settings::Settings;

#[tokio::main]
async fn main() {
    let command = std::env::args().nth(1).unwrap_or_default();
    if command == "--version" || command == "-V" {
        println!("duet {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let cfg = match Settings::from_env() {
        Ok(cfg) => cfg,
        Err(why) => {
            eprintln!("duet: {why}");
            std::process::exit(2);
        }
    };
    if command == "doctor" {
        std::process::exit(i32::from(!duet::doctor::run(&cfg).await));
    }
    if !command.is_empty() && command != "serve" {
        eprintln!(
            "duet: unknown command {command:?}. Use `duet`, `duet doctor` or `duet --version`."
        );
        std::process::exit(2);
    }

    let app = App::new(cfg);
    // The port first: a second lobby started by mistake stops here, before its sweep could remove
    // the rooms of the one already running.
    let listener = tokio::net::TcpListener::bind(app.cfg.bind)
        .await
        .unwrap_or_else(|e| {
            eprintln!("duet: cannot listen on {}: {e}", app.cfg.bind);
            std::process::exit(1);
        });
    app.run();
    if let Some(daytona) = app.daytona() {
        println!("duet: Daytona: {}", daytona.api.pick().await);
        // Rooms left behind by a lobby that crashed would otherwise live until their own deadline.
        match daytona.sweep().await {
            Ok(0) => {}
            Ok(n) => println!("duet: removed {n} sandbox(es) left from an earlier run"),
            Err(why) => eprintln!("duet: could not look for leftover sandboxes: {why}"),
        }
    }
    // Only now, so the sweep above cannot remove them.
    app.keep_warm();
    let sponsors = app.sponsors();
    println!(
        "duet {} on http://{}",
        env!("CARGO_PKG_VERSION"),
        listener.local_addr().unwrap()
    );
    println!("  people open   {}", app.cfg.public_url);
    println!("  live numbers  {}/hood", app.cfg.public_url);
    println!(
        "  admin         {}/hood#key={}   (keep this one to yourself)",
        app.cfg.public_url, app.cfg.admin_key
    );
    println!(
        "  rooms         {}",
        if sponsors["rooms"] == "daytona" {
            "Daytona sandboxes"
        } else {
            "local (inside this process, not a sponsor run)"
        }
    );
    println!(
        "  pairing help  {}",
        if sponsors["ai"] == "nosana" {
            "a model on Nosana"
        } else {
            "rules only (no model set)"
        }
    );
    if sponsors["rooms"] == "daytona" {
        println!(
            "  built ahead   {} room(s), replaced after {} s unused",
            app.cfg.warm_rooms.min(duet::lobby::MAX_WARM),
            app.cfg.warm_max_age_ms / 1000
        );
    }
    println!(
        "  session       {} s, then {} s for a second one",
        app.cfg.session_ms / 1000,
        app.cfg.long_session_ms / 1000
    );

    let saver = app.clone();
    let stop = async move {
        let _ = tokio::signal::ctrl_c().await;
        saver.save();
        // Rooms built ahead are removed now rather than at the end of their own lifetime.
        saver.close_warm().await;
        // Give open pages a moment to hear that the lobby is going.
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let router = duet::web::router(app);
    if let Err(e) = axum::serve(listener, router)
        .with_graceful_shutdown(stop)
        .await
    {
        eprintln!("duet: {e}");
    }
}
