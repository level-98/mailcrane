//! Proof Key for Code Exchange (RFC 7636): the app proves at the token
//! endpoint that it's the one that started the authorization.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use super::OAuthError;

/// A code verifier (RFC 7636 §4.1): 43 characters from 32 random bytes of
/// the operating system's generator, base64url without padding. Held in
/// memory only; `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct CodeVerifier(String);

impl std::fmt::Debug for CodeVerifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeVerifier(…)")
    }
}

impl CodeVerifier {
    /// A fresh verifier from the OS random generator.
    pub fn random() -> Result<Self, OAuthError> {
        Ok(Self(random_token()?))
    }

    /// Wraps a known verifier (tests and the RFC's example).
    pub fn from_string(value: String) -> Self {
        Self(value)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `code_challenge` for `code_challenge_method=S256` (RFC 7636 §4.2):
    /// base64url(SHA-256(verifier)) without padding.
    pub fn challenge(&self) -> String {
        challenge_s256(&self.0)
    }
}

/// base64url(SHA-256(`verifier`)), RFC 7636 §4.2. Public so a fake token
/// endpoint can check what it's sent.
pub fn challenge_s256(verifier: &str) -> String {
    let digest = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest.as_ref())
}

/// 32 random bytes from the OS, base64url without padding: 43 URL-safe
/// characters. Used for verifiers and for `state`.
pub(crate) fn random_token() -> Result<String, OAuthError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|_| OAuthError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_7636_appendix_b_vector() {
        let v = CodeVerifier::from_string("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into());
        assert_eq!(v.challenge(), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn random_verifiers_are_valid_and_differ() {
        let a = CodeVerifier::random().expect("random");
        let b = CodeVerifier::random().expect("random");
        assert_ne!(a, b);
        // RFC 7636 §4.1: 43–128 characters from the unreserved set.
        assert_eq!(a.as_str().len(), 43);
        assert!(a
            .as_str()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)));
        assert_eq!(format!("{a:?}"), "CodeVerifier(…)");
    }
}
