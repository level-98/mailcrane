//! Repository functions, grouped by table.

mod accounts;
mod attachments;
mod contacts;
mod drafts;
mod folders;
mod messages;
mod outbox;
mod pending_ops;
mod rules;
mod search;
mod settings;
mod snooze;
mod threads;

pub use contacts::Correspondent;
pub use drafts::{Draft, DraftFields, DraftId, ServerCopy};
pub use outbox::{
    NewOutboxMessage, OutboxId, OutboxMessage, OutboxState, SaveSentCopy, ScheduleMode, SentCopy,
};
pub use pending_ops::{OpId, OpKind, OpMessage, PendingOp};
pub use rules::{NewRule, Rule, RuleAction, RuleCondition, RuleId};
pub use search::SearchFilter;
pub use settings::{SignatureSetting, MAX_SIGNATURE_BYTES};
pub use snooze::{KeywordedMessage, NewSnooze, Snooze, SnoozeId, SnoozeState};
pub use threads::{ThreadSummary, UnthreadedMessage};

use crate::error::StoreError;

/// Converts a value read from the database into a narrower type, treating
/// out-of-range data as corruption rather than panicking.
pub(crate) fn narrow<T: TryFrom<i64>>(v: i64, column: &'static str) -> Result<T, StoreError> {
    T::try_from(v).map_err(|_| StoreError::Corrupt { column })
}
