//! Flash summarizer implementations for Arm 3.
//!
//! Replay uses a head-budget fixture. Live mode calls an OpenAI-compatible
//! endpoint configured via `SUMMARIZER_BASE_URL`, `SUMMARIZER_API_KEY`, and
//! `SUMMARIZER_MODEL`.

use std::env;
use std::time::Duration;

use async_trait::async_trait;

use crate::chat::{ChatMessage, ChatRequest, ChatResponse, chars_to_tokens};
use crate::http_retry::execute_with_retry;

/// Summarize a capture region to fit the remaining budget.
#[async_trait]
pub trait FlashSummarizer: Send + Sync {
    /// Summarize `region` to at most `budget_chars` characters.
    async fn summarize(&self, region: &str, budget_chars: usize) -> anyhow::Result<String>;
}

/// Fixture summarizer: returns the head of the region up to the budget.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureSummarizer;

#[async_trait]
impl FlashSummarizer for FixtureSummarizer {
    async fn summarize(&self, region: &str, budget_chars: usize) -> anyhow::Result<String> {
        Ok(if region.chars().count() <= budget_chars {
            region.to_owned()
        } else {
            let mut out = region.chars().take(budget_chars).collect::<String>();
            out.push_str("... [summarized]");
            out
        })
    }
}

/// Configuration for the live summarizer.
#[derive(Debug, Clone)]
pub struct SummarizerConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub timeout: Duration,
    /// Optional hard cap on completion tokens.
    ///
    /// When unset, the cap is derived from the character budget but floored at
    /// 2048 tokens. Reasoning-capable models consume completion budget for
    /// `reasoning_content` before emitting visible `content`; a low cap based
    /// only on the desired output length starves the visible summary and
    /// produces `finish_reason == "length"` with empty content. The prompt
    /// itself asks for the target length, so the completion cap only needs to
    /// guarantee enough headroom for reasoning + visible text.
    pub max_tokens: Option<u32>,
}

impl SummarizerConfig {
    /// Default timeout for a single summarization call.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

    /// Minimum completion-token cap for reasoning models.
    ///
    /// zai `glm-5.3-flash` burned 187/200 tokens on `reasoning_content` in the
    /// board owner's probe, leaving visible content fragmented or empty. 2048
    /// gives a safety margin for similar reasoning-heavy models while the
    /// prompt still enforces the desired summary length in characters.
    pub const REASONING_FLOOR_TOKENS: u32 = 2048;

    /// Load from environment. Fails loud if any required variable is missing
    /// so the operator knows the live summarizer path is unset before spend.
    pub fn from_env() -> anyhow::Result<Self> {
        let base_url = env::var("SUMMARIZER_BASE_URL").map_err(|_| {
            anyhow::anyhow!("SUMMARIZER_BASE_URL is required for live Arm 3 (flash summarization)")
        })?;
        let api_key = env::var("SUMMARIZER_API_KEY").map_err(|_| {
            anyhow::anyhow!("SUMMARIZER_API_KEY is required for live Arm 3 (flash summarization)")
        })?;
        let model = env::var("SUMMARIZER_MODEL").map_err(|_| {
            anyhow::anyhow!("SUMMARIZER_MODEL is required for live Arm 3 (flash summarization)")
        })?;
        let timeout = env::var("SUMMARIZER_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .map(Duration::from_secs)
            .unwrap_or(Self::DEFAULT_TIMEOUT);
        let max_tokens = env::var("SUMMARIZER_MAX_TOKENS")
            .ok()
            .and_then(|s| s.parse().ok());
        Ok(Self {
            base_url,
            api_key,
            model,
            timeout,
            max_tokens,
        })
    }

    /// Compute the completion-token cap for a given character budget.
    #[must_use]
    pub fn max_tokens_for_budget(&self, budget_chars: usize) -> u32 {
        self.max_tokens
            .unwrap_or_else(|| chars_to_tokens(budget_chars).max(Self::REASONING_FLOOR_TOKENS))
    }
}

/// Live OpenAI-compatible summarizer.
#[derive(Debug, Clone)]
pub struct LiveSummarizer {
    client: reqwest::Client,
    config: SummarizerConfig,
}

impl LiveSummarizer {
    /// Create from config. No HTTP call is made here.
    pub fn new(config: SummarizerConfig) -> anyhow::Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|e| anyhow::anyhow!("failed to build summarizer HTTP client: {e}"))?;
        Ok(Self { client, config })
    }
}

#[async_trait]
impl FlashSummarizer for LiveSummarizer {
    async fn summarize(&self, region: &str, budget_chars: usize) -> anyhow::Result<String> {
        let request = ChatRequest {
            model: self.config.model.clone(),
            messages: vec![ChatMessage::user(format!(
                "Summarize the following evidence region in at most {budget_chars} characters, preserving facts needed to verify worker claims:\n\n{region}"
            ))],
            max_tokens: self.config.max_tokens_for_budget(budget_chars),
            temperature: 0.0,
        };

        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let req = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&request)
            .build()
            .map_err(|e| anyhow::anyhow!("summarizer failed to build request: {e}"))?;
        let response = execute_with_retry(&self.client, req, "summarizer").await?;

        let chat = response
            .json::<ChatResponse>()
            .await
            .map_err(|e| anyhow::anyhow!("summarizer response failed to parse: {e}"))?;
        chat.first_content()
            .map_err(|e| anyhow::anyhow!("summarizer response invalid: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{bearer_token, body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::exhaustion::is_exhaustion;

    fn test_config(base_url: String) -> SummarizerConfig {
        test_config_with_max_tokens(base_url, None)
    }

    fn test_config_with_max_tokens(base_url: String, max_tokens: Option<u32>) -> SummarizerConfig {
        SummarizerConfig {
            base_url,
            api_key: "test-key".to_owned(),
            model: "test-model".to_owned(),
            timeout: Duration::from_secs(5),
            max_tokens,
        }
    }

    fn ok_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": content}}]
        })
    }

    fn null_content_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{"message": {"role": "assistant", "content": null}}]
        })
    }

    fn reasoning_with_length_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": content,
                    "reasoning_content": "...thinking..."
                },
                "finish_reason": "length"
            }]
        })
    }

    fn empty_content_with_length_response() -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": "",
                    "reasoning_content": "...thinking..."
                },
                "finish_reason": "length"
            }]
        })
    }

    #[tokio::test]
    async fn happy_path_returns_summary() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(bearer_token("test-key"))
            .and(header("content-type", "application/json".to_owned()))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("short summary")))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let result = summarizer
            .summarize("some long evidence", 100)
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(result, "short summary");
    }

    #[tokio::test]
    async fn non_2xx_fails_loud() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let err = match summarizer.summarize("x", 10).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("HTTP 500"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn status_429_is_classified_as_exhaustion() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let err = match summarizer.summarize("x", 10).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("exhausted: 429"), "unexpected error: {err}");
        assert!(is_exhaustion(&anyhow::anyhow!("{err}")));
    }

    #[tokio::test]
    async fn null_content_surfaces_as_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(null_content_response()))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let err = match summarizer.summarize("x", 10).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("content is null"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn reasoning_with_length_and_short_content_succeeds() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(reasoning_with_length_response("short but usable summary")),
            )
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let result = summarizer
            .summarize("some evidence", 100)
            .await
            .unwrap_or_else(|e| panic!("{e}"));

        assert_eq!(result, "short but usable summary");
    }

    #[tokio::test]
    async fn empty_content_with_length_is_hard_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(empty_content_with_length_response()),
            )
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let err = match summarizer.summarize("x", 10).await {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };

        assert!(err.contains("content is empty"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn default_max_tokens_uses_reasoning_floor() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({"max_tokens": 2048})))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("ok")))
            .mount(&server)
            .await;

        let config = test_config(server.uri());
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        summarizer
            .summarize("x", 100)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    #[tokio::test]
    async fn max_tokens_override_is_sent_in_request_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(body_partial_json(serde_json::json!({"max_tokens": 512})))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_response("ok")))
            .mount(&server)
            .await;

        let config = test_config_with_max_tokens(server.uri(), Some(512));
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        summarizer
            .summarize("x", 100)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    #[tokio::test]
    #[ignore = "live probe against z.ai; run with SUMMARIZER_BASE_URL, SUMMARIZER_API_KEY, SUMMARIZER_MODEL in env"]
    async fn live_probe_zai_glm_5_3_flash_returns_non_empty_summary() {
        let config = SummarizerConfig::from_env().unwrap_or_else(|e| panic!("{e}"));
        let summarizer = LiveSummarizer::new(config).unwrap_or_else(|e| panic!("{e}"));
        let region = "The worker reported that the database migration completed at 14:23 UTC and that 1,247 rows were updated. The rollback window closed at 14:45 UTC with no errors reported. The incident commander approved the all-clear at 15:00 UTC.";
        let summary = summarizer
            .summarize(region, 200)
            .await
            .unwrap_or_else(|e| panic!("live probe failed: {e}"));
        assert!(
            !summary.trim().is_empty(),
            "live probe returned empty summary"
        );
        eprintln!("LIVE_PROBE_SUMMARY: {summary}");
    }
}
