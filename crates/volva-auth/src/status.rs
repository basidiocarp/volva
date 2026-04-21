use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use volva_core::{
    AuthCredentialSource, AuthMode, AuthProvider, AuthStatus, AuthTarget, ResolvedCredential,
};

use crate::storage::load_tokens;
use crate::types::StoredAnthropicTokens;

pub const ENV_API_KEY: &str = "ANTHROPIC_API_KEY";

pub fn auth_status(provider: AuthProvider) -> Result<AuthStatus> {
    let saved_tokens = load_tokens(provider)?;
    Ok(resolve_auth_status(
        provider,
        read_env_api_key(provider).as_deref(),
        saved_tokens.as_ref(),
        current_epoch_seconds(),
    ))
}

#[must_use]
pub fn resolve_auth_status(
    provider: AuthProvider,
    env_api_key: Option<&str>,
    saved_tokens: Option<&StoredAnthropicTokens>,
    now_epoch_seconds: u64,
) -> AuthStatus {
    let saved_credential = saved_tokens.map(|tokens| tokens.metadata(provider, now_epoch_seconds));

    if env_api_key.is_some_and(|api_key| !api_key.is_empty()) {
        return AuthStatus {
            provider,
            logged_in: true,
            active_credential_source: Some(AuthCredentialSource::EnvironmentApiKey),
            active_auth_mode: Some(AuthMode::ApiKey),
            saved_credential,
        };
    }

    let active_auth_mode = saved_tokens
        .and_then(|tokens| tokens.effective_credential(provider, now_epoch_seconds))
        .map(|credential| credential.mode);

    AuthStatus {
        provider,
        logged_in: active_auth_mode.is_some(),
        active_credential_source: active_auth_mode.map(|_| AuthCredentialSource::StoredCredential),
        active_auth_mode,
        saved_credential,
    }
}

#[must_use]
pub fn resolve_credential() -> Option<ResolvedCredential> {
    resolve_credential_for_provider(AuthProvider::Anthropic)
}

#[must_use]
pub fn resolve_credential_for_provider(provider: AuthProvider) -> Option<ResolvedCredential> {
    let now_epoch_seconds = current_epoch_seconds();
    if let Some(api_key) = read_env_api_key(provider) {
        return Some(ResolvedCredential {
            mode: AuthMode::ApiKey,
            secret: api_key,
            source: ENV_API_KEY.to_string(),
        });
    }

    load_tokens(provider)
        .ok()
        .flatten()
        .and_then(|tokens| tokens.effective_credential(provider, now_epoch_seconds))
}

#[must_use]
pub fn login_hint(provider: AuthProvider, target: AuthTarget) -> &'static str {
    match (provider, target) {
        (AuthProvider::Anthropic, AuthTarget::ClaudeAi) => {
            "Run `volva auth login anthropic` to start the Claude.ai OAuth flow."
        }
        (AuthProvider::Anthropic, AuthTarget::Console) => {
            "Run `volva auth login anthropic --console` to start the console OAuth flow."
        }
        _ => "Run `volva auth login` to authenticate.",
    }
}

fn read_env_api_key(provider: AuthProvider) -> Option<String> {
    match provider {
        AuthProvider::Anthropic => std::env::var(ENV_API_KEY)
            .ok()
            .filter(|api_key| !api_key.is_empty()),
        _ => None,
    }
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_prefers_env_api_key_over_saved_credentials() {
        let saved = StoredAnthropicTokens {
            access_token: "saved-access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(1_700_000_100),
            scopes: vec!["user:inference".to_string()],
            email: Some("saved@example.com".to_string()),
            organization_id: Some("org_123".to_string()),
            subscription_type: Some("pro".to_string()),
            api_key: None,
            target: AuthTarget::ClaudeAi,
        };

        let status = resolve_auth_status(
            AuthProvider::Anthropic,
            Some("env-anthropic-key"),
            Some(&saved),
            1_700_000_000,
        );

        assert_eq!(status.provider, AuthProvider::Anthropic);
        assert!(status.logged_in);
        assert_eq!(
            status.active_credential_source,
            Some(AuthCredentialSource::EnvironmentApiKey)
        );
        assert_eq!(status.active_auth_mode, Some(AuthMode::ApiKey));
        assert_eq!(
            status.saved_credential,
            Some(volva_core::StoredCredentialMetadata {
                provider: AuthProvider::Anthropic,
                target: AuthTarget::ClaudeAi,
                auth_mode: Some(AuthMode::BearerToken),
                email: Some("saved@example.com".to_string()),
                organization_id: Some("org_123".to_string()),
                subscription_type: Some("pro".to_string()),
                expires_at: Some(1_700_000_100),
                expired: false,
                has_refresh_token: true,
                has_api_key: false,
            })
        );
    }

    #[test]
    fn expired_saved_bearer_credentials_do_not_authenticate() {
        let saved = StoredAnthropicTokens {
            access_token: "saved-access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(100),
            scopes: vec!["user:inference".to_string()],
            email: Some("saved@example.com".to_string()),
            organization_id: None,
            subscription_type: None,
            api_key: None,
            target: AuthTarget::ClaudeAi,
        };

        let status = resolve_auth_status(AuthProvider::Anthropic, None, Some(&saved), 200);

        assert_eq!(
            status,
            AuthStatus {
                provider: AuthProvider::Anthropic,
                logged_in: false,
                active_credential_source: None,
                active_auth_mode: None,
                saved_credential: Some(volva_core::StoredCredentialMetadata {
                    provider: AuthProvider::Anthropic,
                    target: AuthTarget::ClaudeAi,
                    auth_mode: Some(AuthMode::BearerToken),
                    email: Some("saved@example.com".to_string()),
                    organization_id: None,
                    subscription_type: None,
                    expires_at: Some(100),
                    expired: true,
                    has_refresh_token: true,
                    has_api_key: false,
                }),
            }
        );
    }
}
