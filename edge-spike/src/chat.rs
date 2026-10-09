//! OpenAI-compatible chat types shared by the live summarizer and judge.

use serde::{Deserialize, Serialize};

/// Thinking configuration for reasoning-capable models.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ThinkingConfig {
    Disabled,
}

/// OpenAI-compatible chat request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: u32,
    pub temperature: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
}

/// OpenAI-compatible chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

impl ChatMessage {
    /// Create a user message with content.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_owned(),
            content: Some(content.into()),
        }
    }
}

/// OpenAI-compatible chat response.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub choices: Vec<ChatChoice>,
}

impl ChatResponse {
    /// Extract the first message content.
    ///
    /// Null or empty content is a hard error, even when `finish_reason` is
    /// `"length"`. A truncated but non-empty response (`finish_reason ==
    /// "length"`) is accepted because partial content is still usable for
    /// downstream scoring; null/empty content is not.
    pub fn first_content(&self) -> anyhow::Result<String> {
        let choice = self
            .choices
            .first()
            .ok_or_else(|| anyhow::anyhow!("chat response has no choices"))?;
        let content = choice
            .message
            .content
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("chat response content is null"))?
            .trim()
            .to_owned();
        if content.is_empty() {
            anyhow::bail!("chat response content is empty");
        }
        Ok(content)
    }
}

/// One choice in an OpenAI-compatible chat response.
#[derive(Debug, Clone, Deserialize)]
pub struct ChatChoice {
    pub message: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
}

/// Convert a budget in characters to a rough token budget.
#[must_use]
pub fn chars_to_tokens(budget_chars: usize) -> u32 {
    // Conservative approximation: ~4 characters per token for English text.
    budget_chars
        .div_ceil(4)
        .clamp(1, 4096)
        .try_into()
        .unwrap_or(4096)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn sample_request(thinking: Option<ThinkingConfig>) -> ChatRequest {
        ChatRequest {
            model: "m".to_owned(),
            messages: vec![ChatMessage::user("hi")],
            max_tokens: 1,
            temperature: 0.0,
            thinking,
        }
    }

    #[test]
    fn thinking_field_is_absent_by_default() {
        let value = serde_json::to_value(sample_request(None)).unwrap();
        assert!(value.get("thinking").is_none(), "{value:?}");
    }

    #[test]
    fn thinking_disabled_serializes() {
        let value = serde_json::to_value(sample_request(Some(ThinkingConfig::Disabled))).unwrap();
        assert_eq!(value.get("thinking"), Some(&json!({"type": "disabled"})));
    }
}
