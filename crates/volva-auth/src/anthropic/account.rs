use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use volva_core::{AuthMode, AuthTarget};

use crate::types::{AnthropicLoginResult, StoredAnthropicTokens};

use super::oauth::{
    TokenExchangeResponse, normalize_scopes, provider_storage_path, uses_bearer_scope,
};

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicAccountPayload {
    #[serde(default)]
    pub uuid: Option<String>,
    #[serde(default)]
    pub email_address: Option<String>,
    #[serde(default)]
    pub subscription_type: Option<String>,
    #[serde(default)]
    pub subscription_tier: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AnthropicOrganizationPayload {
    #[serde(default)]
    pub uuid: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalizedAnthropicLogin {
    pub result: AnthropicLoginResult,
    pub tokens: StoredAnthropicTokens,
}

pub fn finalize_login(
    target: AuthTarget,
    token_response: TokenExchangeResponse,
    api_key: Option<String>,
) -> Result<FinalizedAnthropicLogin> {
    if token_response.access_token.is_empty() {
        bail!("Anthropic token exchange returned an empty access token");
    }

    let scopes = normalize_scopes(token_response.scope.as_deref());
    let account_email = token_response
        .account
        .as_ref()
        .and_then(|account| account.email_address.clone());
    let organization_id = token_response
        .organization
        .as_ref()
        .and_then(|organization| organization.uuid.clone());
    let subscription_type = token_response
        .account
        .as_ref()
        .and_then(resolve_subscription_type);

    let credential_mode = match target {
        AuthTarget::ClaudeAi => {
            if !uses_bearer_scope(&scopes) {
                bail!("Claude.ai login completed without the user:inference scope");
            }
            AuthMode::BearerToken
        }
        AuthTarget::Console => {
            if api_key.as_deref().is_none_or(str::is_empty) {
                bail!("Console login completed without a usable API key");
            }
            AuthMode::ApiKey
        }
    };

    let (stored_access_token, stored_refresh_token, stored_expires_at, stored_scopes) =
        if matches!(credential_mode, AuthMode::ApiKey) {
            (String::new(), None, None, scopes)
        } else {
            (
                token_response.access_token.clone(),
                token_response.refresh_token.clone(),
                token_response.expires_in.map(expires_at_epoch_seconds),
                scopes,
            )
        };

    Ok(FinalizedAnthropicLogin {
        result: AnthropicLoginResult {
            target,
            account_email: account_email.clone(),
            organization_id: organization_id.clone(),
            subscription_type: subscription_type.clone(),
            credential_mode,
            saved_path: provider_storage_path(),
        },
        tokens: StoredAnthropicTokens {
            access_token: stored_access_token,
            refresh_token: stored_refresh_token,
            expires_at: stored_expires_at,
            scopes: stored_scopes,
            email: account_email.clone(),
            organization_id: organization_id.clone(),
            subscription_type: subscription_type.clone(),
            api_key,
            target,
        },
    })
}

fn resolve_subscription_type(account: &AnthropicAccountPayload) -> Option<String> {
    account
        .subscription_type
        .clone()
        .or_else(|| account.subscription_tier.clone())
        .filter(|value| !value.is_empty())
}

fn expires_at_epoch_seconds(expires_in_seconds: u64) -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|error| anyhow!("system clock is before the unix epoch: {error}"))
        .unwrap_or_default()
        .as_secs();
    now.saturating_add(expires_in_seconds)
}

#[cfg(test)]
mod tests {
    use crate::anthropic::oauth::TokenExchangeResponse;

    use super::{AnthropicAccountPayload, AnthropicOrganizationPayload, finalize_login};
    use volva_core::{AuthMode, AuthTarget};

    fn bearer_response() -> TokenExchangeResponse {
        TokenExchangeResponse {
            access_token: "access-token".to_string(),
            token_type: Some("Bearer".to_string()),
            expires_in: Some(3600),
            refresh_token: Some("refresh-token".to_string()),
            scope: Some("user:profile user:inference".to_string()),
            account: Some(AnthropicAccountPayload {
                uuid: Some("acct-1".to_string()),
                email_address: Some("user@example.com".to_string()),
                subscription_type: Some("pro".to_string()),
                subscription_tier: None,
            }),
            organization: Some(AnthropicOrganizationPayload {
                uuid: Some("org-1".to_string()),
            }),
        }
    }

    #[test]
    fn claude_ai_login_requires_inference_scope() {
        let response = TokenExchangeResponse {
            scope: Some("user:profile".to_string()),
            ..bearer_response()
        };

        let error = finalize_login(AuthTarget::ClaudeAi, response, None)
            .expect_err("missing inference scope should fail");
        assert!(error.to_string().contains("user:inference"));
    }

    #[test]
    fn console_login_requires_api_key() {
        let response = TokenExchangeResponse {
            scope: Some("org:create_api_key user:profile".to_string()),
            ..bearer_response()
        };

        let error = finalize_login(AuthTarget::Console, response, None)
            .expect_err("missing API key should fail");
        assert!(error.to_string().contains("API key"));
    }

    #[test]
    fn finalized_login_carries_metadata_and_mode() {
        let finalized = finalize_login(AuthTarget::ClaudeAi, bearer_response(), None)
            .expect("bearer login should finalize");

        assert_eq!(finalized.result.credential_mode, AuthMode::BearerToken);
        assert_eq!(
            finalized.result.account_email.as_deref(),
            Some("user@example.com")
        );
        assert_eq!(finalized.result.organization_id.as_deref(), Some("org-1"));
        assert_eq!(finalized.result.subscription_type.as_deref(), Some("pro"));
        assert_eq!(finalized.tokens.email.as_deref(), Some("user@example.com"));
    }

    #[test]
    fn console_login_discards_oauth_bearer_secrets_before_storage() {
        let finalized = finalize_login(
            AuthTarget::Console,
            TokenExchangeResponse {
                scope: Some("org:create_api_key user:profile".to_string()),
                ..bearer_response()
            },
            Some("sk-ant-api".to_string()),
        )
        .expect("console login should finalize");

        assert_eq!(finalized.result.credential_mode, AuthMode::ApiKey);
        assert!(finalized.tokens.access_token.is_empty());
        assert_eq!(finalized.tokens.refresh_token, None);
        assert_eq!(finalized.tokens.expires_at, None);
        assert_eq!(
            finalized.tokens.scopes,
            vec!["org:create_api_key".to_string(), "user:profile".to_string()]
        );
        assert_eq!(finalized.tokens.api_key.as_deref(), Some("sk-ant-api"));
    }
}
