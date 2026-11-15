//! The room page and its fonts, packed into the binary by `build.rs`.

use axum::body::Body;
use axum::http::{HeaderMap, Response, StatusCode, header};

include!(concat!(env!("OUT_DIR"), "/room.rs"));

/// What a page may load and do. Scripts and styles are inlined into each page when it is packed,
/// so inline ones are allowed; anything from another site is not.
pub const PAGE_POLICY: &str = "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' ws: wss:; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

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
        // Everything a page needs is inside it or beside it. Nothing is fetched from anyone else,
        // no other site may show the page in a frame (so a Report or Send button cannot be
        // clicked through a disguise), and the page asks for no camera, microphone or location.
        out = out
            .header("Content-Security-Policy", PAGE_POLICY)
            .header("X-Frame-Options", "DENY")
            .header(
                "Permissions-Policy",
                "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
            )
            .header("Cross-Origin-Opener-Policy", "same-origin");
    }
    out.body(Body::from(reply.body)).unwrap()
}

pub fn room_asset(path: &str, headers: &HeaderMap) -> Response<Body> {
    respond(ASSETS, path, headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_cannot_be_framed_or_load_from_other_sites() {
        let page = room_asset("/index.html", &HeaderMap::new());
        let h = page.headers();
        let get = |name: &str| h.get(name).and_then(|v| v.to_str().ok()).unwrap_or("");
        assert!(
            get("content-type").starts_with("text/html"),
            "{:?}",
            get("content-type")
        );
        assert!(get("content-security-policy").contains("frame-ancestors 'none'"));
        assert!(get("content-security-policy").contains("default-src 'self'"));
        assert!(get("content-security-policy").contains("object-src 'none'"));
        assert_eq!(get("x-frame-options"), "DENY");
        assert_eq!(get("x-content-type-options"), "nosniff");
        assert_eq!(get("referrer-policy"), "no-referrer");
        assert!(get("permissions-policy").contains("camera=()"));
    }
}
