use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    DropAfterCommands(usize),
    DropMidResponse,
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
