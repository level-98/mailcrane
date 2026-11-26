//! Helping people spot phishing (M5-07): what the user's provider found
//! when it checked who sent a message, and From names that pretend to be
//! someone else.
//!
//! Only the `Authentication-Results` header the provider added is read
//! (RFC 8601 §5, the same rule one-click unsubscribe uses): it must name
//! the provider and sit above the point where the message entered it.
//! Results the sender wrote into the message themselves are ignored.
//! Nothing is looked up online.

use mailcrane_mime::auth::{summarize, trusted_results, AuthSummary};
use mailcrane_mime::ParsedMessage;

mod display_name;

pub use display_name::{display_name_warning, DisplayNameWarning};
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
    /// The From name shows an address or domain from another organisation
    /// than the real From address (rule C). Independent of `trusted`.
    pub display_name_warning: Option<DisplayNameWarning>,
}

/// Reads the trusted authentication results of `parsed` and checks the
/// display name of its first From address.
pub fn sender_check(parsed: &ParsedMessage) -> SenderCheck {
    let from = parsed.from.first();
    let from_domain = from
        .and_then(|a| a.email.rsplit_once('@'))
        .map(|(_, d)| d)
        .unwrap_or_default();
    let trusted = trusted_results(&parsed.trace);
    let auth = summarize(trusted.unwrap_or_default(), from_domain);
    SenderCheck {
        trusted: trusted.is_some(),
        verdict: trusted.map(|_| verdict(&auth)),
        auth,
        display_name_warning: from
            .and_then(|a| display_name_warning(a.name.as_deref().unwrap_or_default(), &a.email)),
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
