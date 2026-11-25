//! Link cleaning and link checks for the apps.

use mailcrane_core::links::LinkWarning as CoreWarning;

/// Removes tracking parameters (`utm_*`, `fbclid` and others) from an `http`
/// or `https` link; anything else comes back unchanged. The apps call this
/// before opening a link when "Remove tracking from links" is on.
#[uniffi::export]
pub fn clean_url(input: String) -> String {
    mailcrane_core::links::clean_url(&input)
}

/// Why a link deserves a second look (M5-07). Hints only: the user can
/// always open the link.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum LinkWarning {
    /// Not an `http` or `https` link; never open it.
    UnsupportedScheme,
    /// A lookalike host mixing scripts; shown in punycode.
    MixedScript,
    /// The visible text names another site than the link goes to.
    TextHostMismatch {
        /// The host the text shows.
        text_host: String,
    },
    /// The host is an IP address.
    IpAddress,
    /// The URL has `user@` before the host.
    UserInfo,
    /// `http`, not `https`: "Not secure".
    Http,
    /// Longer than 2,048 bytes; the text was not compared.
    TooLong,
}

impl From<CoreWarning> for LinkWarning {
    fn from(w: CoreWarning) -> Self {
        match w {
            CoreWarning::UnsupportedScheme => Self::UnsupportedScheme,
            CoreWarning::MixedScript => Self::MixedScript,
            CoreWarning::TextHostMismatch { text_host } => Self::TextHostMismatch {
                text_host: crate::messages::one_line(&text_host),
            },
            CoreWarning::IpAddress => Self::IpAddress,
            CoreWarning::UserInfo => Self::UserInfo,
            CoreWarning::Http => Self::Http,
            CoreWarning::TooLong => Self::TooLong,
        }
    }
}

/// Where a link really goes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LinkCheck {
    /// The host to show, large: Unicode if it is safe to, else punycode.
    pub display_host: String,
    /// The host in ASCII (punycode) form.
    pub ascii_host: String,
    /// The link is `http`, not `https`.
    pub is_http: bool,
    /// What to warn about; empty means nothing.
    pub warnings: Vec<LinkWarning>,
}

/// Checks where a link goes. `text` is what the reader sees for it, when
/// known (from `MessageBody::Html.links`). Takes no network calls.
#[uniffi::export]
pub fn check_link(url: String, text: Option<String>) -> LinkCheck {
    let c = mailcrane_core::links::check_link(&url, text.as_deref());
    LinkCheck {
        display_host: crate::messages::one_line(&c.display_host),
        ascii_host: crate::messages::one_line(&c.ascii_host),
        is_http: c.is_http,
        warnings: c.warnings.into_iter().map(LinkWarning::from).collect(),
    }
}

/// The visible text of the link the reader tapped, from
/// `MessageBody::Html.links`. `url` is what the web view reported; it is
/// compared in canonical form, so a punycode or lowercased host still
/// matches the href as the sender wrote it. `None` when no link matches.
#[uniffi::export]
pub fn link_text_for(url: String, links: Vec<crate::messages::LinkText>) -> Option<String> {
    mailcrane_core::links::link_text_for(
        &url,
        links.iter().map(|l| (l.href.as_str(), l.text.as_str())),
    )
    .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_core() {
        assert_eq!(
            clean_url("https://example.com/?utm_source=x&a=1#f".into()),
            "https://example.com/?a=1#f"
        );
        assert_eq!(
            clean_url("mailto:a@example.com".into()),
            "mailto:a@example.com"
        );
    }

    #[test]
    fn check_link_reports_mismatch_and_hosts() {
        let c = check_link(
            "https://example.net/login".into(),
            Some("https://example.com/login".into()),
        );
        assert_eq!(c.display_host, "example.net");
        assert_eq!(
            c.warnings,
            vec![LinkWarning::TextHostMismatch {
                text_host: "example.com".into()
            }]
        );
        assert!(check_link("https://example.com/".into(), None)
            .warnings
            .is_empty());
    }

    #[test]
    fn link_text_matches_the_canonical_url() {
        let links = vec![crate::messages::LinkText {
            href: "https://пример.рф/Path?q=a b".into(),
            text: "пример.рф".into(),
        }];
        // What a web view reports for that href.
        let tapped = "https://xn--e1afmkfd.xn--p1ai/Path?q=a%20b";
        assert_eq!(
            link_text_for(tapped.into(), links.clone()).as_deref(),
            Some("пример.рф")
        );
        assert_eq!(link_text_for("https://example.org/".into(), links), None);
    }
}
