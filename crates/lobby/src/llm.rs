//! The model on Nosana's GPUs, reached through its OpenAI-compatible endpoint.
//!
//! Address, model and key are settings (`NOSANA_LLM_URL`, `NOSANA_LLM_MODEL`, `NOSANA_API_KEY`),
//! because at past events they were handed out on site. Every call has a time budget, and the
//! caller always has an answer of its own ready, so the model can only ever add to a plan.

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::sync::Mutex;

pub const DEFAULT_URL: &str = "https://inference.nosana.com/v1";

#[derive(Debug)]
pub enum LlmError {
    Timeout,
    Http(u16, String),
    Network(String),
    /// The model answered, but not with the JSON object it was asked for.
    NotJson(String),
    NoModel,
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmError::Timeout => write!(f, "the model did not answer in time"),
            LlmError::Http(status, body) => {
                write!(f, "the model endpoint answered {status}: {body}")
            }
            LlmError::Network(why) => write!(f, "could not reach the model endpoint: {why}"),
            LlmError::NotJson(text) => write!(f, "the model did not answer with JSON: {text}"),
            LlmError::NoModel => write!(
                f,
                "the endpoint lists no model that is being served right now"
            ),
        }
    }
}

pub struct Answer {
    pub json: Value,
    pub model: String,
    pub ms: u64,
}

pub struct Llm {
    http: reqwest::Client,
    url: String,
    key: Option<String>,
    /// The model named in settings, or the first one the endpoint says it is serving.
    model: Mutex<Option<String>>,
    /// Some endpoints refuse the switch that turns thinking off. Remembered after the first refusal.
    plain: std::sync::atomic::AtomicBool,
}

fn snippet(text: &str) -> String {
    text.chars().take(300).collect()
}

/// The model to chat with, from Nosana's list: one that can write (embedding models are listed
/// too, and cannot), preferring one a GPU is serving right now.
fn choose(models: &[Value]) -> Option<&str> {
    let writes = |m: &&Value| {
        let id = m.get("id").and_then(Value::as_str).unwrap_or("");
        let out = m.get("max_output_tokens").and_then(Value::as_u64);
        let id = id.to_lowercase();
        // Models that cannot hold a conversation are listed too.
        let other = ["embed", "rerank", "whisper", "tts", "clip", "guard"]
            .iter()
            .any(|w| id.contains(w));
        !other && out != Some(0)
    };
    let available = |m: &&Value| m.get("available").and_then(Value::as_bool) == Some(true);
    models
        .iter()
        .filter(writes)
        .find(available)
        .or_else(|| models.iter().find(writes))
        .and_then(|m| m.get("id"))
        .and_then(Value::as_str)
}

impl Llm {
    pub fn new(url: &str, model: Option<String>, key: Option<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(6))
                .user_agent(concat!("duet/", env!("CARGO_PKG_VERSION")))
                .build()
                .expect("http client"),
            url: url.trim_end_matches('/').to_string(),
            key: key.filter(|k| !k.is_empty()),
            model: Mutex::new(model.filter(|m| !m.is_empty())),
            plain: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.key {
            Some(key) => req.bearer_auth(key),
            None => req,
        }
    }

    /// Which model to call. Asked once, then remembered.
    pub async fn model(&self) -> Result<String, LlmError> {
        let mut slot = self.model.lock().await;
        if let Some(model) = slot.as_ref() {
            return Ok(model.clone());
        }
        let reply = self
            .auth(self.http.get(format!("{}/models", self.url)))
            .timeout(Duration::from_secs(8))
            .send()
            .await
            .map_err(|e| LlmError::Network(e.to_string()))?;
        let status = reply.status().as_u16();
        let body = reply.text().await.unwrap_or_default();
        if !(200..300).contains(&status) {
            return Err(LlmError::Http(status, snippet(&body)));
        }
        let list: Value =
            serde_json::from_str(&body).map_err(|_| LlmError::NotJson(snippet(&body)))?;
        let models = list
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let id = choose(&models).ok_or(LlmError::NoModel)?.to_string();
        *slot = Some(id.clone());
        Ok(id)
    }

    /// One question, one JSON object back, inside `budget`.
    pub async fn ask(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
        budget: Duration,
    ) -> Result<Answer, LlmError> {
        self.ask_at(system, user, max_tokens, budget, 0.6).await
    }

    /// The same, at a chosen temperature: 0 where the answer must be read, not written.
    pub async fn ask_at(
        &self,
        system: &str,
        user: &str,
        max_tokens: u32,
        budget: Duration,
        temperature: f32,
    ) -> Result<Answer, LlmError> {
        let started = Instant::now();
        let work = async {
            let model = self.model().await?;
            let mut last = LlmError::Timeout;
            for attempt in 0..3 {
                let mut body = json!({
                    "model": model,
                    "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
                    "temperature": temperature,
                    "max_tokens": max_tokens,
                    "stream": false,
                });
                if !self.plain.load(std::sync::atomic::Ordering::Relaxed) {
                    // Thinking costs seconds and tokens and is not needed for this.
                    body["chat_template_kwargs"] = json!({"enable_thinking": false});
                }
                let sent = self
                    .auth(self.http.post(format!("{}/chat/completions", self.url)))
                    .json(&body)
                    .send()
                    .await;
                let reply = match sent {
                    Ok(reply) => reply,
                    Err(e) => {
                        last = LlmError::Network(e.to_string());
                        tokio::time::sleep(Duration::from_millis(300 << attempt)).await;
                        continue;
                    }
                };
                let status = reply.status().as_u16();
                let text = reply.text().await.unwrap_or_default();
                if status == 400 && !self.plain.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    last = LlmError::Http(status, snippet(&text));
                    continue; // once more, without the thinking switch
                }
                if status == 429 || status >= 500 {
                    last = LlmError::Http(status, snippet(&text));
                    tokio::time::sleep(Duration::from_millis(400 << attempt)).await;
                    continue;
                }
                if status == 404 {
                    *self.model.lock().await = None; // gone: ask which models are served next time
                }
                if !(200..300).contains(&status) {
                    return Err(LlmError::Http(status, snippet(&text)));
                }
                let whole: Value = serde_json::from_str(&text)
                    .map_err(|_| LlmError::NotJson(format!("{} characters", text.len())))?;
                let content = whole
                    .pointer("/choices/0/message/content")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                // A reply cut off at the token limit is not mistaken for a whole one: an inner
                // object of it could otherwise pass for the answer.
                if whole
                    .pointer("/choices/0/finish_reason")
                    .and_then(Value::as_str)
                    == Some("length")
                {
                    return Err(LlmError::NotJson(format!(
                        "cut off at {max_tokens} tokens ({} characters)",
                        content.chars().count()
                    )));
                }
                return match extract_json(content) {
                    Some(json) => Ok(Answer { json, model, ms: 0 }),
                    // Only the size: the words may be someone's own, which are never kept.
                    None => Err(LlmError::NotJson(format!(
                        "{} characters",
                        content.chars().count()
                    ))),
                };
            }
            Err(last)
        };
        match tokio::time::timeout(budget, work).await {
            Ok(Ok(mut answer)) => {
                answer.ms = started.elapsed().as_millis() as u64;
                Ok(answer)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(LlmError::Timeout),
        }
    }
}

/// Finds the JSON object in a model's reply. Models wrap answers in thinking blocks, code fences
/// and friendly sentences; this takes the first balanced `{...}` that parses.
pub fn extract_json(reply: &str) -> Option<Value> {
    // Drop thinking, including a block that was cut off before it closed, and one whose opening
    // tag was in the prompt template, so only its closing tag is in the reply.
    let reply = match (reply.find("<think>"), reply.rfind("</think>")) {
        (None, Some(end)) => &reply[end + "</think>".len()..],
        _ => reply,
    };
    let mut text = String::with_capacity(reply.len());
    let mut rest = reply;
    while let Some(start) = rest.find("<think>") {
        text.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("</think>") {
            rest = &rest[start + end + "</think>".len()..];
        } else {
            rest = "";
            break;
        }
    }
    text.push_str(rest);
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(open) = text[from..].find('{').map(|i| i + from) {
        let (mut depth, mut in_string, mut escaped) = (0usize, false, false);
        for (i, &b) in bytes[open..].iter().enumerate() {
            if in_string {
                match b {
                    _ if escaped => escaped = false,
                    b'\\' => escaped = true,
                    b'"' => in_string = false,
                    _ => {}
                }
                continue;
            }
            match b {
                b'"' => in_string = true,
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(v @ Value::Object(_)) =
                            serde_json::from_str::<Value>(&text[open..=open + i])
                        {
                            return Some(v);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
        from = open + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_chosen_can_write() {
        // Nosana's list on 14 November 2026, with the embedding model put first.
        let listed = serde_json::json!([
            {"id": "nvidia/nemotron-3-embed-1b", "available": true, "max_output_tokens": 0},
            {"id": "old/offline-chat", "available": false},
            {"id": "qwen/qwen3.8-27b", "available": true, "max_output_tokens": 4096},
        ]);
        let models = listed.as_array().unwrap();
        assert_eq!(choose(models), Some("qwen/qwen3.8-27b"));
        assert_eq!(choose(&models[..2]), Some("old/offline-chat"));
        assert_eq!(choose(&models[..1]), None);
    }

    #[test]
    fn a_reranker_or_a_voice_model_is_not_chosen_to_chat() {
        let listed = serde_json::json!([
            {"id": "BAAI/bge-reranker-v2-m3", "available": true},
            {"id": "openai/whisper-large-v3", "available": true},
            {"id": "qwen/qwen3.8-27b", "available": true},
        ]);
        assert_eq!(choose(listed.as_array().unwrap()), Some("qwen/qwen3.8-27b"));
    }

    #[test]
    fn reasoning_without_an_opening_tag_is_dropped() {
        assert_eq!(
            extract_json("Maybe {\"a\": 0}.\n</think>\n{\"a\": 1}").unwrap()["a"],
            1
        );
        assert_eq!(
            extract_json("<think>draft {\"a\": 0}</think>{\"a\": 2}").unwrap()["a"],
            2
        );
    }

    #[test]
    fn plain_json_is_found() {
        assert_eq!(extract_json(r#"{"a": 1}"#).unwrap()["a"], 1);
    }

    #[test]
    fn thinking_fences_and_chatter_are_ignored() {
        let reply = "<think>maybe {\"a\": 0}? no</think>\nSure! Here it is:\n```json\n{\"a\": {\"b\": \"}{\\\"\"}, \"c\": [1, 2]}\n```\nHope that helps {smile}";
        let v = extract_json(reply).unwrap();
        assert_eq!(v["a"]["b"], "}{\"");
        assert_eq!(v["c"][1], 2);
    }

    #[test]
    fn a_broken_first_brace_does_not_hide_a_good_object() {
        assert_eq!(
            extract_json("{not json} then {\"ok\": true}").unwrap()["ok"],
            true
        );
    }

    #[test]
    fn nothing_usable_gives_nothing() {
        assert!(extract_json("").is_none());
        assert!(extract_json("no braces here").is_none());
        assert!(extract_json("{\"cut\": \"off").is_none());
        assert!(extract_json("<think>never closed {\"a\": 1}").is_none());
        assert!(
            extract_json("[1, 2, 3]").is_none(),
            "an array is not the object that was asked for"
        );
    }
}
