//! Two real connections through a whole short session, over real sockets.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::Request;
use duet_room::config::{Activity, Bi, HostScript, Person, RoomConfig, Runtime};
use duet_room::{Phase, Room};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

const KEY_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaa";
const KEY_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbb";
const SECRET: &str = "the-lobby-secret-123456";

fn config(session_ms: u64) -> RoomConfig {
    RoomConfig {
        room_id: "r_it".into(),
        people: [
            Person {
                key: KEY_A.into(),
                name: "Ana".into(),
                color: "#ffb84d".into(),
                lang: "en".into(),
            },
            Person {
                key: KEY_B.into(),
                name: "Aoi".into(),
                color: "#ff4f8b".into(),
                lang: "ja".into(),
            },
        ],
        activity: Activity {
            kind: "wavelength".into(),
            title: Bi::new("Same Wavelength", "以心伝心"),
            data: json!({"prompts": []}),
            seed: 1,
        },
        host: HostScript {
            why: Bi::new("why", "なぜ"),
            opener: Bi::default(),
            nudges: vec![],
            last_minute: Bi::default(),
        },
        session_ms,
        join_wait_ms: 5_000,
        away_ms: 5_000,
        nudge_after_ms: 60_000,
        return_url: "/".into(),
        runtime: Runtime {
            kind: "local".into(),
            id: "r_it".into(),
        },
        long: false,
        alternatives: Vec::new(),
    }
    .checked()
    .unwrap()
}

/// Serves one room on a free port, the way the lobby does for local rooms.
async fn serve(session_ms: u64) -> (String, Arc<Room>) {
    let room = Room::new(config(session_ms), SECRET.into());
    room.run();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let shared = room.clone();
    let app = axum::Router::new()
        .fallback(move |req: Request| duet_room::http::serve_in(shared.clone(), req));
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, room)
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>;

async fn open(addr: &str, key: &str, after: u64) -> (Socket, Value) {
    let (mut ws, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/ws?k={key}&after={after}"))
            .await
            .unwrap();
    let hello = next(&mut ws).await;
    assert_eq!(hello["t"], "hello");
    (ws, hello)
}

async fn next(ws: &mut Socket) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("no message in 5 s")
            .expect("closed")
            .unwrap();
        if let Message::Text(text) = msg {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

/// Reads until a message of this type arrives.
async fn until(ws: &mut Socket, t: &str) -> Value {
    loop {
        let v = next(ws).await;
        if v["t"] == t {
            return v;
        }
    }
}

async fn say(ws: &mut Socket, v: Value) {
    ws.send(Message::Text(v.to_string().into())).await.unwrap();
}

/// A bare HTTP request, so the test needs no client library.
async fn http(
    addr: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.unwrap();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        let _ = write!(req, "{k}: {v}\r\n");
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).to_string();
    let status = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn a_whole_session() {
    let (addr, room) = serve(4_000).await;

    // Strangers are refused before any socket opens.
    assert!(
        tokio_tungstenite::connect_async(format!("ws://{addr}/ws?k=wrong"))
            .await
            .is_err()
    );
    assert!(
        tokio_tungstenite::connect_async(format!("ws://{addr}/ws"))
            .await
            .is_err()
    );

    let (mut a, hello) = open(&addr, KEY_A, 0).await;
    assert_eq!(hello["phase"], "waiting");
    assert_eq!(hello["you"], 0);
    assert_eq!(hello["room"]["people"][1]["name"], "Aoi");
    let (mut b, hello) = open(&addr, KEY_B, 0).await;
    assert_eq!(hello["phase"], "live");
    let ends = until(&mut a, "phase").await;
    assert_eq!(ends["p"], "live");

    // Chat reaches both, in the same order, with the sender's own id echoed back.
    say(
        &mut a,
        json!({"t": "chat", "mid": "a1", "text": "hello Aoi"}),
    )
    .await;
    let at_b = until(&mut b, "chat").await;
    let at_a = until(&mut a, "chat").await;
    assert_eq!(at_b, at_a);
    assert_eq!(at_a["mid"], "a1");
    assert_eq!(at_a["text"], "hello Aoi");

    // Typing is passed on, but never back to the one typing.
    say(&mut a, json!({"t": "live", "k": "typing", "d": true})).await;
    assert_eq!(until(&mut b, "live").await["k"], "typing");

    // A private answer stays private until both are in.
    say(&mut a, json!({"t": "seal", "slot": "q1", "v": "ramen"})).await;
    assert_eq!(until(&mut b, "sealed").await["by"], 0);
    say(&mut b, json!({"t": "seal", "slot": "q1", "v": "ramen"})).await;
    assert_eq!(
        until(&mut a, "reveal").await["v"],
        json!(["ramen", "ramen"])
    );

    // The clock answers, so both screens can agree on the time left.
    say(&mut a, json!({"t": "ping", "c": 42.5})).await;
    let pong = until(&mut a, "pong").await;
    assert_eq!(pong["c"], 42.5);
    assert!(pong["now"].as_u64().is_some());

    // A page that drops and returns gets exactly what it missed.
    let seen = at_b["s"].as_u64().unwrap();
    drop(b);
    say(
        &mut a,
        json!({"t": "chat", "mid": "a2", "text": "still there?"}),
    )
    .await;
    until(&mut a, "chat").await;
    let (mut b, hello) = open(&addr, KEY_B, seen).await;
    let missed: Vec<&Value> = hello["log"].as_array().unwrap().iter().collect();
    assert!(missed.iter().all(|e| e["s"].as_u64().unwrap() > seen));
    assert!(
        missed
            .iter()
            .any(|e| e["t"] == "chat" && e["text"] == "still there?")
    );
    assert!(!missed.iter().any(|e| e["text"] == "hello Aoi"));

    // Garbage and oversized input do not disturb the room.
    a.send(Message::Text("not json".into())).await.unwrap();
    a.send(Message::Text(
        json!({"t": "act", "d": "x".repeat(40_000)})
            .to_string()
            .into(),
    ))
    .await
    .unwrap();
    assert_eq!(until(&mut a, "err").await["code"], "big");

    // The lobby reads numbers, never words, and only with the secret.
    assert_eq!(http(&addr, "GET", "/internal/status", &[], "").await.0, 403);
    assert_eq!(
        http(
            &addr,
            "GET",
            "/internal/status",
            &[("x-duet-secret", KEY_A)],
            ""
        )
        .await
        .0,
        403
    );
    let (code, body) = http(
        &addr,
        "GET",
        "/internal/status",
        &[("x-duet-secret", SECRET)],
        "",
    )
    .await;
    assert_eq!(code, 200);
    assert!(body.contains("\"chat\":[2,0]"), "{body}");
    assert!(!body.contains("hello Aoi"));
    assert_eq!(
        http(
            &addr,
            "GET",
            "/internal/export",
            &[("x-duet-secret", SECRET)],
            ""
        )
        .await
        .0,
        403,
        "nothing was reported"
    );

    // The bell.
    let over = loop {
        let v = until(&mut b, "phase").await;
        if v["p"] == "over" {
            break v;
        }
    };
    assert_eq!(over["why"], "time");
    assert_eq!(room.phase(), Phase::Over);
    say(
        &mut a,
        json!({"t": "chat", "mid": "a3", "text": "too late"}),
    )
    .await;
    assert_eq!(until(&mut a, "err").await["code"], "closed");
}

#[tokio::test]
async fn a_page_without_a_socket_keeps_up() {
    let (addr, _room) = serve(60_000).await;
    let (mut a, _) = open(&addr, KEY_A, 0).await;

    // The second person cannot open a socket, so their page uses plain requests.
    let (code, body) = http(&addr, "GET", &format!("/hello?k={KEY_B}"), &[], "").await;
    assert_eq!(code, 200);
    let hello: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        hello["phase"], "live",
        "arriving by request counts as arriving"
    );
    let (seq, la) = (
        hello["seq"].as_u64().unwrap(),
        hello["la"].as_u64().unwrap(),
    );
    until(&mut a, "phase").await;

    let (code, body) = http(
        &addr,
        "POST",
        &format!("/send?k={KEY_B}"),
        &[("Content-Type", "application/json")],
        &json!([{"t": "chat", "mid": "b1", "text": "hi from a request"}, {"t": "ping", "c": 1}])
            .to_string(),
    )
    .await;
    assert_eq!(code, 200);
    assert!(body.contains("pong"));
    assert_eq!(until(&mut a, "chat").await["text"], "hi from a request");

    // A held request answers at once when there is news...
    say(&mut a, json!({"t": "chat", "mid": "a1", "text": "hi back"})).await;
    let (_, body) = http(
        &addr,
        "GET",
        &format!("/poll?k={KEY_B}&after={seq}&la={la}&hold=5"),
        &[],
        "",
    )
    .await;
    let got: Value = serde_json::from_str(&body).unwrap();
    let texts: Vec<&str> = got["ev"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["t"] == "chat")
        .map(|e| e["text"].as_str().unwrap())
        .collect();
    assert_eq!(texts, ["hi from a request", "hi back"]);

    // ...and waits when there is none, then answers empty.
    let (seq, la) = (got["seq"].as_u64().unwrap(), got["la"].as_u64().unwrap());
    let started = std::time::Instant::now();
    let (_, body) = http(
        &addr,
        "GET",
        &format!("/poll?k={KEY_B}&after={seq}&la={la}&hold=1"),
        &[],
        "",
    )
    .await;
    assert_eq!(body.trim(), "{}");
    assert!(started.elapsed() >= Duration::from_millis(900));

    assert_eq!(http(&addr, "GET", "/poll?k=wrong", &[], "").await.0, 403);
}

#[tokio::test]
async fn a_report_ends_the_session_and_releases_the_chat() {
    let (addr, _room) = serve(60_000).await;
    let (mut a, _) = open(&addr, KEY_A, 0).await;
    let (mut b, _) = open(&addr, KEY_B, 0).await;
    say(
        &mut b,
        json!({"t": "chat", "mid": "b1", "text": "something unkind"}),
    )
    .await;
    until(&mut a, "chat").await;
    say(&mut a, json!({"t": "report", "reason": "rude", "note": ""})).await;
    assert_eq!(until(&mut a, "reported").await["t"], "reported");
    let over = loop {
        let v = until(&mut b, "phase").await;
        if v["p"] == "over" {
            break v;
        }
    };
    assert_eq!(
        over["why"], "ended",
        "the reported person is told only that it ended"
    );
    let (code, body) = http(
        &addr,
        "GET",
        "/internal/export",
        &[("x-duet-secret", SECRET)],
        "",
    )
    .await;
    assert_eq!(code, 200);
    let export: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(export["chat"][0]["text"], "something unkind");
    assert_eq!(export["report"]["reason"], "rude");
}

#[tokio::test]
async fn the_page_and_fonts_are_served_compressed() {
    let (addr, _room) = serve(60_000).await;
    let (code, body) = http(&addr, "GET", "/", &[], "").await;
    assert_eq!(code, 200);
    assert!(body.to_lowercase().contains("<!doctype html"));
    let mut s = TcpStream::connect(&addr).await.unwrap();
    s.write_all(
        format!(
            "GET /fonts/figtree-latin.woff2 HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
        )
        .as_bytes(),
    )
    .await
    .unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.unwrap();
    let head = String::from_utf8_lossy(&raw[..300.min(raw.len())]).to_lowercase();
    assert!(head.starts_with("http/1.1 200"));
    assert!(head.contains("font/woff2"));
    assert!(head.contains("immutable"));
    assert_eq!(
        http(&addr, "GET", "/fonts/../../etc/passwd", &[], "")
            .await
            .0,
        404
    );
}
