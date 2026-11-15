//! The lobby's doors: its two pages, the small API the page uses, and the path local rooms are
//! served on.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Json, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode, Uri, header};
use axum::response::IntoResponse;
use axum::routing::{any, get, post};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::lobby::{App, Poll};
use crate::profile::Answers;

include!(concat!(env!("OUT_DIR"), "/lobby.rs"));

const HOLD: Duration = Duration::from_secs(25);

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/hood", get(hood_page))
        .route("/fonts/{file}", get(font))
        .route("/img/{file}", get(image))
        .route("/healthz", get(|| async { "ok" }))
        .route("/me/{token}", get(me))
        .route("/api/state", get(state))
        .route("/api/join", post(join))
        .route("/api/answers", post(answers))
        .route("/api/understand", post(understand))
        .route("/api/queue", post(queue))
        .route("/api/vote", post(vote))
        .route("/api/back", post(back))
        .route("/api/lang", post(lang))
        .route("/api/forget", post(forget))
        .route("/api/hood", get(hood))
        .route("/api/hood/warm", post(hood_warm))
        .route("/api/calendar.ics", get(calendar))
        .route("/r/{id}", any(room))
        .route("/r/{id}/", any(room))
        .route("/r/{id}/{*rest}", any(room))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(app)
}

fn reply(status: StatusCode, body: Value) -> Response<Body> {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn refuse(why: &str) -> Response<Body> {
    let status = match why {
        "nobody" => StatusCode::UNAUTHORIZED,
        "busy" => StatusCode::CONFLICT,
        "full" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::BAD_REQUEST,
    };
    reply(status, json!({"error": why}))
}

/// The person's secret, from the cookie the lobby set.
fn secret_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|pair| pair.trim().strip_prefix("duet=").map(str::to_string))
        .filter(|t| {
            !t.is_empty()
                && t.len() <= 64
                && t.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
}

fn cookie(app: &App, secret: &str, max_age: u64) -> HeaderValue {
    let secure = if app.cfg.public_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    HeaderValue::from_str(&format!(
        "duet={secret}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax{secure}"
    ))
    .expect("a cookie is plain text")
}

fn hello(app: &App) -> Value {
    json!({"screen": "hello", "v": 0, "sponsors": app.sponsors(), "mins": app.cfg.session_ms as f64 / 60_000.0})
}

async fn page(headers: HeaderMap) -> Response<Body> {
    duet_room::assets::respond(ASSETS, "/index.html", &headers)
}

async fn hood_page(headers: HeaderMap) -> Response<Body> {
    duet_room::assets::respond(ASSETS, "/hood.html", &headers)
}

async fn image(Path(file): Path<String>, headers: HeaderMap) -> Response<Body> {
    duet_room::assets::respond(ASSETS, &format!("/img/{file}"), &headers)
}

async fn font(Path(file): Path<String>, headers: HeaderMap) -> Response<Body> {
    duet_room::assets::respond(ASSETS, &format!("/fonts/{file}"), &headers)
}

/// The private return link: sets the cookie and goes to the front page.
async fn me(State(app): State<Arc<App>>, Path(secret): Path<String>) -> Response<Body> {
    let mut out = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, "/")
        .header(header::CACHE_CONTROL, "no-store")
        .header("Referrer-Policy", "no-referrer");
    if app.knows(&secret) {
        out = out.header(header::SET_COOKIE, cookie(&app, &secret, 30 * 86_400));
    }
    out.body(Body::empty()).unwrap()
}

#[derive(Deserialize)]
struct Seen {
    #[serde(default)]
    v: u64,
}

/// Marks a held request as finished even when the page went away mid-wait.
struct Held<'a> {
    app: &'a App,
    secret: &'a str,
    done: bool,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if !self.done {
            self.app.poll_done(self.secret);
        }
    }
}

/// The page's one open request: answered at once if its screen changed, otherwise held.
async fn state(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Query(seen): Query<Seen>,
) -> Response<Body> {
    let Some(secret) = secret_of(&headers) else {
        return reply(StatusCode::OK, hello(&app));
    };
    match app.poll(&secret, seen.v) {
        Poll::Nobody => reply(StatusCode::OK, hello(&app)),
        Poll::Now(view) => reply(StatusCode::OK, view),
        Poll::Wait(mut changed) => {
            let mut held = Held {
                app: &app,
                secret: &secret,
                done: false,
            };
            let _ = tokio::time::timeout(HOLD, changed.changed()).await;
            held.done = true;
            reply(StatusCode::OK, app.poll_done(&secret))
        }
    }
}

#[derive(Deserialize)]
struct Join {
    name: String,
    #[serde(default)]
    lang: String,
    #[serde(default)]
    adult: bool,
}

async fn join(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Join>,
) -> Response<Body> {
    if !body.adult {
        return refuse("adult");
    }
    match app.join(secret_of(&headers).as_deref(), &body.name, &body.lang) {
        Ok(secret) => {
            let mut out = reply(
                StatusCode::OK,
                app.view_of(&secret).unwrap_or_else(|| hello(&app)),
            );
            out.headers_mut()
                .insert(header::SET_COOKIE, cookie(&app, &secret, 30 * 86_400));
            out
        }
        Err(why) => refuse(why),
    }
}

fn after(app: &App, secret: &str, done: Result<(), &'static str>) -> Response<Body> {
    match done {
        Ok(()) => reply(
            StatusCode::OK,
            app.view_of(secret).unwrap_or_else(|| hello(app)),
        ),
        Err(why) => refuse(why),
    }
}

async fn answers(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Answers>,
) -> Response<Body> {
    let Some(secret) = secret_of(&headers) else {
        return refuse("nobody");
    };
    after(&app, &secret, app.answer(&secret, body))
}

#[derive(Deserialize)]
struct Note {
    #[serde(default)]
    text: String,
}

/// "Say it in your own words": answers with the taps read from the note, for the page to show.
async fn understand(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Note>,
) -> Response<Body> {
    let Some(secret) = secret_of(&headers) else {
        return refuse("nobody");
    };
    match app.understand(&secret, &body.text).await {
        Ok(v) => reply(StatusCode::OK, v),
        Err(why) => refuse(why),
    }
}

#[derive(Deserialize)]
struct Queue {
    on: bool,
}

async fn queue(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Queue>,
) -> Response<Body> {
    let Some(secret) = secret_of(&headers) else {
        return refuse("nobody");
    };
    after(&app, &secret, app.queue(&secret, body.on))
}

#[derive(Deserialize)]
struct Ballot {
    liked: bool,
    again: bool,
}

async fn vote(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Ballot>,
) -> Response<Body> {
    let Some(secret) = secret_of(&headers) else {
        return refuse("nobody");
    };
    after(&app, &secret, app.vote(&secret, body.liked, body.again))
}

async fn back(State(app): State<Arc<App>>, headers: HeaderMap) -> Response<Body> {
    if let Some(secret) = secret_of(&headers) {
        app.back(&secret);
    }
    reply(StatusCode::OK, json!({"ok": true}))
}

#[derive(Deserialize)]
struct Lang {
    lang: String,
}

async fn lang(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<Lang>,
) -> Response<Body> {
    if let Some(secret) = secret_of(&headers) {
        app.set_lang(&secret, &body.lang);
    }
    reply(StatusCode::OK, json!({"ok": true}))
}

async fn forget(State(app): State<Arc<App>>, headers: HeaderMap) -> Response<Body> {
    let Some(secret) = secret_of(&headers) else {
        return refuse("nobody");
    };
    match app.forget(&secret) {
        Ok(()) => {
            let mut out = reply(StatusCode::OK, hello(&app));
            out.headers_mut()
                .insert(header::SET_COOKIE, cookie(&app, "", 0));
            out
        }
        Err(why) => refuse(why),
    }
}

#[derive(Deserialize)]
struct WarmAsk {
    n: usize,
}

/// The admin's "build N rooms at once". Needs the admin key, which only the lobby's own terminal
/// shows.
async fn hood_warm(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    Json(body): Json<WarmAsk>,
) -> Response<Body> {
    let given = headers
        .get("x-duet-admin")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let key = app.cfg.admin_key.as_bytes();
    let same = given.len() == key.len()
        && given
            .bytes()
            .zip(key)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if !same {
        return reply(StatusCode::FORBIDDEN, json!({"error": "admin"}));
    }
    match app.rebuild_warm(body.n) {
        Ok(()) => reply(StatusCode::OK, json!({"ok": true})),
        Err(why) => reply(StatusCode::CONFLICT, json!({"error": why})),
    }
}

async fn hood(State(app): State<Arc<App>>) -> Response<Body> {
    reply(StatusCode::OK, app.hood())
}

async fn calendar(State(app): State<Arc<App>>, headers: HeaderMap) -> Response<Body> {
    match secret_of(&headers).and_then(|s| app.calendar(&s)) {
        Some(ics) => Response::builder()
            .header(header::CONTENT_TYPE, "text/calendar; charset=utf-8")
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"duet.ics\"",
            )
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(ics))
            .unwrap(),
        None => refuse("nothing"),
    }
}

const GONE: &str = "<!doctype html><meta charset=utf-8><meta name=viewport content='width=device-width,initial-scale=1'><title>Duet</title><body style='margin:0;height:100vh;display:grid;place-items:center;background:#0c0914;color:#f4f0fb;font:17px system-ui;text-align:center'><div><p style='font-size:24px;font-weight:600'>This room is gone.</p><p style='color:#ada4c4'>A room lasts one session and is then destroyed.</p><p><a href='/' style='color:#ffb84d'>Back to Duet</a></p></div>";

/// A local room, served under `/r/<id>/`. The room's own router sees the path without that part.
async fn room(State(app): State<Arc<App>>, mut req: Request) -> Response<Body> {
    let path = req.uri().path().to_string();
    let after = path.strip_prefix("/r/").unwrap_or("");
    let (id, rest) = after.split_once('/').unwrap_or((after, ""));
    if !path.ends_with('/') && rest.is_empty() {
        // The page uses relative addresses, so its own address must end in a slash.
        return Response::builder()
            .status(StatusCode::PERMANENT_REDIRECT)
            .header(header::LOCATION, format!("/r/{id}/"))
            .body(Body::empty())
            .unwrap();
    }
    let found = app
        .rooms
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(id)
        .cloned();
    let Some(found) = found else {
        let base = app
            .proxied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(base) = base {
            return proxy(&base, rest, req).await;
        }
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CACHE_CONTROL, "no-store")
            .body(Body::from(GONE))
            .unwrap();
    };
    let inner = match req.uri().query() {
        Some(q) => format!("/{rest}?{q}"),
        None => format!("/{rest}"),
    };
    match inner.parse::<Uri>() {
        Ok(uri) => {
            *req.uri_mut() = uri;
            duet_room::http::serve_in(found, req).await
        }
        Err(_) => (StatusCode::BAD_REQUEST, "bad address").into_response(),
    }
}

/// A room in a Daytona sandbox, passed on to the sandbox with the header that skips Daytona's
/// preview warning, so people stay on the lobby's address. Sockets are not passed on: the page is
/// told to use held requests (`p=1` in its link), and a socket asked for anyway is refused at once,
/// which makes the page fall back by itself.
async fn proxy(base: &str, rest: &str, req: Request) -> Response<Body> {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    let client = CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .build()
            .expect("an http client")
    });
    if rest == "ws" || req.headers().contains_key(header::UPGRADE) {
        return (StatusCode::NOT_IMPLEMENTED, "this room uses held requests").into_response();
    }
    let url = match req.uri().query() {
        Some(q) => format!("{base}/{rest}?{q}"),
        None => format!("{base}/{rest}"),
    };
    let Ok(method) = reqwest::Method::from_bytes(req.method().as_str().as_bytes()) else {
        return (StatusCode::METHOD_NOT_ALLOWED, "no").into_response();
    };
    // Held requests wait up to 25 seconds in the room, so the lobby waits longer.
    let mut out = client
        .request(method, &url)
        .header("X-Daytona-Skip-Preview-Warning", "true")
        .timeout(Duration::from_secs(45));
    for name in [
        header::CONTENT_TYPE,
        header::ACCEPT,
        header::ACCEPT_ENCODING,
        header::ACCEPT_LANGUAGE,
    ] {
        if let Some(value) = req.headers().get(&name) {
            out = out.header(name.as_str(), value.as_bytes());
        }
    }
    let Ok(body) = axum::body::to_bytes(req.into_body(), 1 << 20).await else {
        return (StatusCode::PAYLOAD_TOO_LARGE, "too large").into_response();
    };
    if !body.is_empty() {
        out = out.body(body);
    }
    let Ok(reply) = out.send().await else {
        return (StatusCode::BAD_GATEWAY, "the room did not answer").into_response();
    };
    let mut res = Response::builder().status(reply.status().as_u16());
    for (name, value) in reply.headers() {
        let skip = matches!(
            name.as_str(),
            "connection"
                | "keep-alive"
                | "transfer-encoding"
                | "content-length"
                | "set-cookie"
                | "alt-svc"
                | "strict-transport-security"
        );
        if !skip {
            res = res.header(name.as_str(), value.as_bytes());
        }
    }
    let Ok(bytes) = reply.bytes().await else {
        return (StatusCode::BAD_GATEWAY, "the room's reply was cut off").into_response();
    };
    res.body(Body::from(bytes))
        .unwrap_or_else(|_| (StatusCode::BAD_GATEWAY, "a bad reply").into_response())
}
