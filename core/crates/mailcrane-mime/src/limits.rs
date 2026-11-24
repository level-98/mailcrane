//! Limits on what [`crate::parse_message`] accepts. Going over a limit is an
//! error; nothing is silently cut off.

/// Largest input accepted: 50 MiB.
pub const MAX_INPUT_BYTES: usize = 50 * 1024 * 1024;

/// Deepest nesting of multiparts and `message/rfc822` parts below the
/// top-level message.
pub const MAX_DEPTH: usize = 20;

/// Most MIME parts (multipart containers included) in one input.
pub const MAX_PARTS: usize = 1000;
