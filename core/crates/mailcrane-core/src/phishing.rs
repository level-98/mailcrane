//! Helping people spot phishing (M5-07): what the user's provider found
//! when it checked who sent a message.
//!
//! Only the `Authentication-Results` header the provider added is read
//! (RFC 8601 §5, the same rule one-click unsubscribe uses). Results the
//! sender wrote into the message themselves are ignored. Nothing is looked
//! up online.

use mailcrane_mime::auth::{provider_results, summarize, AuthSummary};
use mailcrane_mime::ParsedMessage;

pub use mailcrane_mime::auth::AuthResult;

/// What the authentication chip says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthVerdict {
    /// DMARC passed, or there is no DMARC result and an aligned DKIM
    /// signature passed.
    Verified,
    /// Neither verified nor failed.
    NotVerified,
    /// DMARC failed, or both DKIM and SPF failed.
    Failed,
}

/// The provider's authentication results for one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SenderCheck {
    /// Whether a header from the user's provider was found. When `false`,
    /// every result is [`AuthResult::None`] and there is no verdict.
    pub trusted: bool,
    /// SPF, DKIM and DMARC from the trusted header.
    pub auth: AuthSummary,
    /// What to show; `None` when `trusted` is `false` (show nothing).
    pub verdict: Option<AuthVerdict>,
}

/// Reads the trusted authentication results of `parsed`.
pub fn sender_check(parsed: &ParsedMessage) -> SenderCheck {
    let from_domain = parsed
        .from
        .first()
        .and_then(|a| a.email.rsplit_once('@'))
        .map(|(_, d)| d)
        .unwrap_or_default();
    let trusted = provider_results(
        &parsed.authentication_results,
        parsed.received_by.as_deref(),
    );
    let auth = summarize(trusted.unwrap_or_default(), from_domain);
    SenderCheck {
        trusted: trusted.is_some(),
        verdict: trusted.map(|_| verdict(&auth)),
        auth,
    }
}

/// The chip for a set of results (rule B4).
pub fn verdict(auth: &AuthSummary) -> AuthVerdict {
    use AuthResult::{Fail, None, Pass};
    if auth.dmarc == Pass || (auth.dmarc == None && auth.dkim == Pass && auth.dkim_aligned) {
        AuthVerdict::Verified
    } else if auth.dmarc == Fail || (auth.dkim == Fail && auth.spf == Fail) {
        AuthVerdict::Failed
    } else {
        AuthVerdict::NotVerified
    }
}

#[cfg(test)]
mod tests;
