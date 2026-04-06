use base64::Engine as _;
use sha2::{Digest, Sha256};

const RANDOM_BYTE_COUNT: usize = 32;
#[cfg(test)]
const MIN_RFC7636_VERIFIER_LEN: usize = 43;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkceParameters {
    pub code_verifier: String,
    pub code_challenge: String,
    pub state: String,
}

impl PkceParameters {
    #[must_use]
    pub fn generate() -> Self {
        let code_verifier = random_base64url();
        let code_challenge = code_challenge(&code_verifier);
        let state = random_base64url();

        Self {
            code_verifier,
            code_challenge,
            state,
        }
    }
}

#[must_use]
pub fn code_challenge(code_verifier: &str) -> String {
    let hash = Sha256::digest(code_verifier.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash)
}

fn random_base64url() -> String {
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();

    let mut bytes = [0_u8; RANDOM_BYTE_COUNT];
    bytes[..16].copy_from_slice(first.as_bytes());
    bytes[16..].copy_from_slice(second.as_bytes());

    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::{MIN_RFC7636_VERIFIER_LEN, PkceParameters, code_challenge};

    #[test]
    fn generated_verifier_is_rfc7636_shaped() {
        let pkce = PkceParameters::generate();

        assert!(
            pkce.code_verifier.len() >= MIN_RFC7636_VERIFIER_LEN,
            "expected verifier length >= {MIN_RFC7636_VERIFIER_LEN}",
        );
        assert!(
            pkce.code_verifier
                .chars()
                .all(|character| character.is_ascii_alphanumeric()
                    || character == '-'
                    || character == '_'),
            "expected verifier to be base64url encoded",
        );
    }

    #[test]
    fn challenge_is_derived_from_verifier() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = code_challenge(verifier);

        assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn generated_state_is_non_empty() {
        let pkce = PkceParameters::generate();

        assert!(!pkce.state.is_empty());
        assert_ne!(pkce.code_verifier, pkce.state);
    }
}
