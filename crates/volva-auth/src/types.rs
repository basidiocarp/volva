use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use volva_core::{
    AuthMode, AuthProvider, AuthTarget, ResolvedCredential, StoredCredentialMetadata,
};

/// Tokens within this many seconds of expiry are treated as already expired,
/// so a near-expiry credential is never handed to a live request (no refresh
/// flow exists yet to recover from a mid-flight 401).
const AUTH_EXPIRY_BUFFER_SECS: u64 = 300;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicLoginRequest {
    pub target: AuthTarget,
    pub open_browser: bool,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnthropicLoginResult {
    pub target: AuthTarget,
    pub account_email: Option<String>,
    pub organization_id: Option<String>,
    pub subscription_type: Option<String>,
    pub credential_mode: AuthMode,
    pub saved_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredAnthropicTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<u64>,
    #[serde(default)]
    pub scopes: Vec<String>,
    pub email: Option<String>,
    pub organization_id: Option<String>,
    pub subscription_type: Option<String>,
    pub api_key: Option<String>,
    #[serde(default = "default_auth_target")]
    pub target: AuthTarget,
}

impl StoredAnthropicTokens {
    #[must_use]
    pub fn uses_bearer_auth(&self) -> bool {
        self.scopes.iter().any(|scope| scope == "user:inference")
    }

    #[must_use]
    pub fn auth_mode(&self) -> Option<AuthMode> {
        if self.uses_bearer_auth() {
            return Some(AuthMode::BearerToken);
        }

        self.api_key
            .as_ref()
            .filter(|api_key| !api_key.is_empty())
            .map(|_| AuthMode::ApiKey)
    }

    #[must_use]
    pub fn is_expired_at(&self, now_epoch_seconds: u64) -> bool {
        matches!(self.auth_mode(), Some(AuthMode::BearerToken))
            && matches!(
                self.expires_at,
                Some(expires_at) if expires_at <= now_epoch_seconds.saturating_add(AUTH_EXPIRY_BUFFER_SECS)
            )
    }

    #[must_use]
    pub fn effective_credential(
        &self,
        provider: AuthProvider,
        now_epoch_seconds: u64,
    ) -> Option<ResolvedCredential> {
        match self.auth_mode() {
            Some(AuthMode::BearerToken) if !self.is_expired_at(now_epoch_seconds) => {
                Some(ResolvedCredential {
                    mode: AuthMode::BearerToken,
                    secret: self.access_token.clone(),
                    source: format!("saved-{provider}-oauth"),
                })
            }
            Some(AuthMode::ApiKey) => self.api_key.as_ref().map(|api_key| ResolvedCredential {
                mode: AuthMode::ApiKey,
                secret: api_key.clone(),
                source: format!("saved-{provider}-api-key"),
            }),
            _ => None,
        }
    }

    #[must_use]
    pub fn metadata(
        &self,
        provider: AuthProvider,
        now_epoch_seconds: u64,
    ) -> StoredCredentialMetadata {
        StoredCredentialMetadata {
            provider,
            target: self.target,
            auth_mode: self.auth_mode(),
            email: self.email.clone(),
            organization_id: self.organization_id.clone(),
            subscription_type: self.subscription_type.clone(),
            expires_at: self.expires_at,
            expired: self.is_expired_at(now_epoch_seconds),
            has_refresh_token: self
                .refresh_token
                .as_ref()
                .is_some_and(|refresh_token| !refresh_token.is_empty()),
            has_api_key: self
                .api_key
                .as_ref()
                .is_some_and(|api_key| !api_key.is_empty()),
        }
    }
}

const fn default_auth_target() -> AuthTarget {
    AuthTarget::ClaudeAi
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_token_within_buffer_is_expired() {
        let now = 1_000_000u64;
        let stored = StoredAnthropicTokens {
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(now + 299),
            scopes: vec!["user:inference".to_string()],
            email: None,
            organization_id: None,
            subscription_type: None,
            api_key: None,
            target: AuthTarget::ClaudeAi,
        };

        assert!(
            stored.is_expired_at(now),
            "token with 299s margin should be expired"
        );
    }

    #[test]
    fn bearer_token_outside_buffer_is_valid() {
        let now = 1_000_000u64;
        let stored = StoredAnthropicTokens {
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(now + 301),
            scopes: vec!["user:inference".to_string()],
            email: None,
            organization_id: None,
            subscription_type: None,
            api_key: None,
            target: AuthTarget::ClaudeAi,
        };

        assert!(
            !stored.is_expired_at(now),
            "token with 301s margin should be valid"
        );
    }

    #[test]
    fn bearer_token_at_exact_buffer_is_expired() {
        let now = 1_000_000u64;
        let stored = StoredAnthropicTokens {
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(now + 300),
            scopes: vec!["user:inference".to_string()],
            email: None,
            organization_id: None,
            subscription_type: None,
            api_key: None,
            target: AuthTarget::ClaudeAi,
        };
        assert!(
            stored.is_expired_at(now),
            "token at exactly the buffer boundary should be expired"
        );
    }

    #[test]
    fn api_key_not_expired_regardless_of_expiry_time() {
        let now = 1_000_000u64;
        let stored = StoredAnthropicTokens {
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(100),
            scopes: vec![],
            email: None,
            organization_id: None,
            subscription_type: None,
            api_key: Some("api-key".to_string()),
            target: AuthTarget::ClaudeAi,
        };

        assert!(
            !stored.is_expired_at(now),
            "api key should never be expired, even with past expires_at"
        );
    }

    #[test]
    fn no_auth_mode_not_expired() {
        let now = 1_000_000u64;
        let stored = StoredAnthropicTokens {
            access_token: "token".to_string(),
            refresh_token: None,
            expires_at: Some(100),
            scopes: vec![],
            email: None,
            organization_id: None,
            subscription_type: None,
            api_key: None,
            target: AuthTarget::ClaudeAi,
        };

        assert!(
            !stored.is_expired_at(now),
            "credential with no auth mode should not be expired"
        );
    }
}
