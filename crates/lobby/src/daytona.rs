//! Daytona, spoken as plain web requests.
//!
//! There is no Rust SDK, so every call here mirrors what Daytona's own Python package sends.
//! Two APIs are spoken: the old one (SDK 0.231.1, `docs/daytona-rest-spec.md`) and the new one
//! that SDK 2.0.0 introduced on 2026-11-13 (`docs/daytona-v2-rest-spec.md`). Which one is used is
//! decided once, by [`Daytona::pick`]. Until `duet doctor` has run against a real key, treat this
//! file as carefully copied rather than proven.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use duet_room::RoomConfig;
use duet_room::text::token;
use serde_json::{Value, json};

use crate::rooms::{OnStage, Ready, Receipt};
use crate::settings::{DaytonaApi, DaytonaSettings};

const ROOM_PORT: u16 = 3000;
/// The contract each API is spoken by. One endpoint answers differently to older callers.
const SDK_V0: &str = "0.231.1";
const SDK_V2: &str = "2.0.0";
/// Without this, a Daytona preview can answer with its own warning page, which would look to
/// the lobby like a room that is up.
const SKIP_WARNING: &str = "X-Daytona-Skip-Preview-Warning";

#[derive(Clone, Debug)]
pub struct Sandbox {
    pub id: String,
    /// In the old API's words: `creating`, `starting`, `started`, `error`, `destroying`,
    /// `destroyed`. The new API's states are translated into these.
    pub state: String,
    /// Base for file and process calls.
    pub toolbox: String,
    pub error: Option<String>,
}

pub struct Daytona {
    http: reqwest::Client,
    api: String,
    api2: String,
    choice: DaytonaApi,
    key: String,
    target: Option<String>,
    /// Whether the new API is in use, once [`Daytona::pick`] has decided.
    v2: AtomicBool,
    /// New API: each sandbox's own access token for its toolbox.
    tokens: Mutex<HashMap<String, String>>,
    /// New API: sandboxes whose room port was already exposed.
    exposed: Mutex<HashSet<String>>,
}

fn snippet(text: &str) -> String {
    text.chars().take(400).collect()
}

fn ok(status: u16) -> bool {
    (200..300).contains(&status)
}

/// Reads the message out of an error body, which may be JSON in one of two shapes or plain text.
fn error_text(status: u16, body: &str) -> String {
    let parsed: Option<Value> = serde_json::from_str(body).ok();
    let message = parsed
        .as_ref()
        .and_then(|v| {
            v.get("message")
                .and_then(Value::as_str)
                .or_else(|| v.get("error").and_then(Value::as_str))
                .map(str::to_string)
        })
        .or_else(|| {
            parsed
                .as_ref()
                .and_then(|v| v.get("errors"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join("; ")
                })
        })
        .unwrap_or_else(|| snippet(body));
    let code = parsed
        .as_ref()
        .and_then(|v| v.get("code"))
        .and_then(Value::as_str)
        .map(|c| format!(" ({c})"))
        .unwrap_or_default();
    format!("Daytona answered {status}: {message}{code}")
}

/// The new API's sandbox states, in the old API's words, so the rest of the lobby has one set.
fn old_state(status: &str) -> &str {
    match status {
        "running" => "started",
        "failed" | "lost" => "error",
        "deleted" => "destroyed",
        "deleting" => "destroying",
        "booting" => "starting",
        "queued" | "pending" | "assigned" => "creating",
        other => other,
    }
}

/// The toolbox address: the proxy with the id put in its place, or added at the end.
fn toolbox_of(proxy: &str, id: &str) -> String {
    let proxy = proxy.trim_end_matches('/');
    if proxy.is_empty() {
        String::new()
    } else if proxy.contains("{sandbox_id}") || proxy.contains("{sandboxId}") {
        proxy
            .replace("{sandbox_id}", id)
            .replace("{sandboxId}", id)
            .trim_end_matches('/')
            .to_string()
    } else {
        format!("{proxy}/{id}")
    }
}

/// The text of a process's log frames. `out_only` keeps what a command printed to stdout.
fn frames_text(logs: &Value, out_only: bool) -> String {
    logs.get("frames")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|f| {
            !out_only
                || matches!(
                    f.get("channel").and_then(Value::as_str),
                    Some("stdout" | "pty")
                )
        })
        .filter_map(|f| f.get("data").and_then(Value::as_str))
        .collect()
}

impl Daytona {
    pub fn new(settings: &DaytonaSettings) -> Self {
        let api2 = if settings.api2_url.is_empty() {
            "https://api.daytona.com".to_string()
        } else {
            settings.api2_url.trim_end_matches('/').to_string()
        };
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .user_agent(concat!("duet/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("http client"),
            api: settings.api_url.trim_end_matches('/').to_string(),
            api2,
            choice: settings.api,
            key: settings.key.clone(),
            target: settings.target.clone(),
            v2: AtomicBool::new(settings.api == DaytonaApi::V2),
            tokens: Mutex::default(),
            exposed: Mutex::default(),
        }
    }

    pub fn is_v2(&self) -> bool {
        self.v2.load(Ordering::Relaxed)
    }

    /// Which API is spoken, in words for the terminal and the admin page.
    pub fn api_name(&self) -> &'static str {
        if self.is_v2() {
            "the new API (SDK 2.0)"
        } else {
            "the original API (SDK 0.231)"
        }
    }

    /// The key, as the new API sees it. Changes nothing.
    pub async fn whoami(&self) -> Result<Value, String> {
        self.json(
            reqwest::Method::GET,
            format!("{}/api-keys/current", self.api2),
            None,
            Duration::from_secs(15),
        )
        .await
    }

    /// Decides which API to speak. With `Auto`, the new API is used if it knows the key.
    /// Returns what was decided and why, for the terminal.
    pub async fn pick(&self) -> String {
        match self.choice {
            DaytonaApi::V0 => {
                self.v2.store(false, Ordering::Relaxed);
                "the original API (SDK 0.231), as set".into()
            }
            DaytonaApi::V2 => {
                self.v2.store(true, Ordering::Relaxed);
                "the new API (SDK 2.0), as set".into()
            }
            DaytonaApi::Auto => match self.whoami().await {
                Ok(_) => {
                    self.v2.store(true, Ordering::Relaxed);
                    "the new API (SDK 2.0): it knows this key".into()
                }
                Err(why) => {
                    self.v2.store(false, Ordering::Relaxed);
                    format!("the original API (SDK 0.231), because the new one said: {why}")
                }
            },
        }
    }

    /// A request to the control plane, with the API key.
    fn request(&self, method: reqwest::Method, url: &str) -> reqwest::RequestBuilder {
        self.request_as(method, url, &self.key)
    }

    fn request_as(
        &self,
        method: reqwest::Method,
        url: &str,
        bearer: &str,
    ) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .bearer_auth(bearer)
            .header("X-Daytona-Source", "duet")
            .header(
                "X-Daytona-SDK-Version",
                if self.is_v2() { SDK_V2 } else { SDK_V0 },
            )
            .header("Accept", "application/json")
    }

    /// Sends, and returns the status and body. Connection failures are retried; answers are not.
    async fn send(
        &self,
        build: impl Fn() -> reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<(u16, String), String> {
        let mut last = String::new();
        for attempt in 0..3u32 {
            match build().timeout(timeout).send().await {
                Ok(reply) => {
                    let status = reply.status().as_u16();
                    let body = reply.text().await.unwrap_or_default();
                    return Ok((status, body));
                }
                Err(e) if e.is_connect() || e.is_timeout() => {
                    last = e.to_string();
                    tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
                }
                Err(e) => return Err(format!("could not reach Daytona: {e}")),
            }
        }
        Err(format!("could not reach Daytona: {last}"))
    }

    fn parse(status: u16, text: &str) -> Result<Value, String> {
        if !ok(status) {
            return Err(error_text(status, text));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(text).map_err(|_| {
            format!(
                "Daytona answered {status} with something that is not JSON: {}",
                snippet(text)
            )
        })
    }

    async fn json(
        &self,
        method: reqwest::Method,
        url: String,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Value, String> {
        let (status, text) = self
            .send(
                || {
                    let req = self.request(method.clone(), &url);
                    match &body {
                        Some(b) => req.json(b),
                        None => req,
                    }
                },
                timeout,
            )
            .await?;
        Self::parse(status, &text)
    }

    /// New API: the sandbox's own toolbox token, minted when there is none or it was refused.
    /// Where minting is not offered, the API key stands in, as the SDK does.
    async fn toolbox_token(&self, id: &str, fresh: bool) -> String {
        if !fresh
            && let Some(t) = self
                .tokens
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .get(id)
        {
            return t.clone();
        }
        let url = format!("{}/sandboxes/{id}/access-token", self.api2);
        let minted = match self
            .send(
                || self.request(reqwest::Method::POST, &url).json(&json!({})),
                Duration::from_secs(20),
            )
            .await
        {
            Ok((status, text)) if ok(status) => {
                serde_json::from_str::<Value>(&text).ok().and_then(|v| {
                    v.get("access_token")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
            }
            _ => None,
        };
        let t = minted.unwrap_or_else(|| self.key.clone());
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(id.to_string(), t.clone());
        t
    }

    /// A call to a sandbox's toolbox. The old API takes the API key; the new one the sandbox's
    /// own token, minted again once if it is refused.
    async fn toolbox_send(
        &self,
        sb: &Sandbox,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
        timeout: Duration,
    ) -> Result<(u16, String), String> {
        if !self.is_v2() {
            return self.send(|| build(&self.key), timeout).await;
        }
        let t = self.toolbox_token(&sb.id, false).await;
        let (status, text) = self.send(|| build(&t), timeout).await?;
        if status != 401 {
            return Ok((status, text));
        }
        let t = self.toolbox_token(&sb.id, true).await;
        self.send(|| build(&t), timeout).await
    }

    fn sandbox(v: &Value) -> Result<Sandbox, String> {
        let id = v
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("no sandbox id in {}", snippet(&v.to_string())))?
            .to_string();
        let proxy = v
            .get("toolbox_proxy_url")
            .or_else(|| v.get("toolboxProxyUrl"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let state = match v.get("status").and_then(Value::as_str) {
            Some(status) => old_state(status).to_string(),
            None => v
                .get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
        };
        Ok(Sandbox {
            toolbox: toolbox_of(proxy, &id),
            state,
            error: v
                .get("errorReason")
                .or_else(|| v.get("error"))
                .and_then(Value::as_str)
                .map(str::to_string),
            id,
        })
    }

    /// Creates a sandbox that Daytona itself deletes after `ttl_minutes`, whatever happens here.
    pub async fn create(
        &self,
        labels: &[(&str, &str)],
        ttl_minutes: u64,
        snapshot: Option<&str>,
    ) -> Result<Sandbox, String> {
        let labels: serde_json::Map<_, _> = labels
            .iter()
            .map(|(k, v)| (k.to_string(), json!(v)))
            .collect();
        if self.is_v2() {
            let mut body = json!({
                "labels": labels,
                // The hard lifetime. Linux sandboxes can only be deleted at the end of it, which is
                // what a room wants. Idle time must not end a room mid-session.
                "lifecycle_policy": {"timeout_seconds": ttl_minutes * 60, "on_timeout": "delete", "idle_expiry_seconds": -1},
                "wait_seconds": 30,
                // A sandbox that never comes up is deleted by Daytona too.
                "create_timeout_seconds": 180,
            });
            if let Some(s) = snapshot {
                body["image"] = json!(s);
            }
            if let Some(t) = &self.target {
                body["region_id"] = json!(t);
            }
            // The same key on a retry, so a lost answer cannot make two rooms.
            let idem = format!("duet-{}", token(20));
            let url = format!("{}/sandboxes", self.api2);
            let (status, text) = self
                .send(
                    || {
                        let mut req = self
                            .request(reqwest::Method::POST, &url)
                            .header("Idempotency-Key", &idem)
                            .json(&body);
                        if let Some(t) = &self.target {
                            req = req.header("x-daytona-region", t);
                        }
                        req
                    },
                    Duration::from_secs(45),
                )
                .await?;
            let v = match Self::parse(status, &text) {
                Ok(v) => v,
                // Some accounts know the key on the new API but may not create Linux sandboxes
                // there yet. When the choice was left to us, the old API takes over.
                Err(why)
                    if self.choice == DaytonaApi::Auto && why.contains("FEATURE_NOT_ENABLED") =>
                {
                    eprintln!(
                        "duet: the new Daytona API may not create sandboxes for this account ({why}); using the original API"
                    );
                    self.v2.store(false, Ordering::Relaxed);
                    return Box::pin(
                        self.create(
                            &labels
                                .iter()
                                .map(|(k, v)| (k.as_str(), v.as_str().unwrap_or("")))
                                .collect::<Vec<_>>(),
                            ttl_minutes,
                            snapshot,
                        ),
                    )
                    .await;
                }
                Err(why) => return Err(why),
            };
            let sb = Self::sandbox(&v)?;
            if let Some(t) = v.get("access_token").and_then(Value::as_str) {
                self.tokens
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(sb.id.clone(), t.to_string());
            }
            return Ok(sb);
        }
        let mut body = json!({
            "env": {},
            "labels": labels,
            "volumes": [],
            "public": false,
            "ttlMinutes": ttl_minutes,
            // Idle time must not stop a room in the middle of a session; the lifetime above ends it.
            "autoStopInterval": 0,
            // Once stopped, it is deleted at once: nothing of a session is kept on disk.
            "autoDeleteInterval": 0,
        });
        if let Some(s) = snapshot {
            body["snapshot"] = json!(s);
        }
        if let Some(t) = &self.target {
            body["target"] = json!(t);
        }
        let v = self
            .json(
                reqwest::Method::POST,
                format!("{}/sandbox", self.api),
                Some(body),
                Duration::from_secs(60),
            )
            .await?;
        Self::sandbox(&v)
    }

    /// The sandbox, or `None` once it no longer exists.
    pub async fn get(&self, id: &str) -> Result<Option<Sandbox>, String> {
        let url = if self.is_v2() {
            format!("{}/sandboxes/{id}", self.api2)
        } else {
            format!("{}/sandbox/{id}", self.api)
        };
        let (status, text) = self
            .send(
                || self.request(reqwest::Method::GET, &url),
                Duration::from_secs(20),
            )
            .await?;
        if status == 404 {
            return Ok(None);
        }
        let v = Self::parse(status, &text)?;
        Self::sandbox(&v).map(Some)
    }

    /// Waits until the sandbox runs. Fails fast on any state it cannot recover from.
    pub async fn wait_started(&self, mut sb: Sandbox, within: Duration) -> Result<Sandbox, String> {
        let deadline = Instant::now() + within;
        let mut pause = Duration::from_millis(100);
        loop {
            match sb.state.as_str() {
                "started" => break,
                "error" | "build_failed" | "destroyed" | "destroying" => {
                    return Err(format!(
                        "the sandbox ended up {}: {}",
                        sb.state,
                        sb.error.clone().unwrap_or_default()
                    ));
                }
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "the sandbox was still {} after {} s",
                    sb.state,
                    within.as_secs()
                ));
            }
            tokio::time::sleep(pause).await;
            pause = (pause * 11 / 10).min(Duration::from_secs(1));
            let known = std::mem::take(&mut sb.toolbox);
            sb = self
                .get(&sb.id)
                .await?
                .ok_or("the sandbox vanished while starting")?;
            // The new API gives the toolbox address on create and not always on a read.
            if sb.toolbox.is_empty() {
                sb.toolbox = known;
            }
        }
        if sb.toolbox.is_empty() {
            if self.is_v2() {
                return Err("Daytona gave no toolbox address".into());
            }
            let v = self
                .json(
                    reqwest::Method::GET,
                    format!("{}/sandbox/{}/toolbox-proxy-url", self.api, sb.id),
                    None,
                    Duration::from_secs(20),
                )
                .await?;
            let url = v
                .get("url")
                .and_then(Value::as_str)
                .ok_or("Daytona gave no toolbox address")?;
            sb.toolbox = format!("{}/{}", url.trim_end_matches('/'), sb.id);
        }
        Ok(sb)
    }

    /// Asks for the sandbox to be deleted. One that is already gone counts as deleted.
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        let url = if self.is_v2() {
            format!("{}/sandboxes/{id}", self.api2)
        } else {
            format!("{}/sandbox/{id}", self.api)
        };
        let (status, text) = self
            .send(
                || self.request(reqwest::Method::DELETE, &url),
                Duration::from_secs(30),
            )
            .await?;
        self.tokens
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        self.exposed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(id);
        if status == 404 || ok(status) {
            Ok(())
        } else {
            Err(error_text(status, &text))
        }
    }

    /// Checks that a deleted sandbox is really gone.
    pub async fn wait_gone(&self, id: &str, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        loop {
            match self.get(id).await {
                Ok(None) => return true,
                Ok(Some(sb)) if sb.state == "destroyed" => return true,
                _ => {}
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    /// Runs a command and waits for it. Returns its exit code and what it printed.
    pub async fn exec(
        &self,
        sb: &Sandbox,
        command: &str,
        timeout_s: u64,
    ) -> Result<(i64, String), String> {
        if self.is_v2() {
            return self.exec_v2(sb, command, timeout_s).await;
        }
        let body = json!({"command": command, "timeout": timeout_s});
        let v = self
            .json(
                reqwest::Method::POST,
                format!("{}/process/execute", sb.toolbox),
                Some(body),
                Duration::from_secs(timeout_s + 10),
            )
            .await?;
        let code = v
            .get("exitCode")
            .or_else(|| v.get("code"))
            .and_then(Value::as_i64)
            .unwrap_or(-1);
        Ok((
            code,
            v.get("result")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        ))
    }

    /// New API: one process, held until it ends, its output read from the log frames.
    async fn exec_v2(
        &self,
        sb: &Sandbox,
        command: &str,
        timeout_s: u64,
    ) -> Result<(i64, String), String> {
        let ms = timeout_s * 1000;
        let url = format!("{}/processes", sb.toolbox);
        // Two clocks: how long the answer is held (waitMs) and when the command is killed.
        let body = json!({"kind": "exec", "shellCommand": command, "stdin": "closed", "timeoutMs": ms, "keepLogs": "on_exit_ttl"});
        let query = [("waitMs", ms.to_string()), ("logs", "text".to_string())];
        let (status, text) = self
            .toolbox_send(
                sb,
                |t| {
                    self.request_as(reqwest::Method::POST, &url, t)
                        .query(&query)
                        .json(&body)
                },
                Duration::from_secs(timeout_s + 10),
            )
            .await?;
        let mut proc = Self::parse(status, &text)?;
        let pid = proc
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("no process id in {}", snippet(&text)))?
            .to_string();
        if proc.get("state").and_then(Value::as_str) != Some("terminal") {
            let wait = format!("{}/processes/{pid}/wait", sb.toolbox);
            let _ = self
                .toolbox_send(
                    sb,
                    |t| {
                        self.request_as(reqwest::Method::GET, &wait, t)
                            .query(&[("timeoutMs", ms.to_string())])
                    },
                    Duration::from_secs(timeout_s + 10),
                )
                .await?;
            let read = format!("{}/processes/{pid}", sb.toolbox);
            let (status, text) = self
                .toolbox_send(
                    sb,
                    |t| self.request_as(reqwest::Method::GET, &read, t),
                    Duration::from_secs(20),
                )
                .await?;
            proc = Self::parse(status, &text)?;
        }
        let mut out = String::new();
        let mut cursor: Option<String> = None;
        let mut complete = false;
        if let Some(logs) = proc.get("logs").filter(|l| l.is_object()) {
            out.push_str(&frames_text(logs, true));
            complete = logs.get("eof").and_then(Value::as_bool).unwrap_or(false);
            cursor = logs
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        let logs_url = format!("{}/processes/{pid}/logs", sb.toolbox);
        for _ in 0..20 {
            if complete {
                break;
            }
            let mut query = vec![("encoding", "text".to_string())];
            if let Some(c) = &cursor {
                query.push(("cursor", c.clone()));
            }
            let (status, text) = self
                .toolbox_send(
                    sb,
                    |t| {
                        self.request_as(reqwest::Method::GET, &logs_url, t)
                            .query(&query)
                    },
                    Duration::from_secs(20),
                )
                .await?;
            let page = Self::parse(status, &text)?;
            let added = frames_text(&page, true);
            let next = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            complete = page.get("eof").and_then(Value::as_bool).unwrap_or(true)
                || next.is_none()
                || next == cursor
                || page
                    .get("frames")
                    .and_then(Value::as_array)
                    .is_none_or(Vec::is_empty);
            out.push_str(&added);
            cursor = next;
        }
        let code = proc.get("exitCode").and_then(Value::as_i64).unwrap_or(-1);
        Ok((code, out))
    }

    /// Puts one file into the sandbox. `path` must be absolute.
    pub async fn upload(&self, sb: &Sandbox, path: &str, bytes: Vec<u8>) -> Result<(), String> {
        let (status, text) = if self.is_v2() {
            let url = format!("{}/files/upload", sb.toolbox);
            let name = path.rsplit('/').next().unwrap_or(path).to_string();
            self.toolbox_send(
                sb,
                |t| {
                    let part = reqwest::multipart::Part::bytes(bytes.clone())
                        .file_name(name.clone())
                        .mime_str("application/octet-stream")
                        .expect("mime");
                    self.request_as(reqwest::Method::POST, &url, t)
                        .query(&[("path", path)])
                        .header("Accept", "*/*")
                        .multipart(reqwest::multipart::Form::new().part("file", part))
                },
                Duration::from_secs(120),
            )
            .await?
        } else {
            let url = format!("{}/files/bulk-upload", sb.toolbox);
            self.send(
                || {
                    let part = reqwest::multipart::Part::bytes(bytes.clone())
                        .file_name(path.to_string())
                        .mime_str("application/octet-stream")
                        .expect("mime");
                    // The path travels as its own field, before the file it names.
                    let form = reqwest::multipart::Form::new()
                        .text("files[0].path", path.to_string())
                        .part("files[0].file", part);
                    self.request(reqwest::Method::POST, &url)
                        .header("Accept", "*/*")
                        .multipart(form)
                },
                Duration::from_secs(120),
            )
            .await?
        };
        if !ok(status) {
            return Err(error_text(status, &text));
        }
        // A 2xx can still carry a list of files that failed.
        let failed = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("errors").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        if failed.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "upload failed: {}",
                snippet(&Value::Array(failed).to_string())
            ))
        }
    }

    /// Starts a command that keeps running after this call returns, with `env` set for it.
    /// Returns an id for reading its output later.
    pub async fn start(
        &self,
        sb: &Sandbox,
        session: &str,
        command: &str,
        env: &[(&str, &str)],
    ) -> Result<String, String> {
        if self.is_v2() {
            // A secret goes in env: every read of a process echoes its command back.
            let env: serde_json::Map<_, _> =
                env.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
            let body = json!({"kind": "exec", "shellCommand": command, "env": env, "stdin": "closed", "keepLogs": "until_cleanup", "name": session});
            let url = format!("{}/processes", sb.toolbox);
            let (status, text) = self
                .toolbox_send(
                    sb,
                    |t| self.request_as(reqwest::Method::POST, &url, t).json(&body),
                    Duration::from_secs(30),
                )
                .await?;
            let v = Self::parse(status, &text)?;
            return Ok(v
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or(session)
                .to_string());
        }
        // The old API's sessions take no environment: it goes in front of the command.
        let prefix = env.iter().fold(String::new(), |mut out, (k, v)| {
            let _ = write!(out, "{k}={v} ");
            out
        });
        let url = format!("{}/process/session", sb.toolbox);
        let (status, text) = self
            .send(
                || {
                    self.request(reqwest::Method::POST, &url)
                        .json(&json!({"sessionId": session}))
                },
                Duration::from_secs(30),
            )
            .await?;
        if !ok(status) && status != 409 {
            return Err(error_text(status, &text));
        }
        let v = self
            .json(
                reqwest::Method::POST,
                format!("{}/process/session/{session}/exec", sb.toolbox),
                Some(json!({"command": format!("{prefix}{command}"), "runAsync": true})),
                Duration::from_secs(30),
            )
            .await?;
        Ok(v.get("cmdId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string())
    }

    /// What a started command has printed so far, for explaining a room that did not come up.
    pub async fn logs(&self, sb: &Sandbox, session: &str, cmd: &str) -> String {
        if self.is_v2() {
            let url = format!("{}/processes/{cmd}/logs", sb.toolbox);
            return match self
                .toolbox_send(
                    sb,
                    |t| {
                        self.request_as(reqwest::Method::GET, &url, t)
                            .query(&[("encoding", "text")])
                    },
                    Duration::from_secs(15),
                )
                .await
            {
                Ok((_, text)) => serde_json::from_str::<Value>(&text)
                    .ok()
                    .map(|v| frames_text(&v, false))
                    .unwrap_or(text),
                Err(e) => e,
            };
        }
        let url = format!(
            "{}/process/session/{session}/command/{cmd}/logs",
            sb.toolbox
        );
        match self
            .send(
                || self.request(reqwest::Method::GET, &url),
                Duration::from_secs(15),
            )
            .await
        {
            Ok((_, text)) => serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("output").and_then(Value::as_str).map(str::to_string))
                .unwrap_or(text),
            Err(e) => e,
        }
    }

    /// A link to one port that works without a key and stops working after `expires_s`.
    pub async fn signed_url(&self, id: &str, port: u16, expires_s: u64) -> Result<String, String> {
        let v = if self.is_v2() {
            // The port is exposed (privately) once, as the SDK does before making a link.
            let first = self
                .exposed
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(id.to_string());
            if first {
                let url = format!("{}/sandboxes/{id}/ports/{port}", self.api2);
                if let Ok((status, text)) = self
                    .send(
                        || {
                            self.request(reqwest::Method::PUT, &url)
                                .json(&json!({"public": false}))
                        },
                        Duration::from_secs(20),
                    )
                    .await
                    && !ok(status)
                {
                    eprintln!("duet: exposing port {port}: {}", error_text(status, &text));
                }
            }
            self.json(
                reqwest::Method::POST,
                format!(
                    "{}/sandboxes/{id}/ports/{port}/signed-preview-urls",
                    self.api2
                ),
                Some(json!({"expires_in_seconds": expires_s})),
                Duration::from_secs(20),
            )
            .await?
        } else {
            self.json(
                reqwest::Method::GET,
                format!(
                    "{}/sandbox/{id}/ports/{port}/signed-preview-url?expiresInSeconds={expires_s}",
                    self.api
                ),
                None,
                Duration::from_secs(20),
            )
            .await?
        };
        v.get("url")
            .and_then(Value::as_str)
            .map(|u| u.trim_end_matches('/').to_string())
            .ok_or_else(|| "Daytona gave no link".to_string())
    }

    /// Every sandbox carrying these labels.
    pub async fn list(&self, labels: &[(&str, &str)]) -> Result<Vec<Sandbox>, String> {
        let wanted = Value::Object(
            labels
                .iter()
                .map(|(k, v)| (k.to_string(), json!(v)))
                .collect(),
        );
        let (url, items_key, cursor_key) = if self.is_v2() {
            (
                format!("{}/sandboxes/search", self.api2),
                "sandboxes",
                "next_cursor",
            )
        } else {
            (format!("{}/sandbox", self.api), "items", "nextCursor")
        };
        let mut found = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let mut query = vec![("labels", wanted.to_string()), ("limit", "100".to_string())];
            if let Some(c) = &cursor {
                query.push(("cursor", c.clone()));
            }
            let (status, text) = self
                .send(
                    || self.request(reqwest::Method::GET, &url).query(&query),
                    Duration::from_secs(30),
                )
                .await?;
            let v = Self::parse(status, &text)?;
            for item in v
                .get(items_key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                // Search is a containment match, but check: a clean-up must never touch others.
                let labels_match = wanted.as_object().is_some_and(|w| {
                    w.iter()
                        .all(|(k, val)| item.get("labels").and_then(|l| l.get(k)) == Some(val))
                });
                if labels_match {
                    found.push(Self::sandbox(item)?);
                }
            }
            cursor = v
                .get(cursor_key)
                .and_then(Value::as_str)
                .filter(|c| !c.is_empty())
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(found)
    }
}

/// Runs rooms in Daytona sandboxes.
pub struct DaytonaHost {
    pub api: Daytona,
    snapshot: Option<String>,
    bin_dir: PathBuf,
    /// A name for this lobby, so a clean-up never touches another lobby's rooms.
    lobby: String,
    /// Links are given this long, by room.
    lifetimes: Mutex<HashMap<String, u64>>,
    http: reqwest::Client,
}

impl DaytonaHost {
    pub fn new(settings: &DaytonaSettings, bin_dir: PathBuf, lobby: String) -> Self {
        Self {
            api: Daytona::new(settings),
            snapshot: settings.snapshot.clone(),
            bin_dir,
            lobby,
            lifetimes: Mutex::default(),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(8))
                .build()
                .expect("http client"),
        }
    }

    fn labels(&self) -> [(&'static str, &str); 2] {
        [("app", "duet"), ("lobby", &self.lobby)]
    }

    /// The room binary built for the sandbox's processor.
    fn binary(&self, machine: &str) -> Result<Vec<u8>, String> {
        let name = match machine.trim() {
            "x86_64" | "amd64" => "duet-room-x86_64",
            "aarch64" | "arm64" => "duet-room-aarch64",
            other => return Err(format!("no room binary for a {other:?} sandbox")),
        };
        let path = self.bin_dir.join(name);
        std::fs::read(&path).map_err(|e| {
            format!(
                "{}: {e}. Build it with scripts/build-room.sh",
                path.display()
            )
        })
    }

    pub async fn prepare(
        &self,
        room_id: &str,
        ttl_ms: u64,
        on: OnStage<'_>,
    ) -> Result<Ready, String> {
        let ttl_minutes = ttl_ms.div_ceil(60_000).max(2);
        on("create", false);
        let labels = self.labels();
        let created = self
            .api
            .create(
                &[labels[0], labels[1], ("room", room_id)],
                ttl_minutes,
                self.snapshot.as_deref(),
            )
            .await?;
        let id = created.id.clone();
        // From here on, any failure must not leave a sandbox behind.
        let built = async {
            let sb = self
                .api
                .wait_started(created, Duration::from_secs(60))
                .await?;
            on("create", true);

            on("move", false);
            let program = if self.snapshot.is_some() {
                "/usr/local/bin/duet-room".to_string()
            } else {
                let (_, machine) = self.api.exec(&sb, "uname -m", 15).await?;
                let (_, dir) = self.api.exec(&sb, "pwd", 15).await?;
                let path = format!("{}/duet-room", dir.trim().trim_end_matches('/'));
                self.api.upload(&sb, &path, self.binary(&machine)?).await?;
                let (code, out) = self.api.exec(&sb, &format!("chmod +x {path}"), 15).await?;
                if code != 0 {
                    return Err(format!(
                        "could not make the room program runnable: {}",
                        snippet(&out)
                    ));
                }
                path
            };
            on("move", true);

            on("start", false);
            let secret = token(24);
            let cmd = self
                .api
                .start(
                    &sb,
                    "room",
                    &format!("{program} --port {ROOM_PORT}"),
                    &[("DUET_ROOM_SECRET", secret.as_str())],
                )
                .await?;
            let expires_s = ttl_minutes * 60 + 60;
            let control = self.api.signed_url(&sb.id, ROOM_PORT, expires_s).await?;
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let up = self
                    .http
                    .get(format!("{control}/healthz"))
                    .header(SKIP_WARNING, "true")
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success());
                if up {
                    break;
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "the room program did not answer. It printed: {}",
                        snippet(&self.api.logs(&sb, "room", &cmd).await)
                    ));
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            on("start", true);
            self.lifetimes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(sb.id.clone(), expires_s);
            Ok(Ready {
                kind: "daytona",
                id: sb.id.clone(),
                secret,
                control: Some(control),
            })
        }
        .await;
        if built.is_err() {
            let _ = self.api.delete(&id).await;
        }
        built
    }

    fn control(ready: &Ready) -> Result<&str, String> {
        ready
            .control
            .as_deref()
            .ok_or_else(|| "this room has no address".to_string())
    }

    pub async fn arm(&self, ready: &Ready, cfg: RoomConfig) -> Result<[String; 2], String> {
        let control = Self::control(ready)?;
        let keys = [cfg.people[0].key.clone(), cfg.people[1].key.clone()];
        let reply = self
            .http
            .post(format!("{control}/internal/arm"))
            .header(SKIP_WARNING, "true")
            .header("x-duet-secret", &ready.secret)
            .json(&cfg)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("could not reach the room: {e}"))?;
        if !reply.status().is_success() {
            return Err(format!(
                "the room refused its plan: {} {}",
                reply.status(),
                snippet(&reply.text().await.unwrap_or_default())
            ));
        }
        // People reach the room through the lobby, which passes their requests on through the
        // control link (see `rooms::Proxied`): no link of their own is made here.
        Ok(keys.map(|k| format!("{control}/#k={k}")))
    }

    async fn read(&self, ready: &Ready, path: &str) -> Result<(u16, Value), String> {
        let control = Self::control(ready)?;
        let reply = self
            .http
            .get(format!("{control}{path}"))
            .header(SKIP_WARNING, "true")
            .header("x-duet-secret", &ready.secret)
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| format!("could not reach the room: {e}"))?;
        let status = reply.status().as_u16();
        let body = reply.text().await.unwrap_or_default();
        Ok((status, serde_json::from_str(&body).unwrap_or(Value::Null)))
    }

    pub async fn status(&self, ready: &Ready) -> Result<Value, String> {
        match self.read(ready, "/internal/status").await? {
            (200, v) if v.is_object() => Ok(v),
            (status, _) => Err(format!("the room answered {status}")),
        }
    }

    pub async fn export(&self, ready: &Ready) -> Result<Option<Value>, String> {
        match self.read(ready, "/internal/export").await? {
            (200, v) if v.is_object() => Ok(Some(v)),
            (403, _) => Ok(None),
            (status, _) => Err(format!("the room answered {status}")),
        }
    }

    pub async fn end(&self, ready: &Ready) {
        if let Ok(control) = Self::control(ready) {
            let _ = self
                .http
                .post(format!("{control}/internal/end"))
                .header(SKIP_WARNING, "true")
                .header("x-duet-secret", &ready.secret)
                .timeout(Duration::from_secs(8))
                .send()
                .await;
        }
    }

    pub async fn destroy(&self, ready: &Ready) -> Receipt {
        let started = Instant::now();
        let asked = self.api.delete(&ready.id).await;
        let ms = started.elapsed().as_millis() as u64;
        let gone = asked.is_ok() && self.api.wait_gone(&ready.id, Duration::from_secs(20)).await;
        self.lifetimes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&ready.id);
        Receipt {
            kind: "daytona".into(),
            id: ready.id.clone(),
            ms,
            gone,
        }
    }

    /// Deletes every sandbox this lobby left behind, such as after a crash. Returns how many.
    pub async fn sweep(&self) -> Result<usize, String> {
        let labels = self.labels();
        let left = self.api.list(&labels).await?;
        let mut removed = 0;
        for sb in left {
            if !matches!(sb.state.as_str(), "destroyed" | "destroying")
                && self.api.delete(&sb.id).await.is_ok()
            {
                removed += 1;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_bodies_are_read_in_every_shape() {
        assert_eq!(
            error_text(404, r#"{"statusCode":404,"message":"Sandbox not found"}"#),
            "Daytona answered 404: Sandbox not found"
        );
        assert_eq!(
            error_text(400, r#"{"error":"bad label"}"#),
            "Daytona answered 400: bad label"
        );
        assert_eq!(
            error_text(400, r#"{"errors":["no space","denied"]}"#),
            "Daytona answered 400: no space; denied"
        );
        assert_eq!(
            error_text(502, "<html>Bad Gateway</html>"),
            "Daytona answered 502: <html>Bad Gateway</html>"
        );
        assert_eq!(
            error_text(500, r#"{"message":{"nested":true}}"#),
            r#"Daytona answered 500: {"message":{"nested":true}}"#
        );
    }

    #[test]
    fn a_sandbox_is_read_from_its_json() {
        let d = Daytona::new(&DaytonaSettings {
            api_url: "https://x/api/".into(),
            key: "k".into(),
            ..Default::default()
        });
        assert_eq!(d.api, "https://x/api");
        let sb = Daytona::sandbox(
            &json!({"id": "abc", "state": "started", "toolboxProxyUrl": "https://proxy/toolbox/"}),
        )
        .unwrap();
        assert_eq!(sb.toolbox, "https://proxy/toolbox/abc");
        let bare = Daytona::sandbox(&json!({"id": "abc"})).unwrap();
        assert_eq!(
            (bare.state.as_str(), bare.toolbox.as_str()),
            ("unknown", "")
        );
        assert!(Daytona::sandbox(&json!({"state": "started"})).is_err());
    }

    #[test]
    fn a_new_api_sandbox_is_read_in_the_old_words() {
        let sb = Daytona::sandbox(&json!({"id": "sbx_1", "status": "running", "toolbox_proxy_url": "https://tb/sandboxes/"}))
            .unwrap();
        assert_eq!(
            (sb.state.as_str(), sb.toolbox.as_str()),
            ("started", "https://tb/sandboxes/sbx_1")
        );
        let templated = Daytona::sandbox(&json!({"id": "sbx_2", "status": "failed", "error": "no room", "toolbox_proxy_url": "https://tb/{sandbox_id}/x"}))
            .unwrap();
        assert_eq!(templated.toolbox, "https://tb/sbx_2/x");
        assert_eq!(
            (templated.state.as_str(), templated.error.as_deref()),
            ("error", Some("no room"))
        );
        for (new, old) in [
            ("deleted", "destroyed"),
            ("deleting", "destroying"),
            ("lost", "error"),
            ("booting", "starting"),
            ("queued", "creating"),
        ] {
            assert_eq!(old_state(new), old);
        }
    }

    #[test]
    fn log_frames_are_read_as_text() {
        let logs = json!({"frames": [{"channel": "stdout", "data": "x86_"}, {"channel": "stderr", "data": "warn"}, {"channel": "stdout", "data": "64\n"}]});
        assert_eq!(frames_text(&logs, true), "x86_64\n");
        assert_eq!(frames_text(&logs, false), "x86_warn64\n");
    }

    #[test]
    fn error_codes_from_the_new_api_are_shown() {
        assert_eq!(
            error_text(
                403,
                r#"{"code":"FEATURE_NOT_ENABLED","message":"Linux sandboxes are not enabled"}"#
            ),
            "Daytona answered 403: Linux sandboxes are not enabled (FEATURE_NOT_ENABLED)"
        );
    }
}
