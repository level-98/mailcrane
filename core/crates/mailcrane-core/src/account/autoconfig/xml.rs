//! The Thunderbird autoconfig format (`clientConfig` version 1.1), as
//! served by providers at their well-known URLs.
//!
//! Only what account setup needs is read: IMAP incoming and SMTP outgoing
//! servers with host, port, socket type, username and authentication.
//! Entries with `socketType` plain are dropped here, so they can never be
//! offered.

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use super::{valid_host, Email};
use crate::account::setup::{Security, ServerConfig};

/// Largest autoconfig document read (M1-15).
pub const MAX_XML_BYTES: usize = 1024 * 1024;

/// Most servers of each kind kept from one document.
const MAX_SERVERS: usize = 20;

/// One server entry, after placeholders are filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlServer {
    pub config: ServerConfig,
    pub username: String,
}

/// What a document offers, in the document's order of preference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientConfig {
    pub provider_name: Option<String>,
    pub imap: Vec<XmlServer>,
    pub smtp: Vec<XmlServer>,
}

#[derive(Default)]
struct Entry {
    kind: String,
    hostname: String,
    port: String,
    socket_type: String,
    username: String,
    auth: Vec<String>,
    auth_current: String,
}

/// Fills in `%EMAILADDRESS%`, `%EMAILLOCALPART%` and `%EMAILDOMAIN%`.
fn fill(template: &str, email: &Email) -> String {
    template
        .replace("%EMAILADDRESS%", &email.address)
        .replace("%EMAILLOCALPART%", &email.local)
        .replace("%EMAILDOMAIN%", &email.domain)
}

impl Entry {
    /// The server, or `None` if it is plain text, malformed, or needs a
    /// login method we don't have (OAuth2, NTLM, GSSAPI, client
    /// certificates). Entries naming no method are taken as password.
    fn finish(self, email: &Email) -> Option<XmlServer> {
        let security = match self.socket_type.trim().to_ascii_uppercase().as_str() {
            "SSL" | "TLS" => Security::Tls,
            "STARTTLS" => Security::StartTls,
            _ => return None,
        };
        let password_ok = self.auth.is_empty()
            || self.auth.iter().any(|a| {
                matches!(
                    a.trim().to_ascii_lowercase().as_str(),
                    "password-cleartext" | "plain" | "secure"
                )
            });
        if !password_ok {
            return None;
        }
        let host = fill(self.hostname.trim(), email).to_ascii_lowercase();
        if !valid_host(&host) {
            return None;
        }
        let port: u32 = self
            .port
            .trim()
            .parse()
            .ok()
            .filter(|p| (1..=65535).contains(p))?;
        let username = fill(self.username.trim(), email);
        let username = if username.is_empty() || username.chars().any(char::is_control) {
            email.address.clone()
        } else {
            username
        };
        Some(XmlServer {
            config: ServerConfig {
                host,
                port,
                security,
                // Kept for the caller; checked again by setup's validate().
            },
            username,
        })
    }
}

/// Parses a document. Malformed XML is `None`; a document with no usable
/// servers is an empty [`ClientConfig`].
pub fn parse(xml: &[u8], email: &Email) -> Option<ClientConfig> {
    if xml.len() > MAX_XML_BYTES {
        return None;
    }
    let text = std::str::from_utf8(xml).ok()?;
    let mut reader = Reader::from_str(text);
    // Whitespace is kept so text split by entities stays intact; fields are
    // trimmed when used.
    // Doctype entities are never expanded; only the predefined ones are.
    let mut out = ClientConfig::default();
    let mut path: Vec<String> = Vec::new();
    let mut entry: Option<Entry> = None;
    let mut seen_root = false;
    let mut provider = String::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_owned();
                if path.is_empty() {
                    if name != "clientConfig" {
                        return None;
                    }
                    seen_root = true;
                }
                if path.len() > 32 {
                    return None;
                }
                if (name == "incomingServer" || name == "outgoingServer") && entry.is_none() {
                    let kind = e
                        .try_get_attribute("type")
                        .ok()
                        .flatten()
                        .and_then(|a| a.normalized_value(XmlVersion::Implicit1_0).ok())
                        .map(|v| v.to_ascii_lowercase())
                        .unwrap_or_default();
                    entry = Some(Entry {
                        kind,
                        ..Entry::default()
                    });
                }
                path.push(name);
            }
            Event::End(_) => {
                let name = path.pop()?;
                if name == "authentication" {
                    if let Some(e) = entry.as_mut() {
                        if e.auth.len() < 16 {
                            e.auth.push(std::mem::take(&mut e.auth_current));
                        }
                    }
                }
                if name == "incomingServer" || name == "outgoingServer" {
                    if let Some(done) = entry.take() {
                        let incoming = name == "incomingServer";
                        let wanted = if incoming { "imap" } else { "smtp" };
                        if done.kind == wanted {
                            if let Some(server) = done.finish(email) {
                                let list = if incoming {
                                    &mut out.imap
                                } else {
                                    &mut out.smtp
                                };
                                if list.len() < MAX_SERVERS {
                                    list.push(server);
                                }
                            }
                        }
                    }
                }
            }
            Event::Empty(_) => {}
            Event::Text(t) => {
                let value = t.xml10_content().into_owned();
                take_text(&path, &mut entry, &mut provider, &value);
            }
            Event::GeneralRef(r) => {
                // `&amp;` and friends arrive on their own events.
                let name: &str = &r;
                let ch = match name {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => return None,
                };
                take_text(&path, &mut entry, &mut provider, ch);
            }
            Event::CData(c) => {
                let value = c.into_inner().into_owned();
                take_text(&path, &mut entry, &mut provider, &value);
            }
            Event::DocType(_) => return None,
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_root || !path.is_empty() {
        return None;
    }
    let name: String = provider
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .take(128)
        .collect();
    out.provider_name = (!name.is_empty()).then_some(name);
    Some(out)
}

/// Appends text to whichever field the current element is.
fn take_text(path: &[String], entry: &mut Option<Entry>, provider: &mut String, value: &str) {
    let Some(leaf) = path.last() else { return };
    let parent = path.len().checked_sub(2).and_then(|i| path.get(i));
    match (entry.as_mut(), parent.map(String::as_str)) {
        (Some(e), Some("incomingServer" | "outgoingServer")) => {
            let field = match leaf.as_str() {
                "hostname" => &mut e.hostname,
                "port" => &mut e.port,
                "socketType" => &mut e.socket_type,
                "username" => &mut e.username,
                "authentication" => &mut e.auth_current,
                _ => return,
            };
            if field.len() < 1024 {
                field.push_str(value);
            }
        }
        (None, Some("emailProvider")) if leaf == "displayName" && provider.len() < 1024 => {
            provider.push_str(value);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Email {
        Email::parse("Alice.Smith@Example.org").expect("email")
    }

    #[test]
    fn imap_and_smtp_with_placeholders() {
        let c = parse(
            include_bytes!("../../../tests/fixtures/autoconfig/basic.xml"),
            &alice(),
        )
        .expect("parses");
        assert_eq!(c.provider_name.as_deref(), Some("Example Mail"));
        assert_eq!(c.imap.len(), 1);
        assert_eq!(c.imap[0].config.host, "imap.example.org");
        assert_eq!(c.imap[0].config.port, 993);
        assert_eq!(c.imap[0].config.security, Security::Tls);
        assert_eq!(c.imap[0].username, "Alice.Smith@example.org");
        assert_eq!(c.smtp[0].config.host, "smtp.example.org");
        assert_eq!(c.smtp[0].config.security, Security::StartTls);
        assert_eq!(c.smtp[0].username, "Alice.Smith");
    }

    #[test]
    fn several_servers_keep_their_order_and_plain_ones_are_dropped() {
        let c = parse(
            include_bytes!("../../../tests/fixtures/autoconfig/multiple.xml"),
            &alice(),
        )
        .expect("parses");
        let imap: Vec<(&str, u32)> = c
            .imap
            .iter()
            .map(|s| (s.config.host.as_str(), s.config.port))
            .collect();
        // The plain entry on 143, the POP3 entry and the OAuth2-only entry
        // are gone.
        assert_eq!(imap, [("imap.example.org", 993), ("mail.example.org", 143)]);
        assert_eq!(c.smtp.len(), 2);
        assert_eq!(c.smtp[1].config.security, Security::Tls);
    }

    #[test]
    fn plain_text_only_offers_nothing() {
        let c = parse(
            include_bytes!("../../../tests/fixtures/autoconfig/plain_only.xml"),
            &alice(),
        )
        .expect("parses");
        assert!(c.imap.is_empty() && c.smtp.is_empty());
    }

    #[test]
    fn malformed_or_hostile_documents_are_refused() {
        for bad in [
            &b"not xml"[..],
            b"<other/>",
            b"<clientConfig><emailProvider>",
            b"<!DOCTYPE x [<!ENTITY a \"aaaa\">]><clientConfig>&a;</clientConfig>",
        ] {
            assert_eq!(
                parse(bad, &alice()),
                None,
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let huge = vec![b' '; MAX_XML_BYTES + 1];
        assert_eq!(parse(&huge, &alice()), None);
    }

    #[test]
    fn bad_hosts_and_ports_are_dropped() {
        let xml = br#"<clientConfig version="1.1"><emailProvider id="x">
            <incomingServer type="imap"><hostname>imap.example.org/evil</hostname><port>993</port><socketType>SSL</socketType></incomingServer>
            <incomingServer type="imap"><hostname>imap.example.org</hostname><port>99999</port><socketType>SSL</socketType></incomingServer>
            <incomingServer type="imap"><hostname>ok.example.org</hostname><port>993</port><socketType>SSL</socketType></incomingServer>
            </emailProvider></clientConfig>"#;
        let c = parse(xml, &alice()).expect("parses");
        assert_eq!(c.imap.len(), 1);
        assert_eq!(c.imap[0].config.host, "ok.example.org");
    }

    #[test]
    fn entities_in_text_are_decoded() {
        let xml = br#"<clientConfig version="1.1"><emailProvider id="x">
            <displayName>Tom &amp; Jerry Mail</displayName></emailProvider></clientConfig>"#;
        let c = parse(xml, &alice()).expect("parses");
        assert_eq!(c.provider_name.as_deref(), Some("Tom & Jerry Mail"));
    }
}
