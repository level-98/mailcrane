//! Sign in with Google (M3-02, #44): Gmail and Google Workspace accounts.
//!
//! An installed-app client (RFC 8252 §8.4: its id isn't a secret), PKCE, and
//! the full-mailbox scope `https://mail.google.com/`, which IMAP and SMTP
//! need for XOAUTH2. The client is the debug Android client registered for
//! package `app.mailcrane` with a custom URI scheme redirect; the release
//! client comes with the release signing key (#70).

use super::super::OAuthProvider;

/// The id stored with an account that signs in with Google.
pub const ID: &str = "google";

/// The Android client registered in Mailcrane's Google Cloud project.
pub const CLIENT_ID: &str =
    "968299964794-osjkaj7s31oglb2jua64jrri0cs1hvlf.apps.googleusercontent.com";

/// The redirect Google's Android clients allow: the package name as a
/// private-use scheme (RFC 8252 §7.1).
pub const REDIRECT_URI: &str = "app.mailcrane:/oauth2redirect";

/// The only scope IMAP and SMTP accept for XOAUTH2.
pub const SCOPE: &str = "https://mail.google.com/";

pub const IMAP_HOST: &str = "imap.gmail.com";
pub const IMAP_PORT: u16 = 993;
pub const SMTP_HOST: &str = "smtp.gmail.com";
pub const SMTP_PORT: u16 = 465;

/// Google's OAuth configuration. `None` only if a constant URL here failed
/// to parse, which the tests rule out.
pub fn provider() -> Option<OAuthProvider> {
    Some(OAuthProvider {
        id: ID.to_owned(),
        authorization_endpoint: url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")
            .ok()?,
        token_endpoint: url::Url::parse("https://oauth2.googleapis.com/token").ok()?,
        client_id: CLIENT_ID.to_owned(),
        scopes: vec![SCOPE.to_owned()],
        redirect_uri: REDIRECT_URI.to_owned(),
        // Always ask, so Google issues a refresh token again when someone
        // signs in a second time (after removing the account, for example).
        extra_auth_params: vec![("prompt".to_owned(), "consent".to_owned())],
    })
}

/// The servers and login for a Google account: Gmail's IMAP and SMTP with
/// implicit TLS, and the full address as the user name.
pub fn account_config(
    email: &str,
    display_name: &str,
) -> crate::account::setup::ManualAccountConfig {
    use crate::account::setup::{ManualAccountConfig, Security, ServerConfig};
    ManualAccountConfig {
        email: email.trim().to_owned(),
        display_name: display_name.trim().to_owned(),
        imap: ServerConfig {
            host: IMAP_HOST.to_owned(),
            port: u32::from(IMAP_PORT),
            security: Security::Tls,
        },
        smtp: ServerConfig {
            host: SMTP_HOST.to_owned(),
            port: u32::from(SMTP_PORT),
            security: Security::Tls,
        },
        username: email.trim().to_owned(),
    }
}

/// Whether `email` is a consumer Gmail address. Workspace accounts on
/// their own domain can sign in with Google too; this only decides when
/// setup suggests it first.
pub fn is_gmail_address(email: &str) -> bool {
    email
        .trim()
        .rsplit_once('@')
        .is_some_and(|(local, domain)| {
            !local.is_empty()
                && matches!(
                    domain.trim_end_matches('.').to_ascii_lowercase().as_str(),
                    "gmail.com" | "googlemail.com"
                )
        })
}

#[cfg(test)]
mod tests;
