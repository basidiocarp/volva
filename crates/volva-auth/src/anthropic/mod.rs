pub mod account;
pub mod callback_server;
pub mod oauth;
pub mod pkce;

use std::time::Duration;

use crate::types::{AnthropicLoginRequest, AnthropicLoginResult, StoredAnthropicTokens};
use anyhow::Result;
use spore::logging::{SpanContext, workflow_span};
use uuid::Uuid;
use volva_core::AuthTarget;

use self::account::FinalizedAnthropicLogin;
use self::callback_server::CallbackServer;
use self::oauth::AuthorizationUrls;
use self::pkce::PkceParameters;

const DEFAULT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicLoginCompletion {
    pub result: AnthropicLoginResult,
    pub tokens: StoredAnthropicTokens,
}

#[derive(Debug)]
pub struct AnthropicLoginSession {
    request: AnthropicLoginRequest,
    pkce: PkceParameters,
    callback_server: CallbackServer,
    authorization_urls: AuthorizationUrls,
    browser_open_attempted: bool,
    correlation_id: String,
}

impl AnthropicLoginSession {
    pub async fn start(request: AnthropicLoginRequest) -> Result<Self> {
        let correlation_id = request
            .correlation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let span_context = auth_span_context("anthropic-auth", &correlation_id);
        let _session_span = workflow_span("anthropic_login_session_start", &span_context).entered();
        let pkce = PkceParameters::generate();
        let callback_server = CallbackServer::bind(request.target, correlation_id.clone()).await?;
        let authorization_urls = {
            let _url_span = workflow_span("anthropic_authorization_urls", &span_context).entered();
            oauth::authorization_urls(
                request.target,
                &pkce.code_challenge,
                &pkce.state,
                &callback_server.callback_url()?,
            )
        };
        let browser_open_attempted = if request.open_browser {
            let _browser_span = workflow_span("anthropic_browser_launch", &span_context).entered();
            oauth::try_open_browser(&authorization_urls.authorize, &span_context)
        } else {
            false
        };

        Ok(Self {
            request,
            pkce,
            callback_server,
            authorization_urls,
            browser_open_attempted,
            correlation_id,
        })
    }

    #[must_use]
    pub fn authorization_urls(&self) -> &AuthorizationUrls {
        &self.authorization_urls
    }

    pub fn callback_url(&self) -> Result<String> {
        self.callback_server.callback_url()
    }

    #[must_use]
    pub fn browser_open_attempted(&self) -> bool {
        self.browser_open_attempted
    }

    #[must_use]
    pub fn correlation_id(&self) -> &str {
        &self.correlation_id
    }

    pub async fn complete(self) -> Result<AnthropicLoginCompletion> {
        let span_context = auth_span_context("anthropic-auth", &self.correlation_id);
        let _complete_span =
            workflow_span("anthropic_login_session_complete", &span_context).entered();
        let callback_url = {
            let _callback_url_span =
                workflow_span("anthropic_callback_url", &span_context).entered();
            self.callback_server.callback_url()?
        };
        let callback = self
            .callback_server
            .wait_for_callback(&self.pkce.state, DEFAULT_CALLBACK_TIMEOUT)
            .await?;
        let token_response = {
            let _token_exchange_span =
                workflow_span("anthropic_token_exchange", &span_context).entered();
            oauth::exchange_code(
                &callback.code,
                &callback.state,
                &self.pkce.code_verifier,
                &callback_url,
                &span_context,
            )
            .await?
        };

        let api_key = match self.request.target {
            AuthTarget::ClaudeAi => None,
            AuthTarget::Console => {
                let _api_key_span =
                    workflow_span("anthropic_api_key_mint", &span_context).entered();
                Some(oauth::create_api_key(&token_response.access_token, &span_context).await?)
            }
            _ => unreachable!(
                "unsupported Anthropic login target: {}",
                self.request.target
            ),
        };

        let FinalizedAnthropicLogin { result, tokens } =
            account::finalize_login(self.request.target, &token_response, api_key)?;

        Ok(AnthropicLoginCompletion { result, tokens })
    }
}

pub async fn login(request: AnthropicLoginRequest) -> Result<AnthropicLoginCompletion> {
    AnthropicLoginSession::start(request)
        .await?
        .complete()
        .await
}

fn auth_span_context(tool: &str, correlation_id: &str) -> SpanContext {
    SpanContext::for_app("volva")
        .with_tool(tool)
        .with_session_id(correlation_id.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use volva_core::AuthTarget;

    use super::DEFAULT_CALLBACK_TIMEOUT;
    use crate::types::AnthropicLoginRequest;

    #[test]
    fn request_shape_still_supports_provider_flow_defaults() {
        let request = AnthropicLoginRequest {
            target: AuthTarget::ClaudeAi,
            open_browser: true,
            correlation_id: None,
        };

        assert!(request.open_browser);
        assert_eq!(DEFAULT_CALLBACK_TIMEOUT, Duration::from_secs(120));
    }
}
