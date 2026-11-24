//! The SMTP envelope: who the message is from and who gets it.

use crate::error::SmtpError;

/// Sender and recipients as given to `MAIL FROM` and `RCPT TO`. Bcc
/// recipients belong in `to`; they are never put in the message headers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub from: String,
    pub to: Vec<String>,
}

/// Addresses go into SMTP commands, so anything that could end the command or
/// start a new one is refused.
pub(crate) fn check_address(address: &str) -> Result<(), SmtpError> {
    let bad = address.is_empty()
        || address.len() > 254
        || address
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '<' | '>'));
    if bad {
        Err(SmtpError::InvalidInput)
    } else {
        Ok(())
    }
}

impl Envelope {
    pub(crate) fn validate(&self) -> Result<(), SmtpError> {
        check_address(&self.from)?;
        if self.to.is_empty() {
            return Err(SmtpError::InvalidInput);
        }
        self.to.iter().try_for_each(|a| check_address(a))
    }
}
