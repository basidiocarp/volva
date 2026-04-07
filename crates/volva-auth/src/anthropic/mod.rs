pub mod account;
pub mod callback_server;
pub mod oauth;
pub mod pkce;

use std::time::Duration;

use crate::types::{AnthropicLoginRequest, AnthropicLoginResult, StoredAnthropicTokens};
use anyhow::Result;
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
}

impl AnthropicLoginSession {
    pub async fn start(request: AnthropicLoginRequest) -> Result<Self> {
        let pkce = PkceParameters::generate();
        let callback_server = CallbackServer::bind(request.target).await?;
        let authorization_urls = oauth::authorization_urls(
            request.target,
            &pkce.code_challenge,
            &pkce.state,
            &callback_server.callback_url()?,
        );
        let browser_open_attempted =
            request.open_browser && oauth::try_open_browser(&authorization_urls.authorize);

        Ok(Self {
            request,
            pkce,
            callback_server,
            authorization_urls,
            browser_open_attempted,
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

    pub async fn complete(self) -> Result<AnthropicLoginCompletion> {
        let callback_url = self.callback_server.callback_url()?;
        let callback = self
            .callback_server
            .wait_for_callback(&self.pkce.state, DEFAULT_CALLBACK_TIMEOUT)
            .await?;
        let token_response = oauth::exchange_code(
            &callback.code,
            &callback.state,
            &self.pkce.code_verifier,
            &callback_url,
        )
        .await?;

        let api_key = match self.request.target {
            AuthTarget::ClaudeAi => None,
            AuthTarget::Console => Some(oauth::create_api_key(&token_response.access_token).await?),
            _ => unreachable!("unsupported Anthropic login target: {}", self.request.target),
        };

        let FinalizedAnthropicLogin { result, tokens } =
            account::finalize_login(self.request.target, token_response, api_key)?;

        Ok(AnthropicLoginCompletion { result, tokens })
    }
}

pub async fn login(request: AnthropicLoginRequest) -> Result<AnthropicLoginCompletion> {
    AnthropicLoginSession::start(request)
        .await?
        .complete()
        .await
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
        };

        assert!(request.open_browser);
        assert_eq!(DEFAULT_CALLBACK_TIMEOUT, Duration::from_secs(120));
    }
}
