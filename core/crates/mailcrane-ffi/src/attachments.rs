//! Saving attachments for the apps (M2-04).

use mailcrane_core::attachments::{self, AttachmentError};
use mailcrane_store::{MessageId, Store};

/// Why an attachment couldn't be saved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum AttachmentFailure {
    /// The message isn't downloaded, or has no such part.
    #[error("attachment not found")]
    NotFound,
    /// The file couldn't be written, or the database failed.
    #[error("storage failure")]
    Storage,
}

impl From<AttachmentError> for AttachmentFailure {
    fn from(e: AttachmentError) -> Self {
        match e {
            AttachmentError::NotFound | AttachmentError::Unreadable => Self::NotFound,
            AttachmentError::Io | AttachmentError::Store(_) => Self::Storage,
        }
    }
}

/// Writes attachment `part_id` (from `MessageDetail.attachments`) of a
/// downloaded message into `dest_dir` (which must exist) under a safe name,
/// never overwriting a file, and returns the file's path. Reads only the
/// database.
#[uniffi::export]
pub fn save_attachment(
    db_path: String,
    message_id: i64,
    part_id: String,
    dest_dir: String,
) -> Result<String, AttachmentFailure> {
    let store = Store::open(&db_path).map_err(|_| AttachmentFailure::Storage)?;
    let path = attachments::save_attachment(
        &store,
        MessageId(message_id),
        &part_id,
        std::path::Path::new(&dest_dir),
    )?;
    Ok(path.to_string_lossy().into_owned())
}

/// A safe file name made from a sender's (see the core's rules).
#[uniffi::export]
pub fn sanitize_filename(name: String) -> String {
    attachments::sanitize_filename(&name)
}
