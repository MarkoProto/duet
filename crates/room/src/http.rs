//! The room's doors: the page, the socket, held requests for pages that cannot keep a socket,
//! and three lobby-only endpoints.
//!
//! Every handler finds its room in the request's extensions, so the same router serves a room
//! running alone in a sandbox and many rooms inside the lobby.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{FromRequestParts, Path, Query};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Request, Response, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;
use tower::ServiceExt;

use crate::assets::room_asset;
use crate::state::{ClientMsg, Frame, Room};

const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const HOLD_MAX: Duration = Duration::from_secs(25);
const SOCKET_PING: Duration = Duration::from_secs(20);
const SOCKET_SILENCE: Duration = Duration::from_secs(50);

/// The room this request is for.
pub struct InRoom(pub Arc<Room>);

impl<S: Send + Sync> FromRequestParts<S> for InRoom {
    type Rejection = (StatusCode, &'static str);

    fn from_request_parts(
        parts: &mut Parts,
        _: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        // Nothing to wait for: the room was put on the request when it was routed.
        std::future::ready(
            parts
                .extensions
                .get::<Arc<Room>>()
                .cloned()
                .map(InRoom)
                .ok_or((StatusCode::NOT_FOUND, "no such room")),
        )
    }
}

/// A person, proven by the key in their link.
pub struct Guest {
    pub room: Arc<Room>,
    pub who: u8,
}

#[derive(Deserialize)]
struct Key {
    k: String,
}

impl<S: Send + Sync> FromRequestParts<S> for Guest {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let InRoom(room) = InRoom::from_request_parts(parts, state).await?;
        let Query(Key { k }) = Query::<Key>::from_request_parts(parts, state)
            .await
            .map_err(|_| (StatusCode::FORBIDDEN, "no key"))?;
        let who = room.who(&k).ok_or((StatusCode::FORBIDDEN, "wrong key"))?;
        Ok(Guest { room, who })
    }
}

/// The lobby, proven by the secret it armed the room with.
pub struct Lobby(pub Arc<Room>);

impl<S: Send + Sync> FromRequestParts<S> for Lobby {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let InRoom(room) = InRoom::from_request_parts(parts, state).await?;
        let shown = parts
            .headers
            .get("x-duet-secret")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if room.is_lobby(shown) {
            Ok(Lobby(room))
        } else {
            Err((StatusCode::FORBIDDEN, "lobby only"))
        }
    }
}

/// Serves one request for `room`. The router is built once and shared.
pub async fn serve_in(room: Arc<Room>, mut req: Request<Body>) -> Response<Body> {
    static ROUTER: OnceLock<Router> = OnceLock::new();
    req.extensions_mut().insert(room);
    match ROUTER.get_or_init(router).clone().oneshot(req).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

pub fn router() -> Router {
    Router::new()
        .route("/", get(page))
        .route("/fonts/{file}", get(font))
        .route("/ws", get(socket))
        .route("/hello", get(hello))
        .route("/poll", get(poll))
        .route("/send", post(send))
        .route("/internal/status", get(status))
        .route("/internal/export", get(export))
        .route("/internal/end", post(end))
}

fn json(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap()
}

async fn page(_: InRoom, headers: HeaderMap) -> Response<Body> {
    room_asset("/index.html", &headers)
}

async fn font(Path(file): Path<String>, headers: HeaderMap) -> Response<Body> {
    room_asset(&format!("/fonts/{file}"), &headers)
}

#[derive(Deserialize)]
#[expect(
    clippy::struct_field_names,
    reason = "the field names are the query parameters"
)]
struct After {
    #[serde(default)]
    after: u64,
    #[serde(default)]
    la: u64,
    #[serde(default)]
    hold: Option<u64>,
}

async fn socket(
    guest: Guest,
    Query(q): Query<After>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    upgrade
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |ws| serve(ws, guest.room, guest.who, q.after))
}

async fn serve(ws: WebSocket, room: Arc<Room>, who: u8, after: u64) {
    let (mut out, mut inp) = ws.split();
    let (hello, mut sent, mut feed) = room.attach(who, after);
    let mut heard = Instant::now();
    let mut ping = tokio::time::interval(SOCKET_PING);
    ping.tick().await;
    let mut ok = out.send(Message::Text(hello)).await.is_ok();
    while ok {
        tokio::select! {
            frame = feed.recv() => match frame {
                Ok(Frame::Event { seq, json }) => {
                    if seq <= sent {
                        continue; // already in the greeting
                    }
                    if seq == sent + 1 {
                        sent = seq;
                        ok = out.send(Message::Text(json)).await.is_ok();
                    } else {
                        ok = catch_up(&room, &mut out, &mut sent).await;
                    }
                }
                Ok(Frame::Live { from, json }) => {
                    if from != Some(who) {
                        ok = out.send(Message::Text(json)).await.is_ok();
                    }
                }
                // This connection was too slow for the feed: resend from the log instead.
                Err(RecvError::Lagged(_)) => ok = catch_up(&room, &mut out, &mut sent).await,
                Err(RecvError::Closed) => break,
            },
            msg = inp.next() => match msg {
                Some(Ok(Message::Text(text))) => {
                    heard = Instant::now();
                    if let Ok(msg) = serde_json::from_str::<ClientMsg>(text.as_str())
                        && let Some(reply) = room.handle(who, msg) {
                            ok = out.send(Message::Text(reply.into())).await.is_ok();
                        }
                }
                Some(Ok(Message::Close(_)) | Err(_)) | None => break,
                Some(Ok(_)) => heard = Instant::now(),
            },
            _ = ping.tick() => {
                if heard.elapsed() > SOCKET_SILENCE {
                    break;
                }
                ok = out.send(Message::Ping(axum::body::Bytes::new())).await.is_ok();
            }
        }
    }
    room.detach(who);
}

async fn catch_up(
    room: &Room,
    out: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    sent: &mut u64,
) -> bool {
    for (seq, json) in room.events_after(*sent) {
        if out.send(Message::Text(json)).await.is_err() {
            return false;
        }
        *sent = seq;
    }
    true
}

/// The greeting, for a page that will use held requests.
async fn hello(guest: Guest, Query(q): Query<After>) -> Response<Body> {
    json(
        StatusCode::OK,
        guest.room.hello_for_poll(guest.who, q.after),
    )
}

/// Answers as soon as there is something new, or after `hold` seconds with nothing.
async fn poll(guest: Guest, Query(q): Query<After>) -> Response<Body> {
    let hold = Duration::from_secs(q.hold.unwrap_or(25)).min(HOLD_MAX);
    let deadline = tokio::time::Instant::now() + hold;
    let mut wake = guest.room.wake();
    loop {
        if let Some(body) = guest.room.poll(guest.who, q.after, q.la) {
            return json(StatusCode::OK, body);
        }
        tokio::select! {
            changed = wake.changed() => if changed.is_err() { break },
            () = tokio::time::sleep_until(deadline) => break,
        }
    }
    json(StatusCode::OK, "{}".into())
}

/// One or more messages from a page on held requests.
async fn send(guest: Guest, body: axum::body::Bytes) -> Response<Body> {
    if body.len() > MAX_MESSAGE_BYTES {
        return json(StatusCode::PAYLOAD_TOO_LARGE, "{}".into());
    }
    let Ok(msgs) = serde_json::from_slice::<Vec<ClientMsg>>(&body) else {
        return json(StatusCode::BAD_REQUEST, "{}".into());
    };
    let replies: Vec<String> = msgs
        .into_iter()
        .take(64)
        .filter_map(|m| guest.room.handle(guest.who, m))
        .collect();
    json(StatusCode::OK, format!("{{\"r\":[{}]}}", replies.join(",")))
}

async fn status(Lobby(room): Lobby) -> Response<Body> {
    json(StatusCode::OK, room.status().to_string())
}

async fn export(Lobby(room): Lobby) -> Response<Body> {
    match room.export() {
        Some(chat) => json(StatusCode::OK, chat.to_string()),
        None => json(StatusCode::FORBIDDEN, "{\"error\":\"not reported\"}".into()),
    }
}

async fn end(Lobby(room): Lobby) -> Response<Body> {
    room.end("ended");
    json(StatusCode::OK, room.status().to_string())
}
