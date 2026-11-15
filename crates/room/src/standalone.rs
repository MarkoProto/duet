//! A room running alone, as it does inside a sandbox: idle until the lobby arms it.

use std::sync::{Arc, RwLock};

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Response, StatusCode, header};
use axum::routing::{get, post};

use crate::text::same_secret;
use crate::{Room, RoomConfig};

pub struct Standalone {
    secret: String,
    room: RwLock<Option<Arc<Room>>>,
}

impl Standalone {
    pub fn new(secret: String) -> Arc<Self> {
        Arc::new(Self {
            secret,
            room: RwLock::new(None),
        })
    }

    pub fn room(&self) -> Option<Arc<Room>> {
        self.room
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Gives the room its plan. Refused if it already has one.
    pub fn arm(&self, cfg: RoomConfig) -> Result<(), &'static str> {
        let mut slot = self
            .room
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if slot.is_some() {
            return Err("already armed");
        }
        let room = Room::new(cfg, self.secret.clone());
        room.run();
        *slot = Some(room);
        Ok(())
    }
}

fn json(status: StatusCode, body: String) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .unwrap()
}

async fn health(State(app): State<Arc<Standalone>>) -> Response<Body> {
    let room = app.room();
    let phase = room.as_ref().map(|r| r.phase().name());
    json(StatusCode::OK, serde_json::json!({"ok": true, "armed": room.is_some(), "phase": phase, "version": env!("CARGO_PKG_VERSION")}).to_string())
}

async fn arm(
    State(app): State<Arc<Standalone>>,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    let shown = headers
        .get("x-duet-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !same_secret(shown, &app.secret) {
        return json(StatusCode::FORBIDDEN, "{\"error\":\"lobby only\"}".into());
    }
    let cfg = match serde_json::from_slice::<RoomConfig>(&body)
        .map_err(|e| e.to_string())
        .and_then(RoomConfig::checked)
    {
        Ok(cfg) => cfg,
        Err(why) => {
            return json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({"error": why}).to_string(),
            );
        }
    };
    match app.arm(cfg) {
        Ok(()) => json(StatusCode::OK, "{\"ok\":true}".into()),
        Err(why) => json(
            StatusCode::CONFLICT,
            serde_json::json!({"error": why}).to_string(),
        ),
    }
}

async fn rest(State(app): State<Arc<Standalone>>, req: Request) -> Response<Body> {
    match app.room() {
        Some(room) => crate::http::serve_in(room, req).await,
        None => Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(header::RETRY_AFTER, "1")
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from("This room is not open yet."))
            .unwrap(),
    }
}

pub fn router(app: Arc<Standalone>) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/internal/arm", post(arm))
        .fallback(rest)
        .with_state(app)
}
