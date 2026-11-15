//! `duet doctor`: the first real contact with the sponsors, one step at a time.
//!
//! It makes a sandbox, moves the room program in, starts it, arms it, reads it, destroys it, and
//! then asks the model one question. Each step prints how long it took. The first failure stops
//! the run and prints what came back, so the fix can be made in minutes rather than guessed at.

use std::time::Instant;

use duet_room::text::token;
use duet_room::{Bi, Person, RoomConfig, Runtime};

use crate::daytona::DaytonaHost;
use crate::llm::Llm;
use crate::planner::{self, Pair};
use crate::profile::Answers;
use crate::settings::Settings;

fn step<T>(name: &str, started: Instant, result: Result<T, String>) -> Result<T, String> {
    let ms = started.elapsed().as_millis();
    match &result {
        Ok(_) => println!("  ok    {name:<44} {ms:>6} ms"),
        Err(why) => println!("  FAIL  {name:<44} {ms:>6} ms\n        {why}"),
    }
    result
}

fn sample_pair() -> (crate::profile::Profile, crate::profile::Profile) {
    let make = |mode: &str, talk: &str, interests: &[&str]| {
        Answers {
            langs: vec!["en".into(), "ja".into()],
            mood: "calm".into(),
            mode: mode.into(),
            style: "team".into(),
            talk: talk.into(),
            interests: interests.iter().map(ToString::to_string).collect(),
            smile: String::new(),
        }
        .profile()
    };
    (
        make("talk", "talker", &["music", "food"]),
        make("make", "listener", &["music", "art"]),
    )
}

async fn daytona(cfg: &Settings) -> Result<(), String> {
    let Some(settings) = &cfg.daytona else {
        println!(
            "  skip  Daytona: no key (set DAYTONA_API_KEY, or put it in ~/Secrets/Daytona/key)"
        );
        return Ok(());
    };
    let host = DaytonaHost::new(settings, cfg.room_bin_dir.clone(), "doctor".into());
    let t = Instant::now();
    let chosen = host.api.pick().await;
    step("API chosen", t, Ok::<(), String>(()))?;
    println!("        {chosen}");
    if host.api.is_v2() {
        match host.api.whoami().await {
            Ok(me) => println!(
                "        key: {} · may: {}",
                me.get("name").and_then(|v| v.as_str()).unwrap_or("?"),
                me.get("permissions")
                    .map(ToString::to_string)
                    .unwrap_or_default()
            ),
            Err(why) => println!("        could not read the key's details: {why}"),
        }
    }
    let whole = Instant::now();
    let on = |key: &'static str, done: bool| {
        if done {
            println!("        . {key}");
        }
    };
    let t = Instant::now();
    let ready = step(
        "sandbox created, room program moved in and started",
        t,
        host.prepare("r_doctor", 5 * 60_000, &on).await,
    )?;
    println!("        sandbox {}", ready.id);
    let result = async {
        let keys = [token(18), token(18)];
        let plan = {
            let (a, b) = sample_pair();
            planner::rules(&Pair { people: [&a, &b], smiles: ["", ""], avoid: &[], liked: [&[], &[]], seed: 1 })
        };
        let room = RoomConfig {
            room_id: "r_doctor".into(),
            people: [0, 1].map(|i| Person { key: keys[i].clone(), name: format!("Doctor {}", i + 1), color: String::new(), lang: "en".into() }),
            activity: plan.activity,
            host: plan.host,
            session_ms: 60_000,
            join_wait_ms: 60_000,
            away_ms: 60_000,
            nudge_after_ms: 20_000,
            return_url: cfg.public_url.clone(),
            runtime: Runtime { kind: "daytona".into(), id: ready.id.clone() },
            long: false,
            alternatives: Vec::new(),
        };
        let t = Instant::now();
        let urls = step("room armed, two signed links made", t, host.arm(&ready, room).await)?;
        let t = Instant::now();
        let status = step("status read through the signed link", t, host.status(&ready).await)?;
        if status["phase"] != "waiting" {
            return Err(format!("unexpected status: {status}"));
        }
        // What a browser will do: open the page, then the socket.
        let http = reqwest::Client::new();
        let page = urls[0].split('#').next().unwrap_or("").to_string();
        let t = Instant::now();
        let fetched = http.get(&page).send().await.map_err(|e| e.to_string());
        let fetched = match fetched {
            Ok(r) if r.status().is_success() => r.text().await.map_err(|e| e.to_string()),
            Ok(r) => Err(format!("the page answered {}", r.status())),
            Err(e) => Err(e),
        };
        let body = step("room page fetched as a browser would", t, fetched)?;
        if !body.to_lowercase().contains("<title>duet") {
            println!("        note: the page did not come back as our room. Daytona may show its preview warning first,\n        which people would see once before the room. It can be turned off for the organization in\n        Daytona's settings. First 200 characters:\n        {}", body.chars().take(200).collect::<String>().replace('\n', " "));
        }
        let t = Instant::now();
        let hello = http.get(format!("{page}hello?k={}", keys[0])).send().await.map_err(|e| e.to_string());
        let hello = match hello {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(format!("the room answered {}", r.status())),
            Err(e) => Err(e),
        };
        step("a person's key opens the room (held-request path)", t, hello)?;
        println!("        open these two in two browsers to try the socket by hand (valid 5 minutes, if you answer n below):\n        {}\n        {}", urls[0], urls[1]);
        Ok(())
    }
    .await;
    let t = Instant::now();
    let receipt = host.destroy(&ready).await;
    step(
        "sandbox deleted and confirmed gone",
        t,
        if receipt.gone {
            Ok(())
        } else {
            Err(format!("deleted in {} ms but still listed", receipt.ms))
        },
    )?;
    result?;
    println!(
        "  Daytona: all steps passed in {} ms",
        whole.elapsed().as_millis()
    );
    Ok(())
}

async fn nosana(cfg: &Settings) -> Result<(), String> {
    let Some(settings) = &cfg.llm else {
        println!(
            "  skip  Nosana: not set (NOSANA_API_KEY or ~/Secrets/Nosana/key, and NOSANA_LLM_URL if one was handed out)"
        );
        return Ok(());
    };
    println!("Nosana at {}", settings.url);
    let llm = Llm::new(&settings.url, settings.model.clone(), settings.key.clone());
    let t = Instant::now();
    let model = step(
        "model chosen",
        t,
        llm.model().await.map_err(|e| e.to_string()),
    )?;
    println!("        {model}");
    let (a, b) = sample_pair();
    let pair = Pair {
        people: [&a, &b],
        smiles: ["a good coffee", "my dog"],
        avoid: &[],
        liked: [&[], &[]],
        seed: 1,
    };
    let (system, user) = planner::question(&pair);
    let t = Instant::now();
    let answer = step(
        "a pair's plan asked for and answered as JSON",
        t,
        llm.ask(&system, &user, 1400, std::time::Duration::from_secs(40))
            .await
            .map_err(|e| e.to_string()),
    )?;
    let (plan, taken) = planner::merge(&pair, &answer.json);
    println!(
        "        activity {}, {taken} of {} parts accepted from the model",
        plan.activity.kind,
        planner::parts(&plan.activity.kind)
    );
    let show = |label: &str, line: &Bi| println!("        {label}: {} / {}", line.en, line.ja);
    show("why", &plan.host.why);
    show("opener", &plan.host.opener);
    if taken < 4 {
        println!(
            "        note: fewer than 4 parts passed the checks. The raw answer was:\n        {}",
            answer.json
        );
    }

    // The other two jobs: reading someone's own words, and the note after both said yes.
    let note = "Long day, a bit tired. I'd love to draw something together, I'm more of a listener. Into music and cats.";
    let (system, user) = crate::reader::question(note);
    let t = Instant::now();
    let read = step(
        "own words read into the taps",
        t,
        llm.ask_at(&system, &user, 300, std::time::Duration::from_secs(40), 0.0)
            .await
            .map_err(|e| e.to_string()),
    )?;
    let guess = crate::reader::merge(crate::reader::rules(note), &read.json);
    println!(
        "        {} of 7 fields from the model: {}",
        guess.by_model,
        guess.view()["answers"]
    );
    let facts = crate::notes::facts(
        "wavelength",
        &serde_json::json!({"rounds": 8, "synced": 5}),
        900,
        45,
    );
    let (system, user) = crate::notes::question(&facts);
    let t = Instant::now();
    let written = step(
        "the note for round two written",
        t,
        llm.ask_at(&system, &user, 400, std::time::Duration::from_secs(40), 0.4)
            .await
            .map_err(|e| e.to_string()),
    )?;
    match crate::notes::checked(&facts, &written.json) {
        Some(n) => println!("        {} / {}", n.en, n.ja),
        None => println!(
            "        note: the model's note did not pass the checks (an invented number, a link, or one language missing), so the rules' note would be shown. It wrote:\n        {}",
            written.json
        ),
    }
    Ok(())
}

/// Runs both checks. Returns whether everything that was attempted passed.
pub async fn run(cfg: &Settings) -> bool {
    println!("duet doctor\n");
    let d = daytona(cfg).await;
    println!();
    let n = nosana(cfg).await;
    println!();
    match (&d, &n) {
        (Ok(()), Ok(())) if cfg.daytona.is_none() && cfg.llm.is_none() => {
            println!("Nothing was tested: no keys were found.");
            true
        }
        (Ok(()), Ok(())) => {
            println!("Done. Nothing that was tried failed.");
            true
        }
        _ => {
            println!(
                "Stopped at the first failure above. The request shapes are in docs/daytona-rest-spec.md (original API)\nand docs/daytona-v2-rest-spec.md (new API). DUET_DAYTONA_API=v0 or v2 forces one."
            );
            false
        }
    }
}
