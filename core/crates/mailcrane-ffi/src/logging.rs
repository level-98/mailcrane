//! Forwards the core's warnings and errors to the app's log (logcat on
//! Android), so a failed sync on a real device says which step failed.
//!
//! Only `WARN` and `ERROR` events from our own crates (`mailcrane_*`
//! targets) are forwarded; anything from a dependency is dropped. The sink
//! trusts our call sites: every `warn!`/`error!` in the core logs only kinds,
//! variant names, fixed command labels, ids and counts, never server text,
//! message content or credentials (AGENTS.md §4.4). Review new call sites
//! with that in mind.

use std::fmt::Write as _;
use std::sync::Arc;

use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// Longest line handed to the sink, in bytes. Longer lines are cut.
const MAX_LINE: usize = 1024;

/// How serious a forwarded log line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LogLevel {
    /// Something failed but the app carries on, e.g. a sync.
    Warn,
    /// Something failed that should not have.
    Error,
}

/// Receives the core's log lines, implemented in Kotlin and Swift. May be
/// called from any thread and must not block.
#[uniffi::export(with_foreign)]
pub trait LogSink: Send + Sync {
    /// One line, e.g. `mailcrane_core::sync::initial: sync failed
    /// stage=sync kind=Protocol error=CommandFailed command=SELECT`.
    fn log(&self, level: LogLevel, line: String);
}

/// Sends the core's warnings and errors to `sink` from now on. Call once at
/// start-up. Returns `false`, and changes nothing, if a sink is already
/// installed. Never panics.
#[uniffi::export]
pub fn install_log_sink(sink: Arc<dyn LogSink>) -> bool {
    tracing::subscriber::set_global_default(SinkSubscriber { sink }).is_ok()
}

/// Whether events from `target` come from our own code.
fn is_ours(target: &str) -> bool {
    target == "mailcrane" || target.starts_with("mailcrane_")
}

/// The level an event is forwarded at, if it is forwarded at all.
fn forwarded(meta: &Metadata<'_>) -> Option<LogLevel> {
    if !is_ours(meta.target()) {
        return None;
    }
    match *meta.level() {
        Level::ERROR => Some(LogLevel::Error),
        Level::WARN => Some(LogLevel::Warn),
        _ => None,
    }
}

struct SinkSubscriber {
    sink: Arc<dyn LogSink>,
}

/// Collects an event's message and fields into one line.
#[derive(Default)]
struct Line {
    message: String,
    fields: String,
}

impl Visit for Line {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={}", field.name(), value);
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={:?}", field.name(), value);
        }
    }
}

/// `line` cut to at most `max` bytes, on a character boundary.
fn truncated(mut line: String, max: usize) -> String {
    if line.len() > max {
        let mut end = max;
        while !line.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        line.truncate(end);
    }
    line
}

impl Subscriber for SinkSubscriber {
    fn enabled(&self, meta: &Metadata<'_>) -> bool {
        forwarded(meta).is_some()
    }

    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::WARN)
    }

    // Spans are never forwarded; they only need a valid, non-zero id.
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }

    fn record(&self, _: &Id, _: &Record<'_>) {}

    fn record_follows_from(&self, _: &Id, _: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let meta = event.metadata();
        let Some(level) = forwarded(meta) else {
            return;
        };
        let mut line = Line::default();
        event.record(&mut line);
        let text = format!("{}: {}{}", meta.target(), line.message, line.fields);
        self.sink.log(level, truncated(text, MAX_LINE));
    }

    fn enter(&self, _: &Id) {}

    fn exit(&self, _: &Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Lines(Mutex<Vec<(LogLevel, String)>>);

    impl LogSink for Lines {
        fn log(&self, level: LogLevel, line: String) {
            if let Ok(mut lines) = self.0.lock() {
                lines.push((level, line));
            }
        }
    }

    fn capture(f: impl FnOnce()) -> Vec<(LogLevel, String)> {
        let lines = Arc::new(Lines::default());
        let subscriber = SinkSubscriber {
            sink: lines.clone(),
        };
        tracing::subscriber::with_default(subscriber, f);
        let captured = lines.0.lock().expect("lock").clone();
        captured
    }

    #[test]
    fn our_warnings_are_forwarded_with_their_fields() {
        let lines = capture(|| {
            tracing::warn!(
                target: "mailcrane_core::sync::initial",
                stage = "sync",
                kind = ?Some(3),
                error = "CommandFailed",
                command = "SELECT",
                "sync failed"
            );
            tracing::error!(target: "mailcrane_ffi::sync", "store failed");
        });
        assert_eq!(
            lines,
            vec![
                (
                    LogLevel::Warn,
                    "mailcrane_core::sync::initial: sync failed stage=sync kind=Some(3) \
                     error=CommandFailed command=SELECT"
                        .to_owned()
                ),
                (
                    LogLevel::Error,
                    "mailcrane_ffi::sync: store failed".to_owned()
                ),
            ]
        );
    }

    #[test]
    fn a_dependency_event_with_message_text_is_dropped() {
        // Message-like text in a String field gets through only from our own
        // call sites, which never log it; from a dependency it never does.
        let text = String::from("Subject: Hello\r\n\r\npassword=hunter2");
        let lines = capture(|| {
            tracing::warn!(target: "rustls::conn", text = %text, "bad record");
            tracing::error!(target: "tokio::runtime", text = %text, "worker failed");
            tracing::warn!(target: "mailcrane", "kept");
        });
        assert_eq!(lines, vec![(LogLevel::Warn, "mailcrane: kept".to_owned())]);
    }

    #[test]
    fn info_and_below_are_dropped_even_from_our_crates() {
        let lines = capture(|| {
            tracing::info!(target: "mailcrane_core::sync::live", "IDLE connection lost");
            tracing::debug!(target: "mailcrane_core::sync::live", "reconnect after");
            tracing::trace!(target: "mailcrane_imap", "detail");
        });
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn long_lines_are_cut_on_a_character_boundary() {
        let line = "é".repeat(MAX_LINE);
        let cut = truncated(line, MAX_LINE);
        assert!(cut.len() <= MAX_LINE);
        assert!(cut.chars().all(|c| c == 'é'));
        assert_eq!(truncated("short".to_owned(), MAX_LINE), "short");
    }

    #[test]
    fn installing_twice_returns_false_and_never_panics() {
        // The global default may already be set by another test in this
        // binary; either way the second call must report false.
        let _ = install_log_sink(Arc::new(Lines::default()));
        assert!(!install_log_sink(Arc::new(Lines::default())));
    }
}
