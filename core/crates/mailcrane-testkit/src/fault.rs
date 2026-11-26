use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    DropAfterCommands(usize),
    /// IMAP: the first command (on any connection) whose line contains this
    /// text, ignoring case, is carried out, then the connection closes
    /// without the reply, as if it dropped just then. Only once per server,
    /// so a retry works.
    DropAfterCommandOnce(String),
    /// IMAP: every command whose line contains `text`, ignoring case, is
    /// answered `NO` (a permanent refusal) and not carried out; with a
    /// `folder`, only while that folder is selected.
    RefuseCommand {
        folder: Option<String>,
        text: String,
    },
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
    /// JMAP only: refuse the first this many `EmailSubmission/set` creates
    /// with `forbiddenToSend` (RFC 8621 §7.5), after any `Email/import` in
    /// the same request has succeeded.
    RefuseSubmissions(usize),
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
    /// SMTP: advertise `FUTURERELEASE 604800 …` (RFC 4865) and accept
    /// `HOLDUNTIL` on `MAIL FROM`.
    FutureRelease,
    /// IMAP: `PERMANENTFLAGS` without `\*`, so custom keywords don't stick.
    NoCustomKeywords,
}
