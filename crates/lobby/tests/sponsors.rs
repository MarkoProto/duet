//! The lobby against stand-ins for the two sponsors.
//!
//! The stand-in Daytona answers the same web requests the real one documents, and when asked to
//! start the room program it starts a real room on a local port. The stand-in Nosana answers like
//! a model: well, badly, or late. Together they prove the lobby's own logic (the order of calls,
//! the fallbacks, what is kept) without a key. They cannot prove that the real services accept
//! these requests: that is what `duet doctor` is for.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Multipart, Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use duet::daytona::DaytonaHost;
use duet::llm::Llm;
use duet::lobby::{App, Poll};
use duet::profile::Answers;
use duet::rooms::{Host, LocalRooms};
use duet::settings::{DaytonaSettings, Settings};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

// ---- a stand-in for Daytona ----------------------------------------------------------------

#[derive(Default)]
struct FakeBox {
    gets: u32,
    room_port: Option<u16>,
    server: Option<tokio::task::JoinHandle<()>>,
    body: Value,
}

#[derive(Default)]
struct FakeDaytona {
    base: Mutex<String>,
    log: Mutex<Vec<String>>,
    boxes: Mutex<HashMap<String, FakeBox>>,
    uploads: Mutex<Vec<(String, String, usize)>>,
    fail_create: AtomicBool,
    next: AtomicU32,
    /// The new API does not know the key (an account that only has the old API).
    v2_refuses: AtomicBool,
    /// The new API knows the key but may not create Linux sandboxes for this account.
    v2_no_linux: AtomicBool,
}

type Fake = Arc<FakeDaytona>;

fn authed(
    fake: &FakeDaytona,
    headers: &HeaderMap,
    what: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    fake.log.lock().unwrap().push(what.to_string());
    let ok = headers.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer test-key");
    if ok {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"statusCode": 401, "message": "Unauthorized"})),
        ))
    }
}

fn sandbox_json(fake: &FakeDaytona, id: &str, state: &str) -> Value {
    json!({"id": id, "name": id, "state": state, "toolboxProxyUrl": format!("{}/toolbox", fake.base.lock().unwrap()), "labels": {}, "env": {}})
}

async fn create(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "POST /sandbox")?;
    if fake.fail_create.load(Ordering::Relaxed) {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            Json(
                json!({"statusCode": 429, "message": "Total CPU limit exceeded", "code": "LIMIT_EXCEEDED"}),
            ),
        ));
    }
    let id = format!(
        "sbx-{:04}-aaaa-bbbb",
        fake.next.fetch_add(1, Ordering::Relaxed)
    );
    fake.boxes.lock().unwrap().insert(
        id.clone(),
        FakeBox {
            body,
            ..Default::default()
        },
    );
    // Like the real one, it is not ready the moment it is created.
    Ok(Json(sandbox_json(&fake, &id, "creating")))
}

async fn get_one(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "GET /sandbox/:id")?;
    let mut boxes = fake.boxes.lock().unwrap();
    let Some(b) = boxes.get_mut(&id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"statusCode": 404, "message": "Sandbox not found"})),
        ));
    };
    b.gets += 1;
    let state = if b.gets >= 2 { "started" } else { "starting" };
    drop(boxes);
    Ok(Json(sandbox_json(&fake, &id, state)))
}

async fn list(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "GET /sandbox")?;
    let wanted: Value =
        serde_json::from_str(q.get("labels").map_or("{}", String::as_str)).unwrap_or(json!({}));
    let boxes = fake.boxes.lock().unwrap();
    let items: Vec<Value> = boxes
        .iter()
        .filter(|(_, b)| {
            wanted
                .as_object()
                .is_some_and(|w| w.iter().all(|(k, v)| b.body["labels"].get(k) == Some(v)))
        })
        .map(|(id, b)| {
            let mut item = sandbox_json(&fake, id, "started");
            item["labels"] = b.body["labels"].clone();
            item
        })
        .collect();
    Ok(Json(json!({"items": items, "nextCursor": null})))
}

async fn delete(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "DELETE /sandbox/:id")?;
    match fake.boxes.lock().unwrap().remove(&id) {
        Some(b) => {
            if let Some(server) = b.server {
                server.abort(); // the room dies with its sandbox
            }
            Ok(Json(sandbox_json(&fake, &id, "destroying")))
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"statusCode": 404, "message": "Sandbox not found"})),
        )),
    }
}

async fn signed(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((id, port)): Path<(String, u16)>,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "GET signed-preview-url")?;
    let expires: u64 = q
        .get("expiresInSeconds")
        .and_then(|v| v.parse().ok())
        .unwrap_or(60);
    assert!(
        expires > 60,
        "the lobby must ask for a lifetime, the default of 60 s is too short"
    );
    assert_eq!(port, 3000);
    let boxes = fake.boxes.lock().unwrap();
    let room = boxes.get(&id).and_then(|b| b.room_port).ok_or((
        StatusCode::BAD_REQUEST,
        Json(json!({"message": "nothing listens on that port"})),
    ))?;
    // A made-up token for the stand-in Daytona. gitleaks:allow
    let token = "s0123456789abcdef0123456789abcdef"; // gitleaks:allow
    Ok(Json(
        json!({"sandboxId": id, "port": port, "token": token, "url": format!("http://127.0.0.1:{room}")}),
    ))
}

async fn execute(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    let command = body["command"].as_str().unwrap_or("").to_string();
    authed(
        &fake,
        &headers,
        &format!(
            "POST execute `{}`",
            command.split_whitespace().next().unwrap_or("")
        ),
    )?;
    assert!(
        body["timeout"].is_u64(),
        "always send a timeout: the daemon's own default is short"
    );
    assert!(fake.boxes.lock().unwrap().contains_key(&id));
    let result = match command.as_str() {
        "uname -m" => "aarch64\n",
        "pwd" => "/home/daytona\n",
        c if c.starts_with("chmod +x /home/daytona/duet-room") => "",
        other => panic!("unexpected command: {other}"),
    };
    Ok(Json(json!({"exitCode": 0, "result": result})))
}

async fn upload(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
    mut form: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "POST bulk-upload")?;
    assert!(fake.boxes.lock().unwrap().contains_key(&id));
    let (mut path, mut file) = (String::new(), None);
    while let Some(field) = form.next_field().await.unwrap() {
        match field.name().unwrap_or("") {
            "files[0].path" => path = field.text().await.unwrap(),
            "files[0].file" => {
                assert!(!path.is_empty(), "the path part comes before the file part");
                let name = field.file_name().unwrap_or("").to_string();
                file = Some((name, field.bytes().await.unwrap().len()));
            }
            other => panic!("unexpected form field {other}"),
        }
    }
    let (name, size) = file.expect("a file part");
    fake.uploads
        .lock()
        .unwrap()
        .push((path.clone(), name, size));
    Ok(Json(json!({"files": [{"path": path, "bytes": size}]})))
}

async fn session(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(_id): Path<String>,
    Json(body): Json<Value>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "POST session")?;
    assert_eq!(body["sessionId"], "room");
    Ok(StatusCode::CREATED)
}

/// "Starts the room program": a real room, on a local port, with the secret from the command.
async fn session_exec(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((id, sid)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "POST session exec")?;
    assert_eq!(sid, "room");
    assert_eq!(body["runAsync"], true);
    let command = body["command"].as_str().unwrap();
    assert!(
        command.contains("/home/daytona/duet-room --port 3000"),
        "{command}"
    );
    let secret = command
        .split_whitespace()
        .find_map(|w| w.strip_prefix("DUET_ROOM_SECRET="))
        .expect("the secret travels in the command")
        .to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = duet_room::standalone::router(duet_room::standalone::Standalone::new(secret));
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut boxes = fake.boxes.lock().unwrap();
    let b = boxes.get_mut(&id).unwrap();
    b.room_port = Some(port);
    b.server = Some(server);
    Ok((StatusCode::ACCEPTED, Json(json!({"cmdId": "cmd-1"}))))
}

// ---- the same stand-in, speaking the new API (SDK 2.0) ---------------------------------------

/// Toolbox calls in the new API carry the sandbox's own token, never the API key.
fn authed_tb(
    fake: &FakeDaytona,
    headers: &HeaderMap,
    id: &str,
    what: &str,
) -> Result<(), (StatusCode, Json<Value>)> {
    fake.log.lock().unwrap().push(what.to_string());
    let want = format!("Bearer tok-{id}");
    if headers.get("authorization").and_then(|v| v.to_str().ok()) == Some(want.as_str()) {
        Ok(())
    } else {
        Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "UNAUTHORIZED", "message": "toolbox token required"})),
        ))
    }
}

async fn v2_whoami(
    State(fake): State<Fake>,
    headers: HeaderMap,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    if fake.v2_refuses.load(Ordering::Relaxed) {
        fake.log
            .lock()
            .unwrap()
            .push("v2 GET /api-keys/current".into());
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({"code": "UNAUTHORIZED", "message": "unknown key"})),
        ));
    }
    authed(&fake, &headers, "v2 GET /api-keys/current")?;
    Ok(Json(
        json!({"name": "test", "permissions": ["write:sandboxes", "delete:sandboxes"]}),
    ))
}

async fn v2_create(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 POST /sandboxes")?;
    assert!(
        headers.get("idempotency-key").is_some(),
        "every create carries an Idempotency-Key"
    );
    if fake.v2_no_linux.load(Ordering::Relaxed) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(
                json!({"code": "FEATURE_NOT_ENABLED", "message": "Linux sandboxes are not enabled for this organization"}),
            ),
        ));
    }
    let life = &body["lifecycle_policy"];
    assert_eq!(
        life["on_timeout"], "delete",
        "Daytona deletes it at the end of its lifetime"
    );
    assert_eq!(
        life["idle_expiry_seconds"], -1,
        "idle time never ends a room mid-session"
    );
    assert!(life["timeout_seconds"].as_u64().unwrap() >= 120);
    let id = format!("sbx_{:04}", fake.next.fetch_add(1, Ordering::Relaxed));
    fake.boxes.lock().unwrap().insert(
        id.clone(),
        FakeBox {
            body,
            ..Default::default()
        },
    );
    let base = fake.base.lock().unwrap().clone();
    Ok(Json(
        json!({"id": id, "name": id, "status": "pending", "toolbox_proxy_url": format!("{base}/tb2/sandboxes"),
        "access_token": format!("tok-{id}"), "wait_cap_seconds": 0}),
    ))
}

async fn v2_get(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 GET /sandboxes/:id")?;
    let mut boxes = fake.boxes.lock().unwrap();
    let Some(b) = boxes.get_mut(&id) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"code": "NOT_FOUND", "message": "Sandbox not found"})),
        ));
    };
    b.gets += 1;
    // A read does not always repeat the toolbox address; the client keeps the one from create.
    Ok(Json(
        json!({"id": id, "name": id, "class": "linux", "status": if b.gets >= 2 { "running" } else { "booting" }, "labels": b.body["labels"]}),
    ))
}

async fn v2_delete(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 DELETE /sandboxes/:id")?;
    match fake.boxes.lock().unwrap().remove(&id) {
        Some(b) => {
            if let Some(server) = b.server {
                server.abort();
            }
            Ok(StatusCode::NO_CONTENT)
        }
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"code": "NOT_FOUND", "message": "Sandbox not found"})),
        )),
    }
}

async fn v2_search(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 GET /sandboxes/search")?;
    let wanted: Value =
        serde_json::from_str(q.get("labels").map_or("{}", String::as_str)).unwrap_or(json!({}));
    let boxes = fake.boxes.lock().unwrap();
    let items: Vec<Value> = boxes
        .iter()
        .filter(|(_, b)| wanted.as_object().is_some_and(|w| w.iter().all(|(k, v)| b.body["labels"].get(k) == Some(v))))
        .map(|(id, b)| json!({"id": id, "name": id, "status": "running", "labels": b.body["labels"], "cursor": id}))
        .collect();
    let n = items.len();
    Ok(Json(
        json!({"sandboxes": items, "next_cursor": null, "total": n, "total_relation": "eq"}),
    ))
}

async fn v2_expose(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((_id, port)): Path<(String, u16)>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 PUT port")?;
    Ok(Json(json!({"port": port, "public": false})))
}

async fn v2_signed(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path((id, port)): Path<(String, u16)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 POST signed-preview-urls")?;
    assert!(
        body["expires_in_seconds"].as_u64().unwrap() > 60,
        "the default of 60 s is too short for a room"
    );
    assert_eq!(port, 3000);
    let room = fake
        .boxes
        .lock()
        .unwrap()
        .get(&id)
        .and_then(|b| b.room_port)
        .ok_or((
            StatusCode::BAD_REQUEST,
            Json(json!({"message": "nothing listens on that port"})),
        ))?;
    Ok(Json(
        json!({"url": format!("http://127.0.0.1:{room}/"), "token": "t", "expires_at": "2026-11-15T10:00:00Z"}),
    ))
}

async fn v2_mint(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed(&fake, &headers, "v2 POST access-token")?;
    Ok(Json(
        json!({"access_token": format!("tok-{id}"), "token_type": "Bearer", "expires_at": "2026-11-15T10:00:00Z"}),
    ))
}

/// One resource for both: a command that runs to its end, or the room program left running.
async fn v2_process(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    Json(body): Json<Value>,
) -> Result<(StatusCode, Json<Value>), (StatusCode, Json<Value>)> {
    let command = body["shellCommand"].as_str().unwrap_or("").to_string();
    if body["name"] == "room" {
        authed_tb(&fake, &headers, &id, "v2 start room")?;
        assert!(
            command.ends_with("/home/daytona/duet-room --port 3000"),
            "{command}"
        );
        assert!(
            !command.contains("SECRET"),
            "the secret is never in the command, which every read echoes back"
        );
        let secret = body["env"]["DUET_ROOM_SECRET"]
            .as_str()
            .expect("the secret travels in env")
            .to_string();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = duet_room::standalone::router(duet_room::standalone::Standalone::new(secret));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut boxes = fake.boxes.lock().unwrap();
        let b = boxes.get_mut(&id).unwrap();
        b.room_port = Some(port);
        b.server = Some(server);
        return Ok((
            StatusCode::CREATED,
            Json(json!({"id": "p-room", "kind": "exec", "state": "running"})),
        ));
    }
    authed_tb(
        &fake,
        &headers,
        &id,
        &format!(
            "v2 exec `{}`",
            command.split_whitespace().next().unwrap_or("")
        ),
    )?;
    assert!(
        body["timeoutMs"].is_u64() && q.contains_key("waitMs"),
        "both clocks are sent"
    );
    let out = match command.as_str() {
        "uname -m" => "aarch64\n",
        "pwd" => "/home/daytona\n",
        c if c.starts_with("chmod +x /home/daytona/duet-room") => "",
        other => panic!("unexpected command: {other}"),
    };
    Ok((
        StatusCode::CREATED,
        Json(
            json!({"id": "p1", "kind": "exec", "state": "terminal", "reason": "exited", "exitCode": 0,
        "logs": {"frames": [{"channel": "stderr", "data": "noise "}, {"channel": "stdout", "data": out}], "nextCursor": "c1", "eof": true, "truncatedHead": false}}),
        ),
    ))
}

async fn v2_upload(
    State(fake): State<Fake>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Query(q): Query<HashMap<String, String>>,
    mut form: Multipart,
) -> Result<Json<Value>, (StatusCode, Json<Value>)> {
    authed_tb(&fake, &headers, &id, "v2 upload")?;
    let path = q.get("path").cloned().unwrap_or_default();
    assert!(path.starts_with('/'), "the new API wants an absolute path");
    let field = form.next_field().await.unwrap().expect("one part");
    assert_eq!(field.name(), Some("file"));
    let name = field.file_name().unwrap_or("").to_string();
    let size = field.bytes().await.unwrap().len();
    fake.uploads
        .lock()
        .unwrap()
        .push((path.clone(), name, size));
    Ok(Json(json!({"path": path, "bytes": size})))
}

async fn fake_daytona() -> (Fake, String) {
    let fake = Fake::default();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    fake.base.lock().unwrap().clone_from(&base);
    let app = Router::new()
        .route("/api/sandbox", post(create).get(list))
        .route("/api/sandbox/{id}", get(get_one).delete(delete))
        .route(
            "/api/sandbox/{id}/ports/{port}/signed-preview-url",
            get(signed),
        )
        .route("/toolbox/{id}/process/execute", post(execute))
        .route("/toolbox/{id}/files/bulk-upload", post(upload))
        .route("/toolbox/{id}/process/session", post(session))
        .route("/api-keys/current", get(v2_whoami))
        .route("/sandboxes", post(v2_create))
        .route("/sandboxes/search", get(v2_search))
        .route("/sandboxes/{id}", get(v2_get).delete(v2_delete))
        .route("/sandboxes/{id}/access-token", post(v2_mint))
        .route(
            "/sandboxes/{id}/ports/{port}",
            axum::routing::put(v2_expose),
        )
        .route(
            "/sandboxes/{id}/ports/{port}/signed-preview-urls",
            post(v2_signed),
        )
        .route("/tb2/sandboxes/{id}/processes", post(v2_process))
        .route("/tb2/sandboxes/{id}/files/upload", post(v2_upload))
        .route(
            "/toolbox/{id}/process/session/{sid}/exec",
            post(session_exec),
        )
        .with_state(fake.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (fake, base)
}

// ---- a stand-in for the model --------------------------------------------------------------

#[derive(Clone, Copy)]
enum Model {
    Good,
    Garbage,
    Slow,
}

async fn fake_nosana(mode: Model) -> (String, Arc<AtomicU32>) {
    let calls = Arc::new(AtomicU32::new(0));
    let counted = calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let t = |en: &str, ja: &str| json!({"en": en, "ja": ja});
    let good = json!({
        "activity": "sketch",
        "why": t("One of you talks and one of you listens, and you both love music.", "一人は話し好き、一人は聞き上手。二人とも音楽が好き。"),
        "opener": t("Draw the last song you heard.", "最後に聴いた曲を絵にしてみて。"),
        "nudges": [t("What do you listen to on the train?", "電車では何を聴きますか？"), t("First concert?", "初めてのライブは？"), t("Headphones or speakers?", "ヘッドホン派？スピーカー派？")],
        "last_minute": t("Sign it together.", "二人でサインを。"),
        "prompts": [t("A song as weather", "曲を天気で表すと"), t("A concert for cats", "猫のためのコンサート"), t("Your street at night", "夜の自分の通り"), t("A shared umbrella", "相合傘")],
    });
    let app = Router::new()
        .route("/v1/models", get(|| async { Json(json!({"data": [{"id": "not-served", "available": false}, {"id": "fake/model-27b", "available": true}]})) }))
        .route(
            "/v1/chat/completions",
            post(move |Json(body): Json<Value>| {
                let (counted, good) = (counted.clone(), good.clone());
                async move {
                    counted.fetch_add(1, Ordering::Relaxed);
                    assert_eq!(body["model"], "fake/model-27b", "the model that is being served is the one called");
                    assert!(!body["messages"][1]["content"].as_str().unwrap().contains("Ana"), "names never reach the model");
                    // Three kinds of question: read a note, plan a pair, write the note for round two.
                    let system = body["messages"][0]["content"].as_str().unwrap_or("");
                    let good = if system.contains("Turn it into their answers") {
                        assert_eq!(body["temperature"], 0.0, "a note is read, not written");
                        json!({"langs": ["en"], "mood": "tired", "mode": "make", "style": null, "talk": "listener", "interests": ["music", "pets", "astrology"], "smile": "my cat learned to open doors"})
                    } else if system.contains("Write the short note") {
                        json!({"en": "You made something together. See you again soon!", "ja": "一緒に何かを作りました。また会いましょう！"})
                    } else {
                        good
                    };
                    let content = match mode {
                        Model::Good => format!("<think>They both like music…</think>\nHere you go:\n```json\n{good}\n```"),
                        Model::Garbage => "I'm sorry, I can't help with that.".to_string(),
                        Model::Slow => {
                            tokio::time::sleep(Duration::from_secs(4)).await;
                            good.to_string()
                        }
                    };
                    Json(json!({"choices": [{"message": {"role": "assistant", "content": content}}]}))
                }
            }),
        );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (url, calls)
}

// ---- the lobby under test ------------------------------------------------------------------

struct Lobby {
    app: Arc<App>,
    port: u16,
    dir: PathBuf,
}

async fn lobby(
    tune: impl FnOnce(&mut Settings),
    daytona: Option<&str>,
    model: Option<&str>,
) -> Lobby {
    let dir = std::env::temp_dir().join(format!("duet-test-{}", duet_room::text::token(9)));
    std::fs::create_dir_all(dir.join("bin")).unwrap();
    for name in ["duet-room-x86_64", "duet-room-aarch64"] {
        std::fs::write(dir.join("bin").join(name), b"\x7fELF pretend room program").unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = Settings::quick(dir.clone());
    cfg.public_url = format!("http://127.0.0.1:{port}");
    cfg.session_ms = 30_000;
    cfg.vote_wait_ms = 20_000;
    cfg.join_wait_ms = 20_000;
    cfg.rematch_after_ms = 3_600_000;
    tune(&mut cfg);
    let rooms = LocalRooms::default();
    let local = Host::Local {
        rooms: rooms.clone(),
        public_url: cfg.public_url.clone(),
    };
    let remote = daytona.map(|base| {
        let (base, v2) = base
            .strip_suffix("#v2")
            .map_or((base, false), |b| (b, true));
        let settings = DaytonaSettings {
            api_url: format!("{base}/api"),
            api2_url: base.to_string(),
            api: if v2 {
                duet::settings::DaytonaApi::V2
            } else {
                duet::settings::DaytonaApi::V0
            },
            key: "test-key".into(),
            target: None,
            snapshot: None,
        };
        Host::Daytona(Box::new(DaytonaHost::new(
            &settings,
            dir.join("bin"),
            "testlobby".into(),
        )))
    });
    let llm = model.map(|url| Llm::new(url, None, Some("nos_test".into())));
    let app = App::with(cfg, rooms, local, remote, llm);
    app.run();
    let router = duet::web::router(app.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Lobby { app, port, dir }
}

fn answers(talk: &str, mode: &str, interests: &[&str]) -> Answers {
    Answers {
        langs: vec!["en".into()],
        mood: "calm".into(),
        mode: mode.into(),
        style: "team".into(),
        talk: talk.into(),
        interests: interests.iter().map(ToString::to_string).collect(),
        smile: "my neighbour's dog".into(),
    }
}

impl Lobby {
    fn join(&self, name: &str, talk: &str) -> String {
        let secret = self.app.join(None, name, "en").unwrap();
        self.app
            .answer(&secret, answers(talk, "talk", &["music", "food"]))
            .unwrap();
        secret
    }

    /// What this person's page would show now. Asking also counts as being here.
    fn view(&self, secret: &str) -> Value {
        match self.app.poll(secret, 0) {
            Poll::Now(view) => view,
            _ => panic!("no view"),
        }
    }

    async fn until(&self, secret: &str, screen: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let view = self.view(secret);
            if view["screen"] == screen {
                return view;
            }
            assert!(
                Instant::now() < deadline,
                "still on {} instead of {screen}: {view}",
                view["screen"]
            );
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens the room as a browser would, from the link the lobby handed out. A room in a sandbox is
/// reached through the lobby with held requests (its link says `p=1`): that path is checked first,
/// then the test talks to the room over a socket on the sandbox itself.
async fn enter(lobby: &Lobby, link: &str) -> Socket {
    let (page, rest) = link
        .split_once("#k=")
        .expect("a room link carries its key after #");
    let key = rest.split('&').next().unwrap();
    let mut page = page.to_string();
    if rest.contains("p=1") {
        let reply = reqwest::get(format!("{page}hello?k={key}"))
            .await
            .expect("the lobby answers for the room");
        assert!(
            reply.status().is_success(),
            "the room greets through the lobby"
        );
        let hello: Value = reply.json().await.expect("a greeting");
        assert!(hello.is_object(), "{hello}");
        let room = page
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap()
            .to_string();
        let base = lobby
            .app
            .proxied
            .lock()
            .unwrap()
            .get(&room)
            .cloned()
            .expect("the lobby passes this room on");
        page = format!("{base}/");
    }
    let ws = format!("{}ws?k={key}", page.replacen("http", "ws", 1));
    let (mut socket, _) = tokio_tungstenite::connect_async(ws)
        .await
        .expect("the room link opens");
    assert_eq!(read(&mut socket, "hello").await["t"], "hello");
    socket
}

async fn read(socket: &mut Socket, kind: &str) -> Value {
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(6), socket.next())
            .await
            .unwrap_or_else(|_| panic!("no {kind} in 6 s"))
            .expect("open")
            .unwrap();
        if let Message::Text(text) = msg {
            let v: Value = serde_json::from_str(text.as_str()).unwrap();
            if v["t"] == kind {
                return v;
            }
        }
    }
}

async fn say(socket: &mut Socket, v: Value) {
    socket
        .send(Message::Text(v.to_string().into()))
        .await
        .unwrap();
}

// ---- the tests -----------------------------------------------------------------------------

#[tokio::test]
async fn a_session_in_a_sandbox_planned_by_the_model() {
    let (fake, base) = fake_daytona().await;
    let (model, calls) = fake_nosana(Model::Good).await;
    let lobby = lobby(|_| {}, Some(&base), Some(&model)).await;
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));

    let a = lobby.until(&ana, "room").await;
    let b = lobby.until(&aoi, "room").await;
    assert_eq!(a["runtime"]["kind"], "daytona");
    assert_eq!(
        a["activity"]["kind"], "sketch",
        "the model's choice was used"
    );
    assert!(
        a["why"]["ja"].as_str().unwrap().contains("音楽"),
        "and its words, in both languages"
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1, "one call per pair");
    let (link_a, link_b) = (a["url"].as_str().unwrap(), b["url"].as_str().unwrap());
    assert!(
        link_a.contains(&format!(":{}/r/", lobby.port)) && link_a.ends_with("&p=1"),
        "the browser stays on the lobby's address, which passes the room on: {link_a}"
    );
    assert_ne!(
        link_a.split("#k=").nth(1),
        link_b.split("#k=").nth(1),
        "each person has their own key"
    );

    // What was asked of Daytona, in order.
    let calls_made = fake.log.lock().unwrap().clone();
    let expected = [
        "POST /sandbox",
        "GET /sandbox/:id",
        "GET /sandbox/:id",
        "POST execute `uname`",
        "POST execute `pwd`",
        "POST bulk-upload",
        "POST execute `chmod`",
        "POST session",
        "POST session exec",
        "GET signed-preview-url",
    ];
    assert_eq!(calls_made, expected);
    let created = fake
        .boxes
        .lock()
        .unwrap()
        .values()
        .next()
        .unwrap()
        .body
        .clone();
    assert!(
        created["ttlMinutes"].as_u64().unwrap() >= 2,
        "Daytona itself is told when to destroy it"
    );
    assert_eq!(created["autoStopInterval"], 0);
    assert_eq!(created["autoDeleteInterval"], 0);
    assert_eq!(created["labels"]["app"], "duet");
    assert_eq!(created["public"], false);
    let uploads = fake.uploads.lock().unwrap().clone();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].0, "/home/daytona/duet-room");
    assert!(uploads[0].2 > 0);

    // The room is a real one.
    let (mut sa, mut sb) = (enter(&lobby, link_a).await, enter(&lobby, link_b).await);
    // Through the lobby, a held request and a send both reach the room.
    let (page, rest) = link_a.split_once("#k=").unwrap();
    let key = rest.split('&').next().unwrap();
    let poll = reqwest::get(format!("{page}poll?k={key}&after=0&hold=0"))
        .await
        .unwrap();
    assert!(
        poll.status().is_success(),
        "a held request passes through: {}",
        poll.status()
    );
    let sent = reqwest::Client::new()
        .post(format!("{page}send?k={key}"))
        .header("content-type", "application/json")
        .body(r#"[{"t":"ping","c":1}]"#)
        .send()
        .await
        .unwrap();
    assert!(
        sent.status().is_success(),
        "a send passes through: {}",
        sent.status()
    );
    say(
        &mut sa,
        json!({"t": "chat", "mid": "a1", "text": "hello from the sandbox"}),
    )
    .await;
    assert_eq!(
        read(&mut sb, "chat").await["text"],
        "hello from the sandbox"
    );
    say(
        &mut sb,
        json!({"t": "score", "d": {"kind": "sketch", "strokes": 7}}),
    )
    .await;
    say(&mut sb, json!({"t": "leave"})).await;
    read(&mut sa, "phase").await;
    lobby.app.back(&ana);

    // The bell: the sandbox is deleted, checked, and both are asked.
    let vote = lobby.until(&ana, "vote").await;
    lobby.until(&aoi, "vote").await;
    assert_eq!(vote["receipt"]["kind"], "daytona");
    assert_eq!(vote["receipt"]["gone"], true);
    // After the bell the passage is gone: the page gets 404, which it reads as "the room is over,
    // go back", instead of trying again for ever.
    assert!(
        lobby.app.proxied.lock().unwrap().is_empty(),
        "the bell removes the passage"
    );
    let after = reqwest::get(format!("{page}poll?k={key}&after=0&hold=0"))
        .await
        .unwrap();
    assert_eq!(after.status().as_u16(), 404);
    assert!(fake.boxes.lock().unwrap().is_empty(), "no sandbox is left");
    let after = fake.log.lock().unwrap()[expected.len()..].to_vec();
    assert!(
        after.contains(&"DELETE /sandbox/:id".to_string())
            && after.last().unwrap() == "GET /sandbox/:id",
        "{after:?}"
    );
    assert!(
        tokio_tungstenite::connect_async(
            link_a
                .split_once("#k=")
                .map(|(p, k)| format!("{}ws?k={k}", p.replacen("http", "ws", 1)))
                .unwrap()
        )
        .await
        .is_err(),
        "the old link is dead"
    );

    // Both say yes: a second round is set, and only what was done is kept.
    lobby.app.vote(&ana, true, true).unwrap();
    assert_eq!(lobby.view(&ana)["voted"], true);
    lobby.app.vote(&aoi, true, true).unwrap();
    let result = lobby.until(&ana, "result").await;
    assert_eq!(result["mutual"], true);
    assert_eq!(result["partner"]["name"], "Aoi");
    assert_eq!(result["reservation"]["with"], "Aoi");
    lobby.app.save();
    let saved = std::fs::read_to_string(lobby.dir.join("state.json")).unwrap();
    assert!(
        saved.contains("\"strokes\":7"),
        "the activity's own record is kept"
    );
    assert!(
        !saved.contains("hello from the sandbox"),
        "what was said is not"
    );
    assert!(!lobby.dir.join("reports").exists());
    let hood = lobby.app.hood();
    assert_eq!(hood["plans"]["model"], 1);
    assert_eq!(hood["totals"]["mutual"], 1);
    assert!(
        !hood.to_string().contains("Aoi"),
        "the projector page never shows names"
    );
}

#[tokio::test]
async fn when_daytona_refuses_the_pair_still_gets_a_room() {
    let (fake, base) = fake_daytona().await;
    fake.fail_create.store(true, Ordering::Relaxed);
    let lobby = lobby(|_| {}, Some(&base), None).await;
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let a = lobby.until(&ana, "room").await;
    assert_eq!(
        a["runtime"]["kind"], "local",
        "and it is labelled as local, not as a sponsor run"
    );
    assert!(
        a["url"]
            .as_str()
            .unwrap()
            .contains(&format!(":{}/r/", lobby.port))
    );
    assert_eq!(lobby.app.hood()["plans"]["local_fallbacks"], 1);
    let b = lobby.until(&aoi, "room").await;
    let (mut sa, mut sb) = (
        enter(&lobby, a["url"].as_str().unwrap()).await,
        enter(&lobby, b["url"].as_str().unwrap()).await,
    );
    say(
        &mut sa,
        json!({"t": "chat", "mid": "a1", "text": "still works"}),
    )
    .await;
    assert_eq!(read(&mut sb, "chat").await["text"], "still works");
}

#[tokio::test]
async fn a_model_that_talks_nonsense_or_takes_too_long_changes_nothing() {
    for mode in [Model::Garbage, Model::Slow] {
        let (model, calls) = fake_nosana(mode).await;
        let lobby = lobby(|cfg| cfg.plan_budget_ms = 600, None, Some(&model)).await;
        let started = Instant::now();
        let (ana, _aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
        let a = lobby.until(&ana, "room").await;
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "the pair does not wait for a slow model"
        );
        assert!(
            ["wavelength", "thisorthat"].contains(&a["activity"]["kind"].as_str().unwrap_or("")),
            "the rules chose a talking activity, as if no model existed: {}",
            a["activity"]["kind"]
        );
        assert_eq!(a["why"]["en"], "You both picked music and food.");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        let hood = lobby.app.hood();
        assert_eq!(hood["plans"]["model"], 0);
        assert_eq!(hood["plans"]["rules"], 1);
    }
}

#[tokio::test]
async fn a_report_keeps_the_chat_and_closes_the_pair_to_each_other() {
    let lobby = lobby(|_| {}, None, None).await;
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let (a, b) = (
        lobby.until(&ana, "room").await,
        lobby.until(&aoi, "room").await,
    );
    let (mut sa, mut sb) = (
        enter(&lobby, a["url"].as_str().unwrap()).await,
        enter(&lobby, b["url"].as_str().unwrap()).await,
    );
    say(
        &mut sb,
        json!({"t": "chat", "mid": "b1", "text": "something unkind"}),
    )
    .await;
    read(&mut sa, "chat").await;
    say(
        &mut sa,
        json!({"t": "report", "reason": "rude", "note": "see the chat"}),
    )
    .await;
    read(&mut sa, "reported").await;
    lobby.app.back(&ana);
    let vote = lobby.until(&aoi, "vote").await;
    assert_eq!(
        vote["over"]["why"], "ended",
        "the reported person is only told it ended"
    );
    assert!(!vote.to_string().contains("report"));

    let reports: Vec<_> = std::fs::read_dir(lobby.dir.join("reports"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(reports.len(), 1);
    let kept: Value = serde_json::from_slice(&std::fs::read(reports[0].path()).unwrap()).unwrap();
    assert_eq!(kept["room"]["chat"][0]["text"], "something unkind");
    assert_eq!(kept["room"]["report"]["reason"], "rude");
    assert_eq!(lobby.app.hood()["totals"]["reports"], 1);

    // Even if both now say yes, these two are never paired again.
    lobby.app.vote(&ana, false, true).unwrap();
    lobby.app.vote(&aoi, true, true).unwrap();
    lobby.until(&ana, "result").await;
    lobby.app.queue(&ana, true).unwrap();
    lobby.app.queue(&aoi, true).unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(lobby.view(&ana)["screen"], "wait");
    assert_eq!(lobby.view(&aoi)["screen"], "wait");
}

#[tokio::test]
async fn someone_who_never_arrives_does_not_cost_the_other_their_place() {
    let lobby = lobby(|cfg| cfg.join_wait_ms = 900, None, None).await;
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let a = lobby.until(&ana, "room").await;
    lobby.until(&aoi, "room").await;
    let _sa = enter(&lobby, a["url"].as_str().unwrap()).await; // only Ana walks in
    let back = lobby.until(&ana, "wait").await;
    assert_eq!(back["note"], "noshow");
    let missed = lobby.until(&aoi, "home").await;
    assert_eq!(missed["note"], "missed");
    // Ana is paired with the next person to come, not with Aoi again.
    let ben = lobby.join("Ben", "listener");
    let next = lobby.until(&ana, "room").await;
    assert_eq!(next["partner"]["name"], "Ben");
    lobby.until(&ben, "room").await;
}

#[tokio::test]
async fn an_odd_one_out_waits_and_goes_first_next_time() {
    let lobby = lobby(|_| {}, None, None).await;
    let people: Vec<String> = ["Ana", "Aoi", "Ben"]
        .iter()
        .map(|n| lobby.join(n, "both"))
        .collect();
    let mut waiting = None;
    for _ in 0..100 {
        let screens: Vec<String> = people
            .iter()
            .map(|p| lobby.view(p)["screen"].as_str().unwrap().to_string())
            .collect();
        if screens.iter().filter(|s| *s == "room").count() == 2 {
            waiting = screens.iter().position(|s| s == "wait");
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let odd = &people[waiting.expect("two are in a room and one waits")];
    assert_eq!(lobby.view(odd)["here"], 1);
    let dee = lobby.join("Dee", "both");
    lobby.until(odd, "room").await;
    lobby.until(&dee, "room").await;
}

#[tokio::test]
async fn a_restart_keeps_people_and_plans_and_nothing_else() {
    let lobby = lobby(|_| {}, None, None).await;
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let (a, b) = (
        lobby.until(&ana, "room").await,
        lobby.until(&aoi, "room").await,
    );
    let (mut sa, _sb) = (
        enter(&lobby, a["url"].as_str().unwrap()).await,
        enter(&lobby, b["url"].as_str().unwrap()).await,
    );
    say(&mut sa, json!({"t": "leave"})).await;
    lobby.app.back(&ana);
    lobby.until(&ana, "vote").await;
    lobby.app.vote(&ana, true, true).unwrap();
    lobby.app.vote(&aoi, false, true).unwrap();
    lobby.until(&ana, "result").await;
    lobby.app.save();

    let mut cfg = lobby.app.cfg.clone();
    cfg.public_url = "http://127.0.0.1:1".into();
    let rooms = LocalRooms::default();
    let again = App::with(
        cfg.clone(),
        rooms.clone(),
        Host::Local {
            rooms,
            public_url: cfg.public_url.clone(),
        },
        None,
        None,
    );
    let view = again
        .view_of(&ana)
        .expect("Ana is still known by her secret");
    assert_eq!(view["screen"], "home");
    assert_eq!(view["me"]["name"], "Ana");
    assert_eq!(view["reservation"]["with"], "Aoi");
    assert!(
        again
            .calendar(&ana)
            .unwrap()
            .contains("SUMMARY:Duet with Aoi")
    );
    assert_eq!(again.hood()["totals"]["sessions"], 1);
    // Forgetting removes the person and the plan that involved them.
    again.forget(&aoi).unwrap();
    assert!(again.view_of(&aoi).is_none());
    assert!(again.view_of(&ana).unwrap()["reservation"].is_null());
}

#[tokio::test]
async fn leftover_sandboxes_are_swept_but_only_this_lobbys() {
    let (fake, base) = fake_daytona().await;
    let settings = DaytonaSettings {
        api_url: format!("{base}/api"),
        key: "test-key".into(),
        target: None,
        snapshot: None,
        ..Default::default()
    };
    let host = DaytonaHost::new(&settings, PathBuf::from("."), "testlobby".into());
    host.api
        .create(
            &[("app", "duet"), ("lobby", "testlobby"), ("room", "r_old")],
            5,
            None,
        )
        .await
        .unwrap();
    host.api
        .create(&[("app", "duet"), ("lobby", "another-lobby")], 5, None)
        .await
        .unwrap();
    host.api
        .create(&[("app", "something-else")], 5, None)
        .await
        .unwrap();
    assert_eq!(host.sweep().await.unwrap(), 1);
    assert_eq!(
        fake.boxes.lock().unwrap().len(),
        2,
        "other lobbies' rooms and other apps are left alone"
    );
    // A wrong key is reported as what it is.
    let wrong = DaytonaHost::new(
        &DaytonaSettings {
            key: "nope".into(),
            ..settings
        },
        PathBuf::from("."),
        "testlobby".into(),
    );
    let err = wrong.sweep().await.unwrap_err();
    assert!(err.contains("401") && err.contains("Unauthorized"), "{err}");
}

// ---- Nosana's other two jobs, and rooms built ahead ----------------------------------------

#[tokio::test]
async fn own_words_become_taps_and_are_not_kept() {
    let (model, calls) = fake_nosana(Model::Good).await;
    let lobby = lobby(|_| {}, None, Some(&model)).await;
    let secret = lobby.app.join(None, "Ana", "en").unwrap();
    let note =
        "Long day, a bit tired. I would love to draw something together. Into music and my cat.";
    let read = lobby.app.understand(&secret, note).await.unwrap();
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    let a = &read["answers"];
    assert_eq!(a["mood"], "tired");
    assert_eq!(a["mode"], "make");
    assert_eq!(
        a["style"], "team",
        "what the model left open, the rules filled from \"together\""
    );
    assert_eq!(a["talk"], "listener");
    assert_eq!(
        a["interests"],
        json!(["music", "pets"]),
        "an interest that is not on the list is dropped"
    );
    assert_eq!(a["smile"], "my cat learned to open doors");
    assert_eq!(read["missing"], json!([]));
    // The person still sends the taps themselves; until then nothing about the note is stored.
    assert_eq!(lobby.view(&secret)["screen"], "ask");
    lobby.app.save();
    let saved = std::fs::read_to_string(lobby.dir.join("state.json")).unwrap();
    assert!(!saved.contains("Long day"), "the note itself is never kept");
    assert_eq!(lobby.app.hood()["ai"]["reads"]["model"], 1);
    assert!(
        lobby.app.understand(&secret, "   ").await.is_err(),
        "an empty note is refused"
    );
}

#[tokio::test]
async fn own_words_are_read_by_rules_without_a_model_or_when_it_fails() {
    let without = lobby(|_| {}, None, None).await;
    let secret = without.app.join(None, "Aoi", "ja").unwrap();
    let read = without
        .app
        .understand(
            &secret,
            "今日はのんびり。ゲームで勝負したい！アニメと猫が好き。",
        )
        .await
        .unwrap();
    assert_eq!(read["answers"]["langs"], json!(["ja"]));
    assert_eq!(read["answers"]["mode"], "play");
    assert_eq!(read["answers"]["style"], "rival");
    assert_eq!(
        read["missing"],
        json!(["talk"]),
        "what nobody could read is asked"
    );
    assert_eq!(without.app.hood()["ai"]["reads"]["rules"], 1);

    let (model, _) = fake_nosana(Model::Garbage).await;
    let confused = lobby(|_| {}, None, Some(&model)).await;
    let secret = confused.app.join(None, "Aoi", "en").unwrap();
    let read = confused
        .app
        .understand(&secret, "I just want to talk, I am a good listener")
        .await
        .unwrap();
    assert_eq!(read["answers"]["mode"], "talk");
    assert_eq!(read["answers"]["talk"], "listener");
}

async fn both_say_yes(lobby: &Lobby) -> (String, String) {
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let (a, b) = (
        lobby.until(&ana, "room").await,
        lobby.until(&aoi, "room").await,
    );
    let (mut sa, mut sb) = (
        enter(lobby, a["url"].as_str().unwrap()).await,
        enter(lobby, b["url"].as_str().unwrap()).await,
    );
    say(
        &mut sa,
        json!({"t": "score", "d": {"kind": "sketch", "strokes": 41}}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    say(&mut sa, json!({"t": "leave"})).await;
    say(&mut sb, json!({"t": "leave"})).await;
    lobby.app.back(&ana);
    lobby.until(&ana, "vote").await;
    lobby.app.vote(&ana, true, true).unwrap();
    lobby.app.vote(&aoi, true, true).unwrap();
    lobby.until(&ana, "result").await;
    (ana, aoi)
}

#[tokio::test]
async fn both_saying_yes_brings_a_note_written_from_the_record() {
    let (model, _) = fake_nosana(Model::Good).await;
    let lobby = lobby(|_| {}, None, Some(&model)).await;
    let (ana, aoi) = both_say_yes(&lobby).await;
    // The rules' note shows at once; the model's replaces it when it holds up.
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let note = lobby.view(&ana)["reservation"]["note"].clone();
        if note["en"] == "You made something together. See you again soon!" {
            assert_eq!(
                lobby.view(&aoi)["reservation"]["note"],
                note,
                "both see the same note"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the model's note never came: {note}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(lobby.app.hood()["ai"]["notes"]["model"], 1);
}

#[tokio::test]
async fn without_a_model_the_note_comes_from_the_rules() {
    let lobby = lobby(|_| {}, None, None).await;
    let (ana, _) = both_say_yes(&lobby).await;
    let note = lobby.view(&ana)["reservation"]["note"].clone();
    let en = note["en"].as_str().unwrap_or_default().to_string();
    assert!(
        !en.is_empty() && note["ja"].as_str().is_some_and(|j| !j.is_empty()),
        "{note}"
    );
    assert!(en.contains("Next time"), "{en}");
}

/// Waits until the hood shows `n` rooms built ahead and ready.
async fn ready_ahead(lobby: &Lobby, n: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        let warm = lobby.app.hood()["warm"].clone();
        let ready = warm["rooms"]
            .as_array()
            .map_or(0, |r| r.iter().filter(|r| r["state"] == "ready").count());
        if ready == n {
            return warm;
        }
        assert!(
            Instant::now() < deadline,
            "{ready} of {n} rooms ready ahead: {warm}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn rooms_are_built_ahead_all_at_once_and_a_pair_gets_one() {
    let (fake, base) = fake_daytona().await;
    let lobby = lobby(|c| c.warm_rooms = 2, Some(&base), None).await;
    lobby.app.keep_warm();
    let warm = ready_ahead(&lobby, 2).await;
    assert_eq!(warm["batch"]["n"], 2, "both were built in one batch");
    assert!(warm["batch"]["done_ms"].is_u64());
    for room in warm["rooms"].as_array().unwrap() {
        let steps: Vec<&str> = room["marks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["k"].as_str().unwrap())
            .collect();
        assert_eq!(steps, ["create", "move", "start"], "every step is timed");
    }
    let created = |fake: &Fake| {
        fake.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| *l == "POST /sandbox")
            .count()
    };
    assert_eq!(created(&fake), 2);

    let (ana, _aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let a = lobby.until(&ana, "room").await;
    assert_eq!(a["runtime"]["kind"], "daytona");
    let hood = lobby.app.hood();
    assert_eq!(
        hood["rooms"][0]["warm"], true,
        "the pair got a room that was already built"
    );
    assert_eq!(hood["warm"]["taken"], 1);
    // The one that was taken is replaced.
    ready_ahead(&lobby, 2).await;
    assert_eq!(created(&fake), 3);
    assert!(hood["timings"]["steps"]["create"]["n"].as_u64().unwrap() >= 2);
}

#[tokio::test]
async fn the_admin_can_rebuild_rooms_ahead_but_only_with_the_key() {
    let (fake, base) = fake_daytona().await;
    let lobby = lobby(|c| c.warm_rooms = 1, Some(&base), None).await;
    lobby.app.keep_warm();
    ready_ahead(&lobby, 1).await;
    let http = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/api/hood/warm", lobby.port);
    let refused = http
        .post(&url)
        .header("x-duet-admin", "guess")
        .json(&json!({"n": 3}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 403);
    let ok = http
        .post(&url)
        .header("x-duet-admin", "test-admin")
        .json(&json!({"n": 3}))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
    let warm = ready_ahead(&lobby, 3).await;
    assert_eq!(warm["batch"]["n"], 3, "three new ones, built in one batch");
    // The old one was removed rather than left to its lifetime.
    let deadline = Instant::now() + Duration::from_secs(5);
    while fake.boxes.lock().unwrap().len() != 3 {
        assert!(Instant::now() < deadline, "the old room was not removed");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    lobby.app.close_warm().await;
    assert_eq!(
        fake.boxes.lock().unwrap().len(),
        0,
        "stopping removes them all"
    );
}

#[tokio::test]
async fn without_daytona_nothing_is_built_ahead() {
    let lobby = lobby(|c| c.warm_rooms = 3, None, None).await;
    lobby.app.keep_warm();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(lobby.app.hood()["warm"]["on"], false);
    assert!(lobby.app.rebuild_warm(2).is_err());
}

// ---- the new Daytona API (SDK 2.0) ---------------------------------------------------------

#[tokio::test]
async fn a_session_in_a_sandbox_on_the_new_daytona_api() {
    let (fake, base) = fake_daytona().await;
    let lobby = lobby(|_| {}, Some(&format!("{base}#v2")), None).await;
    let (ana, aoi) = (lobby.join("Ana", "talker"), lobby.join("Aoi", "listener"));
    let a = lobby.until(&ana, "room").await;
    let b = lobby.until(&aoi, "room").await;
    assert_eq!(a["runtime"]["kind"], "daytona");
    assert!(a["runtime"]["id"].as_str().unwrap().starts_with("sbx_"));
    let calls_made = fake.log.lock().unwrap().clone();
    let expected = [
        "v2 POST /sandboxes",
        "v2 GET /sandboxes/:id",
        "v2 GET /sandboxes/:id",
        "v2 exec `uname`",
        "v2 exec `pwd`",
        "v2 upload",
        "v2 exec `chmod`",
        "v2 start room",
        "v2 PUT port",
        "v2 POST signed-preview-urls",
    ];
    assert_eq!(calls_made, expected);
    assert_eq!(fake.uploads.lock().unwrap()[0].0, "/home/daytona/duet-room");
    // The room works through the links, exactly as with the old API.
    let (mut sa, mut sb) = (
        enter(&lobby, a["url"].as_str().unwrap()).await,
        enter(&lobby, b["url"].as_str().unwrap()).await,
    );
    say(
        &mut sa,
        json!({"t": "chat", "mid": "n1", "text": "hello from the new API"}),
    )
    .await;
    assert_eq!(
        read(&mut sb, "chat").await["text"],
        "hello from the new API"
    );
    say(&mut sa, json!({"t": "leave"})).await;
    lobby.app.back(&ana);
    let vote = lobby.until(&ana, "vote").await;
    assert!(
        fake.boxes.lock().unwrap().is_empty(),
        "the sandbox is deleted at the bell"
    );
    assert_eq!(
        vote["receipt"]["gone"], true,
        "and checked to be gone: {}",
        vote["receipt"]
    );
}

#[tokio::test]
async fn auto_speaks_the_new_api_when_it_knows_the_key_and_the_old_one_otherwise() {
    let (fake, base) = fake_daytona().await;
    let settings = DaytonaSettings {
        api_url: format!("{base}/api"),
        api2_url: base.clone(),
        key: "test-key".into(),
        ..Default::default()
    };
    let host = DaytonaHost::new(&settings, PathBuf::from("."), "testlobby".into());
    assert!(host.api.pick().await.contains("new API"));
    assert!(host.api.is_v2());
    // Clean-up works on the new API too, and only on this lobby's rooms.
    host.api
        .create(&[("app", "duet"), ("lobby", "testlobby")], 5, None)
        .await
        .unwrap();
    host.api
        .create(&[("app", "duet"), ("lobby", "another")], 5, None)
        .await
        .unwrap();
    assert_eq!(host.sweep().await.unwrap(), 1);
    assert_eq!(fake.boxes.lock().unwrap().len(), 1);

    fake.v2_refuses.store(true, Ordering::Relaxed);
    let old = DaytonaHost::new(&settings, PathBuf::from("."), "testlobby".into());
    let said = old.api.pick().await;
    assert!(
        said.contains("original API") && said.contains("unknown key"),
        "{said}"
    );
    assert!(!old.api.is_v2());
}

#[tokio::test]
async fn when_the_new_api_may_not_make_sandboxes_auto_uses_the_original_one() {
    let (fake, base) = fake_daytona().await;
    fake.v2_no_linux.store(true, Ordering::Relaxed);
    let settings = DaytonaSettings {
        api_url: format!("{base}/api"),
        api2_url: base.clone(),
        key: "test-key".into(),
        ..Default::default()
    };
    let host = DaytonaHost::new(&settings, PathBuf::from("."), "testlobby".into());
    assert!(host.api.pick().await.contains("new API"));
    let sb = host
        .api
        .create(&[("app", "duet"), ("lobby", "testlobby")], 5, None)
        .await
        .unwrap();
    assert!(!host.api.is_v2(), "it switched to the original API");
    assert!(
        sb.id.starts_with("sbx-"),
        "and made the sandbox there: {}",
        sb.id
    );
    let log = fake.log.lock().unwrap().clone();
    assert_eq!(
        log[log.len() - 2..],
        ["v2 POST /sandboxes", "POST /sandbox"]
    );
}
