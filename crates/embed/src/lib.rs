//! Web files packed into the binary.
//!
//! At build time each page is flattened into one HTML file (its style sheets and scripts are
//! pasted in), then stored three ways: as is, gzip and Brotli. At run time a request costs a
//! binary search and a slice: nothing is read from disk and nothing is compressed.

/// One file as it will be served.
pub struct Asset {
    /// Path it answers on, such as `/index.html`.
    pub path: &'static str,
    pub mime: &'static str,
    /// Quoted, ready for the `ETag` header.
    pub etag: &'static str,
    pub raw: &'static [u8],
    pub gz: Option<&'static [u8]>,
    pub br: Option<&'static [u8]>,
    /// Never changes under this name, so a browser may keep it for a year.
    pub immutable: bool,
}

/// What to send back. The caller turns it into its own response type.
pub struct Reply {
    pub status: u16,
    pub mime: &'static str,
    pub etag: &'static str,
    pub encoding: Option<&'static str>,
    pub cache: &'static str,
    pub body: &'static [u8],
}

/// Finds a file by path. `assets` is sorted by path at build time.
pub fn find<'a>(assets: &'a [Asset], path: &str) -> Option<&'a Asset> {
    assets
        .binary_search_by(|a| a.path.cmp(path))
        .ok()
        .map(|i| &assets[i])
}

/// Picks the smallest form the browser accepts, or answers 304 when it already has this one.
pub fn reply(asset: &Asset, accept_encoding: Option<&str>, if_none_match: Option<&str>) -> Reply {
    let cache = if asset.immutable {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    if let Some(tags) = if_none_match
        && tags
            .split(',')
            .any(|t| t.trim().trim_start_matches("W/") == asset.etag)
    {
        return Reply {
            status: 304,
            mime: asset.mime,
            etag: asset.etag,
            encoding: None,
            cache,
            body: &[],
        };
    }
    let accepts = |name: &str| {
        accept_encoding.is_some_and(|h| {
            h.split(',').any(|part| {
                let mut bits = part.split(';');
                let coding = bits.next().unwrap_or("").trim();
                let refused =
                    bits.any(|q| matches!(q.trim(), "q=0" | "q=0.0" | "q=0.00" | "q=0.000"));
                coding.eq_ignore_ascii_case(name) && !refused
            })
        })
    };
    let (encoding, body) = match (asset.br, asset.gz) {
        (Some(br), _) if accepts("br") => (Some("br"), br),
        (_, Some(gz)) if accepts("gzip") => (Some("gzip"), gz),
        _ => (None, asset.raw),
    };
    Reply {
        status: 200,
        mime: asset.mime,
        etag: asset.etag,
        encoding,
        cache,
        body,
    }
}

#[cfg(feature = "build")]
pub mod build {
    //! Used from `build.rs`. Walks the web folders and writes the table of [`super::Asset`]s.

    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::fs;
    use std::io::Write as _;
    use std::path::{Path, PathBuf};

    /// A folder to serve, and the URL prefix it appears under (`""` for the root).
    pub struct Mount<'a> {
        pub dir: &'a Path,
        pub prefix: &'a str,
    }

    fn mime_of(path: &Path) -> Option<(&'static str, bool)> {
        // (type, worth compressing)
        Some(match path.extension()?.to_str()? {
            "html" => ("text/html; charset=utf-8", true),
            "svg" => ("image/svg+xml", true),
            "json" => ("application/json", true),
            "txt" => ("text/plain; charset=utf-8", true),
            "ico" => ("image/x-icon", true),
            "woff2" => ("font/woff2", false),
            "png" => ("image/png", false),
            "webp" => ("image/webp", false),
            "jpg" | "jpeg" => ("image/jpeg", false),
            _ => return None, // .css and .js only ever travel inside a page
        })
    }

    fn fnv(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
        })
    }

    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.push(path);
            }
        }
    }

    /// Resolves `href`/`src` as written in a page: `/s/x` means the shared folder, anything else
    /// is beside the page.
    fn resolve(page: &Path, shared: &Path, link: &str) -> PathBuf {
        match link.strip_prefix("/s/") {
            Some(rest) => shared.join(rest),
            None => page.parent().unwrap_or(Path::new(".")).join(link),
        }
    }

    /// Pastes every local style sheet and script into the page, so a page is one request.
    fn flatten(page: &Path, shared: &Path) -> String {
        let html = fs::read_to_string(page).unwrap_or_else(|e| panic!("{}: {e}", page.display()));
        let mut out = String::with_capacity(html.len() * 4);
        let mut rest = html.as_str();
        loop {
            let css = rest.find("<link rel=\"stylesheet\" href=\"");
            let js = rest.find("<script src=\"");
            let (at, is_css) = match (css, js) {
                (Some(c), Some(j)) if c < j => (c, true),
                (Some(c), None) => (c, true),
                (_, Some(j)) => (j, false),
                (None, None) => break,
            };
            out.push_str(&rest[..at]);
            let open = if is_css {
                "<link rel=\"stylesheet\" href=\""
            } else {
                "<script src=\""
            };
            let after = &rest[at + open.len()..];
            let quote = after.find('"').expect("unclosed link");
            let link = &after[..quote];
            let close = if is_css { ">" } else { "></script>" };
            let end = after[quote..].find(close).expect("unclosed tag") + quote + close.len();
            let file = resolve(page, shared, link);
            println!("cargo:rerun-if-changed={}", file.display());
            let body = fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{} wants {}: {e}", page.display(), file.display()));
            if is_css {
                let _ = write!(out, "<style>{}</style>", body.trim());
            } else {
                // A script must not contain its own closing tag.
                let _ = write!(
                    out,
                    "<script>{}</script>",
                    body.trim().replace("</script", "<\\/script")
                );
            }
            rest = &after[end..];
        }
        out.push_str(rest);
        out
    }

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        enc.write_all(bytes).expect("gzip");
        enc.finish().expect("gzip")
    }

    fn brotli(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = brotli::CompressorWriter::new(&mut out, 4096, 11, 22);
            enc.write_all(bytes).expect("brotli");
        }
        out
    }

    /// Writes `<out_dir>/<name>.rs` holding `pub static ASSETS: &[duet_embed::Asset]`.
    pub fn pack(out_dir: &Path, name: &str, mounts: &[Mount], shared: &Path) {
        let blobs = out_dir.join(format!("{name}-blobs"));
        fs::create_dir_all(&blobs).expect("blob dir");
        let mut table: BTreeMap<String, String> = BTreeMap::new();
        let mut n = 0usize;
        for mount in mounts {
            println!("cargo:rerun-if-changed={}", mount.dir.display());
            let mut files = Vec::new();
            walk(mount.dir, &mut files);
            for file in files {
                println!("cargo:rerun-if-changed={}", file.display());
                let Some((mime, squeeze)) = mime_of(&file) else {
                    continue;
                };
                let is_page = file.extension().is_some_and(|e| e == "html");
                let raw = if is_page {
                    flatten(&file, shared).into_bytes()
                } else {
                    fs::read(&file).expect("read")
                };
                let rel = file
                    .strip_prefix(mount.dir)
                    .expect("inside mount")
                    .to_string_lossy()
                    .replace('\\', "/");
                let url = format!("{}/{}", mount.prefix.trim_end_matches('/'), rel);
                let save = |suffix: &str, bytes: &[u8]| -> String {
                    let path = blobs.join(format!("{n}.{suffix}"));
                    fs::write(&path, bytes).expect("write blob");
                    format!("include_bytes!({:?})", path.to_string_lossy())
                };
                let raw_src = save("raw", &raw);
                let pick = |packed: Vec<u8>| (packed.len() + 64 < raw.len()).then_some(packed);
                let gz = if squeeze { pick(gzip(&raw)) } else { None };
                let br = if squeeze { pick(brotli(&raw)) } else { None };
                let gz_src = gz.map_or("None".to_string(), |b| format!("Some({})", save("gz", &b)));
                let br_src = br.map_or("None".to_string(), |b| format!("Some({})", save("br", &b)));
                let etag = format!("\"{:016x}\"", fnv(&raw));
                let immutable = !squeeze; // fonts and pictures keep their name for life
                table.insert(
                    url.clone(),
                    format!(
                        "    duet_embed::Asset {{ path: {url:?}, mime: {mime:?}, etag: {etag:?}, raw: {raw_src}, gz: {gz_src}, br: {br_src}, immutable: {immutable} }},\n"
                    ),
                );
                n += 1;
            }
        }
        let mut code = String::from("pub static ASSETS: &[duet_embed::Asset] = &[\n");
        for row in table.values() {
            code.push_str(row); // BTreeMap keeps paths sorted, which `find` relies on
        }
        code.push_str("];\n");
        fs::write(out_dir.join(format!("{name}.rs")), code).expect("write table");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> Asset {
        Asset {
            path: "/a",
            mime: "text/html",
            etag: "\"1\"",
            raw: b"raw",
            gz: Some(b"gz"),
            br: Some(b"br"),
            immutable: false,
        }
    }

    #[test]
    fn picks_the_best_coding_offered() {
        assert_eq!(
            reply(&asset(), Some("gzip, deflate, br"), None).encoding,
            Some("br")
        );
        assert_eq!(reply(&asset(), Some("gzip"), None).encoding, Some("gzip"));
        assert_eq!(
            reply(&asset(), Some("br;q=0, gzip"), None).encoding,
            Some("gzip")
        );
        assert_eq!(reply(&asset(), None, None).body, b"raw");
    }

    #[test]
    fn answers_not_modified() {
        assert_eq!(reply(&asset(), None, Some("\"1\"")).status, 304);
        assert_eq!(reply(&asset(), None, Some("W/\"1\", \"2\"")).status, 304);
        assert_eq!(reply(&asset(), None, Some("\"2\"")).status, 200);
    }

    #[test]
    fn finds_by_path() {
        let all = [asset()];
        assert!(find(&all, "/a").is_some());
        assert!(find(&all, "/b").is_none());
    }
}
