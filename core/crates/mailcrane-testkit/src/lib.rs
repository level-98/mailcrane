//! Fake IMAP, SMTP, JMAP and ManageSieve servers for tests, able to misbehave on purpose.

#![forbid(unsafe_code)]

pub mod fault;
pub mod https;
pub mod imap;
pub mod jmap;
mod loopback;
pub mod mailbox;
pub mod oauth;
pub mod secrets;
pub mod sieve;
pub mod smtp;
mod tls;

pub use fault::{Fault, Quirk};
pub use https::{FakeHttps, HttpsRequest};
pub use imap::{FakeImap, FakeImapBuilder};
pub use jmap::{FakeEmail, FakeJmap, FakeMailbox, FakePush, FakePushSubscription, JmapSentRequest};
pub use mailbox::{Mailbox, Message};
pub use oauth::FakeTokenEndpoint;
pub use secrets::MemorySecretStore;
pub use sieve::{FakeSieve, FakeSieveBuilder, SieveFault};
pub use smtp::{FakeSmtp, SentMessage};
pub use tls::{DevCertificates, TestTls};

/// Start a fake IMAP server, deliver a synthetic message, and inspect the mailbox.
///
/// ```no_run
/// # #[tokio::main]
/// # async fn main() -> Result<(), String> {
/// use mailcrane_testkit::{FakeImap, Fault};
///
/// let imap = FakeImap::builder()
///     .user("alice@example.org", "password")
///     .folder("INBOX", Vec::new())
///     .fault(Fault::Delay(std::time::Duration::from_millis(10)))
///     .start()
///     .await?;
///
/// imap.mailbox()
///     .deliver("INBOX", b"From: sender@example.com\r\n\r\nHello".to_vec())
///     .await;
/// assert_eq!(imap.mailbox().exists("INBOX").await, 1);
/// imap.shutdown().await;
/// # Ok(())
/// # }
/// ```
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests;
