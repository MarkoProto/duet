//! The room page and its fonts, packed into the binary by `build.rs`.

use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode, header};

include!(concat!(env!("OUT_DIR"), "/room.rs"));

/// Turns a packed file into a response. Shared with the lobby, which has its own table.
pub fn respond(
    assets: &'static [duet_embed::Asset],
    path: &str,
    headers: &HeaderMap,
) -> Response<Body> {
    let Some(asset) = duet_embed::find(assets, path) else {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .unwrap();
    };
    let get = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    let reply = duet_embed::reply(
        asset,
        get(header::ACCEPT_ENCODING),
        get(header::IF_NONE_MATCH),
    );
    let mut out = Response::builder()
        .status(reply.status)
        .header(header::CONTENT_TYPE, reply.mime)
        .header(header::ETAG, reply.etag)
        .header(header::CACHE_CONTROL, reply.cache)
        .header(header::VARY, "Accept-Encoding")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer");
    if let Some(encoding) = reply.encoding {
        out = out.header(header::CONTENT_ENCODING, encoding);
    }
    if reply.mime.starts_with("text/html") {
        // Everything a page needs is inside it or beside it. Nothing is fetched from anyone else.
        out = out.header(
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' ws: wss:; base-uri 'none'; form-action 'self'",
        );
    }
    out.body(Body::from(reply.body)).unwrap()
}

pub fn room_asset(path: &str, headers: &HeaderMap) -> Response<Body> {
    respond(ASSETS, path, headers)
}
