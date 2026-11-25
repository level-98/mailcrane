//! A provider's OAuth configuration.

use super::OAuthError;

/// Where and how to sign in with one provider. Client ids for installed
/// apps aren't secret (RFC 8252 §8.4), so they live in the source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthProvider {
    /// A short name, such as `google`.
    pub id: String,
    /// RFC 6749 §3.1.
    pub authorization_endpoint: url::Url,
    /// RFC 6749 §3.2.
    pub token_endpoint: url::Url,
    pub client_id: String,
    pub scopes: Vec<String>,
    /// Where the browser comes back to: a private-use scheme based on the
    /// app id (RFC 8252 §7.1), such as `app.mailcrane:/oauth2redirect`.
    pub redirect_uri: String,
    /// Provider-specific authorization parameters (`prompt`, `access_type`…).
    pub extra_auth_params: Vec<(String, String)>,
}

impl OAuthProvider {
    /// Both endpoints must be `https:`; tokens never cross plain HTTP.
    pub(crate) fn check(&self) -> Result<(), OAuthError> {
        let https = |u: &url::Url| u.scheme() == "https" && u.host_str().is_some();
        if !https(&self.authorization_endpoint) || !https(&self.token_endpoint) {
            return Err(OAuthError::InvalidProvider);
        }
        if self.client_id.is_empty() || self.redirect_uri.is_empty() {
            return Err(OAuthError::InvalidProvider);
        }
        Ok(())
    }
}
