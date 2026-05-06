pub mod anthropic;
pub mod api_key_resolver;
pub mod status;
pub mod storage;
pub mod types;

pub use anthropic::{AnthropicLoginCompletion, AnthropicLoginSession, login as login_anthropic};
pub use api_key_resolver::{ApiKeyResolver, AuthError};
pub use status::{
    ENV_API_KEY, auth_status, login_hint, resolve_auth_status, resolve_credential,
    resolve_credential_for_provider,
};
pub use storage::{
    auth_dir, clear_tokens, config_dir, load_tokens, provider_tokens_path, save_tokens,
};
pub use types::{AnthropicLoginRequest, AnthropicLoginResult, StoredAnthropicTokens};
