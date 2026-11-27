//! Repository functions, grouped by table.

mod accounts;
mod attachments;
mod contacts;
mod drafts;
mod folders;
mod jmap;
mod messages;
mod notify;
mod outbox;
mod pending_ops;
mod push;
mod rules;
mod search;
mod server_rules;
mod settings;
mod snooze;
mod threads;

pub use contacts::Correspondent;
pub use drafts::{Draft, DraftFields, DraftId, ServerCopy};
pub use jmap::{JmapRow, Protocol};
pub use notify::NewMail;
pub use outbox::{
    NewOutboxMessage, OutboxId, OutboxMessage, OutboxState, SaveSentCopy, ScheduleMode, SentCopy,
};
pub use pending_ops::{CancelReport, DroppedOp, NewOp, OpId, OpKind, OpMessage, PendingOp};
pub use push::PushSubscriptionRow;
pub use rules::{NewRule, Rule, RuleAction, RuleCondition, RuleId};
pub use search::SearchFilter;
pub use server_rules::{ServerRules, ServerRulesState};
pub use settings::{
    is_valid_authserv_id, SignatureSetting, ACCOUNT_COLOR_COUNT, MAX_AUTHSERV_ID_LEN,
    MAX_SIGNATURE_BYTES,
};
pub use snooze::{KeywordedMessage, NewSnooze, Snooze, SnoozeId, SnoozeState, Snoozing};
pub use threads::{ThreadSummary, UnthreadedMessage};

use crate::error::StoreError;

/// Converts a value read from the database into a narrower type, treating
/// out-of-range data as corruption rather than panicking.
pub(crate) fn narrow<T: TryFrom<i64>>(v: i64, column: &'static str) -> Result<T, StoreError> {
    T::try_from(v).map_err(|_| StoreError::Corrupt { column })
}
