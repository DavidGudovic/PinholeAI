//! llama-server (llama.cpp) for the Describe tab: launch args and an
//! OpenAI-compatible chat call with one image as a base64 data URL.
//!
//! Describe sends only the fixed captioner instruction from the registry
//! (`captioner.prompts`). "Improve my prompt" also sends the user's prompt, to this
//! loopback server only; it is held in memory and never logged or written.
//!
//! Every launch gets a fresh random API key, passed in the environment
//! ([`API_KEY_ENV`], the env form of llama-server's `--api-key`, so it doesn't
//! show up in the process list) and sent as `Authorization: Bearer <key>`:
//! other local programs and web pages can't use the server. Only `/health`
//! stays public (llama.cpp `server-http.cpp`).

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

use crate::sdapi::ApiError;

/// Environment variable llama-server reads its API key from (`--api-key KEY`,
/// env `LLAMA_API_KEY`, see llama.cpp `common/arg.cpp` at the pinned tag).
pub const API_KEY_ENV: &str = "LLAMA_API_KEY";

/// Launch arguments (the pin's `launch_defaults` are added by the caller).
/// `backend == "cpu"` keeps every layer on the CPU; otherwise llama.cpp's
/// automatic fit decides how many layers go to the GPU.
pub fn launch_args(
    model: &Path,
    mmproj: &Path,
    port: u16,
    backend: &str,
    ctx_size: u32,
) -> Vec<String> {
    let mut a = vec![
        "-m".to_string(),
        model.to_string_lossy().into_owned(),
        "--mmproj".to_string(),
        mmproj.to_string_lossy().into_owned(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        port.to_string(),
        "-c".to_string(),
        ctx_size.to_string(),
    ];
    if backend == "cpu" {
        a.extend([
            "-ngl".to_string(),
            "0".to_string(),
            "--no-mmproj-offload".to_string(),
        ]);
    } else {
        a.extend(["-ngl".to_string(), "auto".to_string()]);
    }
    a
}

/// Sampling for "Improve my prompt" (see [`LlamaClient::rewrite`]).
static REWRITE_SAMPLING: std::sync::LazyLock<serde_json::Value> = std::sync::LazyLock::new(|| {
    serde_json::json!({
        "temperature": 0.6,
        "top_p": 0.9,
        "top_k": 40,
        "repeat_penalty": 1.1,
        "repeat_last_n": 64,
        "dry_multiplier": 0.8,
        "stop": ["\n\n"]
    })
});

#[derive(Clone)]
enum Http {
    Local(pinhole_net::LocalClient),
    #[cfg(any(test, feature = "test-util"))]
    Plain(reqwest::Client),
}

/// Client for one llama-server instance. No `Debug`: it may hold the API key.
#[derive(Clone)]
pub struct LlamaClient {
    http: Http,
    base: String,
    api_key: Option<String>,
}

impl LlamaClient {
    pub fn new(local: pinhole_net::LocalClient, base: impl Into<String>) -> Self {
        Self {
            http: Http::Local(local),
            base: base.into().trim_end_matches('/').to_string(),
            api_key: None,
        }
    }

    #[cfg(any(test, feature = "test-util"))]
    pub fn new_plain_for_tests(base: impl Into<String>) -> Self {
        let base = base.into().trim_end_matches('/').to_string();
        assert!(
            base.starts_with("http://127.0.0.1:"),
            "test client is loopback-only"
        );
        Self {
            http: Http::Plain(
                reqwest::Client::builder()
                    .no_proxy()
                    .build()
                    .expect("client"),
            ),
            base,
            api_key: None,
        }
    }

    /// Send `Authorization: Bearer <key>` with every request.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into()).filter(|k| !k.is_empty());
        self
    }

    fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.api_key {
            Some(k) => rb.bearer_auth(k),
            None => rb,
        }
    }

    fn get(&self, path: &str) -> Result<reqwest::RequestBuilder, ApiError> {
        let rb = match &self.http {
            Http::Local(c) => c
                .get(&self.base, path)
                .map_err(|e| ApiError::Net(e.to_string()))?,
            #[cfg(any(test, feature = "test-util"))]
            Http::Plain(c) => c.get(format!("{}{}", self.base, path)),
        };
        Ok(self.auth(rb))
    }

    fn post(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::RequestBuilder, ApiError> {
        let rb = match &self.http {
            Http::Local(c) => c
                .post_json(&self.base, path, body)
                .map_err(|e| ApiError::Net(e.to_string()))?,
            #[cfg(any(test, feature = "test-util"))]
            Http::Plain(c) => c.post(format!("{}{}", self.base, path)).json(body),
        };
        Ok(self.auth(rb))
    }

    /// Model ids from `GET /v1/models` (llama-server reports the `-m` path as
    /// given, unless an alias is set). Needs the API key.
    pub async fn model_ids(&self) -> Result<Vec<String>, ApiError> {
        let resp = self
            .get("/v1/models")?
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ApiError::Timeout
                } else if e.is_connect() {
                    ApiError::Connect
                } else {
                    ApiError::Net(e.without_url().to_string())
                }
            })?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(ApiError::Status {
                code: status,
                error: String::new(),
            });
        }
        let v: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ApiError::Decode(e.without_url().to_string()))?;
        Ok(v.get("data")
            .and_then(|d| d.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `GET /health` → 200 once the model is loaded (503 while loading).
    pub async fn is_ready(&self) -> bool {
        let Ok(rb) = self.get("/health") else {
            return false;
        };
        matches!(rb.timeout(Duration::from_secs(5)).send().await, Ok(r) if r.status().is_success())
    }

    /// Ask the VLM to describe one image. Returns the trimmed answer.
    pub async fn describe(
        &self,
        instruction: &str,
        mime: &str,
        image: &[u8],
        max_tokens: u32,
    ) -> Result<String, ApiError> {
        use base64::Engine as _;
        let data_url = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(image)
        );
        let messages = serde_json::json!([{
            "role": "user",
            "content": [
                { "type": "image_url", "image_url": { "url": data_url } },
                { "type": "text", "text": instruction }
            ]
        }]);
        self.chat(
            messages,
            max_tokens,
            serde_json::json!({ "temperature": 0.2 }),
        )
        .await
    }

    /// Text in, text out: `system` instruction plus the user's `text` as the chat message.
    pub async fn rewrite(
        &self,
        system: &str,
        text: &str,
        max_tokens: u32,
    ) -> Result<String, ApiError> {
        let messages = serde_json::json!([
            { "role": "system", "content": system },
            { "role": "user", "content": text }
        ]);
        // A 3B model with greedy-ish sampling and no penalty loops ("bedroom, bedroom, …") on
        // one-word ideas: moderate temperature, a repeat penalty and llama.cpp's DRY sampler.
        self.chat(messages, max_tokens, REWRITE_SAMPLING.clone())
            .await
    }

    async fn chat(
        &self,
        messages: serde_json::Value,
        max_tokens: u32,
        sampling: serde_json::Value,
    ) -> Result<String, ApiError> {
        let mut body = serde_json::json!({
            "messages": messages,
            "max_tokens": max_tokens,
            "stream": false,
            "cache_prompt": false
        });
        if let (Some(b), Some(s)) = (body.as_object_mut(), sampling.as_object()) {
            b.extend(s.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        let resp = self
            .post("/v1/chat/completions", &body)?
            .timeout(Duration::from_secs(10 * 60))
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ApiError::Timeout
                } else if e.is_connect() {
                    ApiError::Connect
                } else {
                    ApiError::Net(e.without_url().to_string())
                }
            })?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| ApiError::Net(e.without_url().to_string()))?;
        if !(200..300).contains(&status) {
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            let msg = v
                .pointer("/error/message")
                .and_then(|m| m.as_str())
                .or_else(|| v.get("error").and_then(|m| m.as_str()))
                .unwrap_or("")
                .chars()
                .take(200)
                .collect();
            return Err(ApiError::Status {
                code: status,
                error: msg,
            });
        }
        #[derive(Deserialize)]
        struct Msg {
            #[serde(default)]
            content: Option<String>,
        }
        #[derive(Deserialize)]
        struct Choice {
            message: Msg,
        }
        #[derive(Deserialize)]
        struct Resp {
            choices: Vec<Choice>,
        }
        let r: Resp = serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::Decode(e.to_string().chars().take(200).collect()))?;
        let text = r
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .unwrap_or_default();
        Ok(clean_caption(&text))
    }
}

/// Strip chat-template leftovers / quotes / "Prompt:" preambles.
pub fn clean_caption(s: &str) -> String {
    let mut t = s.trim();
    for prefix in ["Prompt:", "prompt:", "Tags:", "tags:", "Description:"] {
        if let Some(rest) = t.strip_prefix(prefix) {
            t = rest.trim_start();
        }
    }
    let t = t.trim_matches(|c| c == '"' || c == '`' || c == '\'').trim();
    t.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_args_bind_loopback_and_offload() {
        let a = launch_args(
            Path::new("/m/model.gguf"),
            Path::new("/m/mmproj.gguf"),
            4321,
            "vulkan",
            8192,
        );
        assert!(a.windows(2).any(|w| w == ["--host", "127.0.0.1"]));
        assert!(a.windows(2).any(|w| w == ["--port", "4321"]));
        assert!(a.windows(2).any(|w| w == ["--mmproj", "/m/mmproj.gguf"]));
        assert!(a.windows(2).any(|w| w == ["-ngl", "auto"]));
        let c = launch_args(Path::new("m"), Path::new("p"), 1, "cpu", 4096);
        assert!(c.windows(2).any(|w| w == ["-ngl", "0"]));
    }

    #[test]
    fn cleans_captions() {
        assert_eq!(
            clean_caption("  Prompt: \"a cat on a mat\"  "),
            "a cat on a mat"
        );
        assert_eq!(clean_caption("1girl, solo"), "1girl, solo");
    }
}
