//! One-tap unsubscribe for the apps (M4-10). The app shows the button when
//! `unsubscribe_options` returns something, asks the user to confirm with
//! the method it will use, and only then calls `one_click_unsubscribe`, or
//! opens compose with the `mailto` target, or opens the web page.

use mailcrane_core::unsubscribe::{self, UnsubscribeError};
use mailcrane_store::{MessageId, Store};

/// Why unsubscribing failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum UnsubscribeFailure {
    /// One-click isn't offered for this message.
    #[error("one-click unsubscribe is not offered")]
    NotOffered,
    /// The sender's server refused (or redirected), with this HTTP status.
    #[error("the server answered {status}")]
    Refused {
        /// The HTTP status.
        status: u16,
    },
    /// No connection, no answer in time, or a bad certificate.
    #[error("network failure")]
    Network,
    /// The message or its body isn't stored, or the database failed.
    #[error("storage failure")]
    Storage,
}

impl From<UnsubscribeError> for UnsubscribeFailure {
    fn from(error: UnsubscribeError) -> Self {
        match error {
            UnsubscribeError::NotOffered => Self::NotOffered,
            UnsubscribeError::Refused(status) => Self::Refused { status },
            UnsubscribeError::Timeout | UnsubscribeError::Network | UnsubscribeError::Tls => {
                Self::Network
            }
            UnsubscribeError::UnknownMessage | UnsubscribeError::Store(_) => Self::Storage,
        }
    }
}

/// An email to prepare for unsubscribing. The user sends it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeMailto {
    pub address: String,
    pub subject: Option<String>,
    pub body: Option<String>,
}

/// How a message can be unsubscribed from, best first.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct UnsubscribeInfo {
    /// The sender's address, for "Unsubscribe from …?".
    pub sender: String,
    /// One-click is possible (the sender asks for it and the message is
    /// authenticated).
    pub one_click: bool,
    /// An email to prepare, if offered.
    pub mailto: Option<UnsubscribeMailto>,
    /// A web page to open after the user confirms, if offered.
    pub web_url: Option<String>,
}

/// The message's unsubscribe options, or `None` (none offered, or the body
/// isn't downloaded). Reads only the database.
#[uniffi::export]
pub fn unsubscribe_options(
    db_path: String,
    message_id: i64,
) -> Result<Option<UnsubscribeInfo>, UnsubscribeFailure> {
    let store = Store::open(&db_path).map_err(|_| UnsubscribeFailure::Storage)?;
    let options = unsubscribe::unsubscribe_options(&store, MessageId(message_id))?;
    Ok(options.map(|o| UnsubscribeInfo {
        sender: crate::messages::display_line(&o.sender),
        one_click: o.one_click.is_some(),
        mailto: o.mailto.map(|m| UnsubscribeMailto {
            address: m.address,
            subject: m.subject,
            body: m.body,
        }),
        web_url: o.web,
    }))
}

/// Sends the one-click unsubscribe request (RFC 8058) for the message.
/// Call only after the user confirmed. Blocking, at most about 15 seconds.
#[uniffi::export]
pub fn one_click_unsubscribe(db_path: String, message_id: i64) -> Result<(), UnsubscribeFailure> {
    // Never on the caller's stack (#462).
    crate::worker::on_job_thread(|| {
        let store = Store::open(&db_path).map_err(|_| UnsubscribeFailure::Storage)?;
        let runtime = crate::sync::runtime().map_err(|_| UnsubscribeFailure::Network)?;
        runtime.block_on(unsubscribe::one_click_unsubscribe(
            &store,
            MessageId(message_id),
            &[],
        ))?;
        Ok(())
    })
    .unwrap_or(Err(UnsubscribeFailure::Network))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::StoreError;

    #[test]
    fn errors_map() {
        assert_eq!(
            UnsubscribeFailure::from(UnsubscribeError::Refused(302)),
            UnsubscribeFailure::Refused { status: 302 }
        );
        assert_eq!(
            UnsubscribeFailure::from(UnsubscribeError::Tls),
            UnsubscribeFailure::Network
        );
        assert_eq!(
            UnsubscribeFailure::from(UnsubscribeError::Store(StoreError::Invalid { field: "x" })),
            UnsubscribeFailure::Storage
        );
    }

    #[test]
    fn a_missing_database_is_a_storage_error() {
        let dir = std::env::temp_dir().join("mailcrane-ffi-unsub-missing");
        let path = dir
            .join("nested")
            .join("m.db")
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            unsubscribe_options(path.clone(), 1),
            Err(UnsubscribeFailure::Storage)
        );
        assert_eq!(
            one_click_unsubscribe(path, 1),
            Err(UnsubscribeFailure::Storage)
        );
    }
}
