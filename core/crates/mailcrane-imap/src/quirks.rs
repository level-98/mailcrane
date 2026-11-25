//! Provider behaviour the app has to work around, each detected one way and
//! named so code and tests can refer to it (AGENTS.md §7).

use crate::command::Arg;
use crate::parser::{Code, StatusKind};
use crate::session::{has_cap, Session};
use crate::store_cmds::ChangeError;

/// A known provider behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerQuirk {
    /// The server files a copy of every message sent through its SMTP server
    /// in the Sent folder itself; uploading another would duplicate it.
    AutoSavesSent,
}

/// Microsoft's IMAP hosts. Outlook.com and Microsoft 365 save messages sent
/// through their SMTP servers to Sent Items themselves.
const MICROSOFT_HOSTS: &[&str] = &["outlook.office365.com", "imap-mail.outlook.com"];

/// The quirks of a server with these capabilities at `host`.
pub fn detect(capabilities: &[String], host: &str) -> Vec<ServerQuirk> {
    let mut quirks = Vec::new();
    // Gmail (it advertises X-GM-EXT-1) puts sent mail in "Sent Mail" itself.
    let gmail = has_cap(capabilities, "X-GM-EXT-1");
    let host = host.trim_end_matches('.');
    let microsoft = MICROSOFT_HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host));
    if gmail || microsoft {
        quirks.push(ServerQuirk::AutoSavesSent);
    }
    quirks
}

impl Session {
    /// The quirks of this server.
    pub fn quirks(&self) -> Vec<ServerQuirk> {
        detect(self.capabilities(), self.host())
    }

    /// `CREATE <name>` (RFC 9051 §6.3.4). A folder that already exists counts
    /// as created.
    pub async fn create_folder(&mut self, raw_name: &str) -> Result<(), ChangeError> {
        let done = self
            .run_command("CREATE", &[Arg::Str(raw_name.as_bytes())])
            .await?;
        let exists = matches!(&done.text.code, Some(Code::Other { name, .. })
            if name.eq_ignore_ascii_case("ALREADYEXISTS"));
        if done.status == StatusKind::Ok || exists {
            Ok(())
        } else {
            Err(ChangeError::Refused { transient: false })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(list: &[&str]) -> Vec<String> {
        list.iter().map(|c| (*c).to_owned()).collect()
    }

    #[test]
    fn gmail_and_microsoft_save_sent_mail_themselves() {
        assert_eq!(
            detect(&caps(&["IMAP4rev1", "X-GM-EXT-1"]), "imap.example.org"),
            [ServerQuirk::AutoSavesSent]
        );
        for host in [
            "outlook.office365.com",
            "IMAP-MAIL.OUTLOOK.COM",
            "outlook.office365.com.",
        ] {
            assert_eq!(
                detect(&caps(&["IMAP4rev1"]), host),
                [ServerQuirk::AutoSavesSent],
                "{host}"
            );
        }
        assert!(detect(&caps(&["IMAP4rev1"]), "imap.example.org").is_empty());
        // Only exact hosts: a lookalike doesn't count.
        assert!(detect(&caps(&[]), "outlook.office365.com.example.org").is_empty());
    }
}
