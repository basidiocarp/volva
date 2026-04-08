use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use volva_core::{
    AuthMode, AuthProvider, AuthTarget, ResolvedCredential, StoredCredentialMetadata,
};

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
            && matches!(self.expires_at, Some(expires_at) if expires_at <= now_epoch_seconds)
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
