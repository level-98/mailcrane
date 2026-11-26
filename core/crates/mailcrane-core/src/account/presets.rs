//! Providers that need an app-specific password for third-party apps (M3-05):
//! iCloud, Yahoo and AOL. Their settings are built in, so typing one of
//! their addresses needs no lookup, and setup shows short steps with a link
//! to the provider's own page for making an app password.
//!
//! The steps themselves are in the apps' strings files, so they can be
//! translated. The server settings and links here, and the steps, must be
//! checked with a real account for each provider (issue #47).

use super::setup::{Security, ServerConfig};

/// A provider with a built-in preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPasswordProvider {
    Icloud,
    Yahoo,
    Aol,
}

/// What setup needs to know about a provider with a preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub provider: AppPasswordProvider,
    /// The provider's name, as it calls itself.
    pub name: &'static str,
    pub imap: ServerConfig,
    pub smtp: ServerConfig,
    /// The provider's page for making an app password: opened in the
    /// system browser, never fetched by the app.
    pub app_password_url: &'static str,
}

impl Preset {
    /// The login for `email`. All three providers take the full address.
    pub fn username(&self, email: &str) -> String {
        email.trim().to_owned()
    }
}

const ICLOUD_DOMAINS: &[&str] = &["icloud.com", "me.com", "mac.com"];

/// Yahoo's own domains and its country domains. Yahoo Japan is a separate
/// company with its own mail service, so `yahoo.co.jp` isn't here.
const YAHOO_DOMAINS: &[&str] = &[
    "yahoo.com",
    "ymail.com",
    "rocketmail.com",
    "yahoo.at",
    "yahoo.be",
    "yahoo.ca",
    "yahoo.ch",
    "yahoo.co.id",
    "yahoo.co.in",
    "yahoo.co.nz",
    "yahoo.co.th",
    "yahoo.co.uk",
    "yahoo.co.za",
    "yahoo.com.ar",
    "yahoo.com.au",
    "yahoo.com.br",
    "yahoo.com.co",
    "yahoo.com.hk",
    "yahoo.com.mx",
    "yahoo.com.my",
    "yahoo.com.ph",
    "yahoo.com.sg",
    "yahoo.com.tr",
    "yahoo.com.tw",
    "yahoo.com.vn",
    "yahoo.de",
    "yahoo.dk",
    "yahoo.es",
    "yahoo.fi",
    "yahoo.fr",
    "yahoo.gr",
    "yahoo.ie",
    "yahoo.in",
    "yahoo.it",
    "yahoo.nl",
    "yahoo.no",
    "yahoo.pl",
    "yahoo.pt",
    "yahoo.ro",
    "yahoo.se",
];

const AOL_DOMAINS: &[&str] = &[
    "aol.com",
    "aim.com",
    "aol.co.uk",
    "aol.de",
    "aol.fr",
    "aol.it",
    "aol.es",
    "aol.nl",
    "aol.be",
    "aol.ca",
    "aol.com.au",
    "aol.com.br",
    "aol.com.mx",
];

fn tls(host: &str, port: u32) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        port,
        security: Security::Tls,
    }
}

fn starttls(host: &str, port: u32) -> ServerConfig {
    ServerConfig {
        host: host.to_owned(),
        port,
        security: Security::StartTls,
    }
}

/// The preset for `email`'s domain, if it has one. The domain must be one
/// of the listed ones exactly (ignoring case and a trailing dot): look-alikes
/// such as `notyahoo.com` or `yahoo.com.example.net` don't match.
pub fn preset_for(email: &str) -> Option<Preset> {
    let (local, domain) = email.trim().rsplit_once('@')?;
    if local.is_empty() {
        return None;
    }
    let domain = domain.trim_end_matches('.').to_ascii_lowercase();
    let listed = |list: &[&str]| list.contains(&domain.as_str());
    if listed(ICLOUD_DOMAINS) {
        Some(Preset {
            provider: AppPasswordProvider::Icloud,
            name: "iCloud Mail",
            imap: tls("imap.mail.me.com", 993),
            smtp: starttls("smtp.mail.me.com", 587),
            app_password_url: "https://account.apple.com/account/manage",
        })
    } else if listed(YAHOO_DOMAINS) {
        Some(Preset {
            provider: AppPasswordProvider::Yahoo,
            name: "Yahoo Mail",
            imap: tls("imap.mail.yahoo.com", 993),
            smtp: tls("smtp.mail.yahoo.com", 465),
            app_password_url: "https://login.yahoo.com/account/security",
        })
    } else if listed(AOL_DOMAINS) {
        Some(Preset {
            provider: AppPasswordProvider::Aol,
            name: "AOL Mail",
            imap: tls("imap.aol.com", 993),
            smtp: tls("smtp.aol.com", 465),
            app_password_url: "https://login.aol.com/account/security",
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests;
