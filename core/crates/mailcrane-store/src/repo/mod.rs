//! Repository functions, grouped by table.

mod accounts;
mod attachments;
mod folders;
mod messages;
mod pending_ops;

pub use pending_ops::{OpId, OpKind, OpMessage, PendingOp};

use crate::error::StoreError;

/// Converts a value read from the database into a narrower type, treating
/// out-of-range data as corruption rather than panicking.
pub(crate) fn narrow<T: TryFrom<i64>>(v: i64, column: &'static str) -> Result<T, StoreError> {
    T::try_from(v).map_err(|_| StoreError::Corrupt { column })
}
