use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    DropAfterCommands(usize),
    DropMidResponse,
    /// SMTP only: close the connection right after `354`, before the message ends.
    DropMidData,
    /// SMTP only: answer `RCPT TO` for exactly this address with `550`.
    RejectRecipient(String),
    Delay(Duration),
    MalformedResponse,
    RejectLogin,
    TempFailure,
    PermFailure,
    NoStartTls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quirk {
    GmailLabels,
    NoCondstore,
    NoQresync,
    NoIdle,
    NoMove,
    NoUidplus,
    NoSpecialUse,
}
