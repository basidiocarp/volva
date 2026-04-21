use std::fmt::Write as _;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::Client;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use spore::logging::{SpanContext, request_span};
use tracing::warn;
use volva_core::{
    AuthMode, ExecutionSessionIdentity, ExecutionSessionState, OAUTH_BETA_HEADER_NAME,
    OAUTH_BETA_HEADER_VALUE, ResolvedCredential,
};

pub const DEFAULT_ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
pub const ANTHROPIC_API_VERSION: &str = "2023-06-01";
const DEFAULT_INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);
const DEFAULT_MAX_RETRY_DELAY: Duration = Duration::from_secs(8);
const DEFAULT_MAX_RETRIES: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiClientConfig {
    pub base_url: String,
    pub model: String,
}

impl Default for ApiClientConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_ANTHROPIC_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRequest {
    pub prompt: String,
    pub max_tokens: u32,
    pub session: ExecutionSessionIdentity,
}

impl ChatRequest {
    #[must_use]
    pub fn new(
        prompt: impl Into<String>,
        max_tokens: u32,
        session: ExecutionSessionIdentity,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            max_tokens,
            session,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatResponse {
    pub id: String,
    pub model: String,
    pub stop_reason: Option<String>,
    pub text: String,
    pub request_id: Option<String>,
    pub organization_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct MessagesRequestBody {
    model: String,
    max_tokens: u32,
    messages: Vec<MessageParam>,
}

#[derive(Debug, Serialize)]
struct MessageParam {
    role: &'static str,
    content: String,
}

#[derive(Debug, Deserialize)]
struct MessagesResponseBody {
    id: String,
    model: String,
    stop_reason: Option<String>,
    content: Vec<ContentBlock>,
}

#[derive(Debug, Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RateLimitSnapshot {
    requests_remaining: Option<String>,
    tokens_remaining: Option<String>,
    requests_reset: Option<String>,
    tokens_reset: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ResponseHeaderSnapshot {
    request_id: Option<String>,
    organization_id: Option<String>,
    rate_limits: RateLimitSnapshot,
}

#[must_use]
pub fn auth_header_kind(credential: &ResolvedCredential) -> &'static str {
    match credential.mode {
        AuthMode::ApiKey => "x-api-key",
        AuthMode::BearerToken => "authorization",
        _ => "authorization",
    }
}

pub async fn chat_once(
    config: &ApiClientConfig,
    credential: &ResolvedCredential,
    request: &ChatRequest,
) -> Result<ChatResponse> {
    chat_once_with_state_observer(config, credential, request, |_| {}).await
}

#[allow(clippy::too_many_lines)]
pub async fn chat_once_with_state_observer<F>(
    config: &ApiClientConfig,
    credential: &ResolvedCredential,
    request: &ChatRequest,
    mut on_state: F,
) -> Result<ChatResponse>
where
    F: FnMut(ExecutionSessionState),
{
    let base_span_context = SpanContext::for_app("volva")
        .with_tool("anthropic-api")
        .with_session_id(request.session.session_id.to_string())
        .with_workspace_root(request.session.workspace.workspace_root.clone());
    let _request_span = request_span("anthropic_chat", &base_span_context).entered();
    let client = Client::builder()
        .build()
        .context("failed to build Anthropic API client")?;
    let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));
    let body = MessagesRequestBody {
        model: config.model.clone(),
        max_tokens: request.max_tokens,
        messages: vec![MessageParam {
            role: "user",
            content: request.prompt.clone(),
        }],
    };

    let mut attempts = 0_u32;
    let mut delay = DEFAULT_INITIAL_RETRY_DELAY;

    let response = loop {
        attempts += 1;
        let mut http_request = client
            .post(&url)
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("content-type", "application/json")
            .header("accept", "application/json");

        http_request = match credential.mode {
            AuthMode::ApiKey => http_request.header("x-api-key", &credential.secret),
            AuthMode::BearerToken => http_request
                .header("authorization", format!("Bearer {}", credential.secret))
                .header(OAUTH_BETA_HEADER_NAME, OAUTH_BETA_HEADER_VALUE),
            _ => bail!(
                "unsupported auth mode `{}` for Anthropic API path",
                credential.mode
            ),
        };

        let response = http_request
            .json(&body)
            .send()
            .await
            .context("failed to send Anthropic messages request")?;
        let status = response.status();
        let response_headers = extract_response_headers(response.headers());

        if status.is_success() {
            break (response, response_headers);
        }

        if (status.as_u16() == 429 || status.as_u16() == 529) && attempts <= DEFAULT_MAX_RETRIES {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = response.text().await.unwrap_or_default();
            let wait = retry_after.unwrap_or(delay);
            let retry_span_context = response_headers.request_id.as_ref().map_or_else(
                || base_span_context.clone(),
                |request_id| {
                    base_span_context
                        .clone()
                        .with_request_id(request_id.clone())
                },
            );
            let _retry_span = request_span("anthropic_chat_retry", &retry_span_context).entered();
            let rate_limit_summary =
                summarize_rate_limits(&response_headers.rate_limits).unwrap_or_default();
            let response_summary = if body.is_empty() {
                String::new()
            } else {
                summarize_error_body(&body)
            };
            warn!(
                status = %status,
                attempt = attempts,
                max_attempts = DEFAULT_MAX_RETRIES,
                wait_secs = wait.as_secs(),
                organization_id = response_headers.organization_id.as_deref().unwrap_or("-"),
                rate_limits = if rate_limit_summary.is_empty() {
                    "-"
                } else {
                    rate_limit_summary.as_str()
                },
                response = if response_summary.is_empty() {
                    "-"
                } else {
                    response_summary.as_str()
                },
                "Anthropic request scheduled for retry",
            );
            on_state(ExecutionSessionState::Paused);
            tokio::time::sleep(wait).await;
            on_state(ExecutionSessionState::Resumed);
            delay = std::cmp::min(delay * 2, DEFAULT_MAX_RETRY_DELAY);
            continue;
        }

        let body = response.text().await.unwrap_or_default();
        bail!(
            "{}",
            format_api_error(status.as_u16(), &body, &response_headers)
        );
    };

    let (response, response_headers) = response;
    let payload = response
        .json::<MessagesResponseBody>()
        .await
        .context("failed to parse Anthropic messages response")?;
    Ok(ChatResponse {
        id: payload.id,
        model: payload.model,
        stop_reason: payload.stop_reason,
        text: extract_text(&payload.content),
        request_id: response_headers.request_id,
        organization_id: response_headers.organization_id,
    })
}

fn extract_text(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter(|block| block.block_type == "text")
        .filter_map(|block| block.text.as_deref())
        .collect::<Vec<_>>()
        .join("")
}

fn format_api_error(status: u16, body: &str, headers: &ResponseHeaderSnapshot) -> String {
    let base = if let Ok(error) = serde_json::from_str::<ApiErrorResponse>(body) {
        match status {
            401 => format!("Anthropic authentication failed: {}", error.error.message),
            429 => format_rate_limit_error(&error.error.message, &headers.rate_limits),
            529 => format!("Anthropic is overloaded: {}", error.error.message),
            _ => format!(
                "Anthropic request failed with {status}: {}",
                error.error.message
            ),
        }
    } else if body.is_empty() {
        format!("Anthropic request failed with {status}")
    } else {
        format!("Anthropic request failed with {status}: {body}")
    };

    append_response_diagnostics(base, headers)
}

fn summarize_error_body(body: &str) -> String {
    if let Ok(error) = serde_json::from_str::<ApiErrorResponse>(body) {
        return error.error.message;
    }

    body.to_string()
}

fn extract_rate_limit_snapshot(headers: &HeaderMap) -> RateLimitSnapshot {
    RateLimitSnapshot {
        requests_remaining: header_string(headers, "anthropic-ratelimit-requests-remaining"),
        tokens_remaining: header_string(headers, "anthropic-ratelimit-tokens-remaining"),
        requests_reset: header_string(headers, "anthropic-ratelimit-requests-reset"),
        tokens_reset: header_string(headers, "anthropic-ratelimit-tokens-reset"),
    }
}

fn extract_response_headers(headers: &HeaderMap) -> ResponseHeaderSnapshot {
    ResponseHeaderSnapshot {
        request_id: header_string(headers, "request-id"),
        organization_id: header_string(headers, "anthropic-organization-id"),
        rate_limits: extract_rate_limit_snapshot(headers),
    }
}

fn header_string(headers: &HeaderMap, key: &str) -> Option<String> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .map(ToOwned::to_owned)
}

fn summarize_rate_limits(snapshot: &RateLimitSnapshot) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(requests) = &snapshot.requests_remaining {
        let mut summary = format!("requests remaining={requests}");
        if requests == "0" {
            summary.push_str(" (request limit exhausted)");
        }
        if let Some(reset) = &snapshot.requests_reset {
            let _ = write!(summary, ", reset={reset}");
        }
        parts.push(summary);
    }
    if let Some(tokens) = &snapshot.tokens_remaining {
        let mut summary = format!("tokens remaining={tokens}");
        if tokens == "0" {
            summary.push_str(" (token limit exhausted)");
        }
        if let Some(reset) = &snapshot.tokens_reset {
            let _ = write!(summary, ", reset={reset}");
        }
        parts.push(summary);
    }

    (!parts.is_empty()).then(|| parts.join("; "))
}

fn append_response_diagnostics(base: String, headers: &ResponseHeaderSnapshot) -> String {
    let mut diagnostics = Vec::new();

    if let Some(request_id) = &headers.request_id {
        diagnostics.push(format!("request_id={request_id}"));
    }
    if let Some(organization_id) = &headers.organization_id {
        diagnostics.push(format!("organization_id={organization_id}"));
    }

    if diagnostics.is_empty() {
        base
    } else {
        format!("{base} ({})", diagnostics.join(", "))
    }
}

fn format_rate_limit_error(message: &str, snapshot: &RateLimitSnapshot) -> String {
    if let Some(summary) = summarize_rate_limits(snapshot) {
        return format!("Anthropic rate limit hit: {message}. {summary}");
    }

    format!("Anthropic rate limit hit: {message}")
}

#[cfg(test)]
mod tests {
    use super::{
        ChatResponse, ContentBlock, OAUTH_BETA_HEADER_VALUE, RateLimitSnapshot,
        ResponseHeaderSnapshot, extract_text, format_api_error, summarize_rate_limits,
    };

    #[test]
    fn extract_text_joins_text_blocks() {
        let blocks = vec![
            ContentBlock {
                block_type: "text".to_string(),
                text: Some("hello".to_string()),
            },
            ContentBlock {
                block_type: "tool_use".to_string(),
                text: None,
            },
            ContentBlock {
                block_type: "text".to_string(),
                text: Some(" world".to_string()),
            },
        ];

        assert_eq!(extract_text(&blocks), "hello world");
    }

    #[test]
    fn oauth_beta_header_matches_validated_auth_contract() {
        assert_eq!(OAUTH_BETA_HEADER_VALUE, "oauth-2025-04-20");
    }

    #[test]
    fn chat_response_shape_keeps_minimal_fields() {
        let response = ChatResponse {
            id: "msg_123".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            stop_reason: Some("end_turn".to_string()),
            text: "hello".to_string(),
            request_id: Some("req_123".to_string()),
            organization_id: Some("org_123".to_string()),
        };

        assert_eq!(response.id, "msg_123");
        assert_eq!(response.text, "hello");
        assert_eq!(response.request_id.as_deref(), Some("req_123"));
        assert_eq!(response.organization_id.as_deref(), Some("org_123"));
    }

    #[test]
    fn formats_structured_rate_limit_errors_cleanly() {
        let body = r#"{"type":"error","error":{"type":"rate_limit_error","message":"Error"}}"#;
        let headers = ResponseHeaderSnapshot {
            request_id: Some("req_123".to_string()),
            organization_id: Some("org_123".to_string()),
            rate_limits: RateLimitSnapshot {
                requests_remaining: Some("0".to_string()),
                tokens_remaining: Some("17".to_string()),
                requests_reset: Some("2026-04-02T12:00:00Z".to_string()),
                tokens_reset: None,
            },
        };

        assert_eq!(
            format_api_error(429, body, &headers),
            "Anthropic rate limit hit: Error. requests remaining=0 (request limit exhausted), reset=2026-04-02T12:00:00Z; tokens remaining=17 (request_id=req_123, organization_id=org_123)"
        );
    }

    #[test]
    fn summarizes_token_limit_exhaustion() {
        let rate_limits = RateLimitSnapshot {
            requests_remaining: Some("9".to_string()),
            tokens_remaining: Some("0".to_string()),
            requests_reset: None,
            tokens_reset: Some("2026-04-02T12:05:00Z".to_string()),
        };

        assert_eq!(
            summarize_rate_limits(&rate_limits),
            Some(
                "requests remaining=9; tokens remaining=0 (token limit exhausted), reset=2026-04-02T12:05:00Z".to_string()
            )
        );
    }

    #[test]
    fn formats_request_metadata_for_non_rate_limit_errors() {
        let body =
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"Bad model"}}"#;
        let headers = ResponseHeaderSnapshot {
            request_id: Some("req_456".to_string()),
            organization_id: Some("org_456".to_string()),
            rate_limits: RateLimitSnapshot {
                requests_remaining: None,
                tokens_remaining: None,
                requests_reset: None,
                tokens_reset: None,
            },
        };

        assert_eq!(
            format_api_error(400, body, &headers),
            "Anthropic request failed with 400: Bad model (request_id=req_456, organization_id=org_456)"
        );
    }
}
