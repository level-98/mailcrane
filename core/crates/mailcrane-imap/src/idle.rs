//! Waiting for changes with `IDLE` (RFC 2177, RFC 9051 §6.3.13).
//!
//! The client sends `IDLE`, the server answers `+`, and from then on the
//! server may send untagged `EXISTS`, `EXPUNGE` and `FETCH` responses as
//! the selected folder changes. The client ends the wait with `DONE`, and
//! the server finishes the command with a tagged `OK`.
//!
//! [`Session::idle`] does one such wait. It ends when something changed
//! (after a short debounce, so a burst of changes needs one sync), when
//! the caller's time limit is reached, or when the caller asks it to stop.
//! Servers may drop a client that idles for 30 minutes (RFC 2177 §3), so
//! callers re-issue it well before that, every 25 minutes or so.

use crate::connection::within;
use crate::error::ImapError;
use crate::parser::{Condition, Response, StatusKind, Untagged};
use crate::session::{has_cap, Session};
use std::future::Future;
use std::time::Duration;
use tokio::time::Instant;

/// Why [`Session::idle`] returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleOutcome {
    /// The server reported a change in the selected folder: sync it.
    Changed,
    /// The time limit passed without a change: idle again.
    TimedOut,
    /// The caller's `stop` future finished.
    Stopped,
}

/// Whether an untagged response, sent during `IDLE`, means the selected
/// folder changed. `BYE` is an error: the server is closing the connection.
fn is_change(untagged: &Untagged) -> Result<bool, ImapError> {
    Ok(match untagged {
        Untagged::Condition {
            condition: Condition::Bye,
            ..
        } => return Err(ImapError::ServerBye),
        // RFC 9051 §7.4.1, §7.5.1, §7.5.2.
        Untagged::Exists(_)
        | Untagged::Expunge(_)
        | Untagged::Fetch(_)
        | Untagged::FetchMalformed { .. } => true,
        // RFC 7162 §3.2.10: with QRESYNC enabled, VANISHED replaces EXPUNGE.
        Untagged::Vanished { earlier, .. } => !earlier,
        // `* OK still here`, `* n RECENT` and the like change nothing.
        _ => false,
    })
}

impl Session {
    /// Whether the server advertises `IDLE` (RFC 2177 §1).
    pub fn supports_idle(&self) -> bool {
        has_cap(self.capabilities(), "IDLE")
    }

    /// Waits for changes in the selected folder with `IDLE` (RFC 2177).
    ///
    /// Returns [`IdleOutcome::Changed`] once the server reports a change
    /// and `debounce` has passed with the rest of any burst collected,
    /// [`IdleOutcome::TimedOut`] after `max` without a change, and
    /// [`IdleOutcome::Stopped`] as soon as `stop` finishes. In every case
    /// `DONE` is sent and the tagged reply read before returning, so the
    /// session can run other commands next.
    ///
    /// A server that refuses `IDLE` gives [`ImapError::CommandFailed`].
    /// Waiting for the server has no time limit other than `max`; sending
    /// `IDLE` and `DONE` each have the command timeout.
    pub async fn idle(
        &mut self,
        max: Duration,
        debounce: Duration,
        stop: impl Future<Output = ()>,
    ) -> Result<IdleOutcome, ImapError> {
        self.last_command = Some("IDLE");
        let tag = self.tags.next();
        let limit = self.command_timeout;
        let deadline = Instant::now() + max;
        let mut changed = false;

        // RFC 2177 §3: the server answers IDLE with a continuation.
        let start = format!("{tag} IDLE\r\n");
        within(limit, async {
            self.conn.write(start.as_bytes()).await?;
            loop {
                match self.conn.read_response().await? {
                    Response::Continuation { .. } => return Ok(()),
                    Response::Untagged(u) => changed |= is_change(&u)?,
                    Response::Tagged { tag: t, status, .. } if t == tag => {
                        // NO or BAD: refused. OK without `+` is not valid.
                        return Err(if status == StatusKind::Ok {
                            ImapError::Protocol
                        } else {
                            ImapError::CommandFailed
                        });
                    }
                    Response::Tagged { .. } => return Err(ImapError::Protocol),
                }
            }
        })
        .await??;

        tokio::pin!(stop);
        let mut settle_at = changed.then(|| Instant::now() + debounce);
        let outcome = loop {
            let wake = settle_at.map_or(deadline, |at| at.min(deadline));
            // `biased`: stopping and the deadline win over a server that
            // never stops sending.
            tokio::select! {
                biased;
                () = &mut stop => break IdleOutcome::Stopped,
                () = tokio::time::sleep_until(wake) => {
                    break if changed { IdleOutcome::Changed } else { IdleOutcome::TimedOut };
                }
                response = self.conn.read_response() => match response? {
                    Response::Untagged(u) => {
                        if is_change(&u)? && !changed {
                            changed = true;
                            settle_at = Some(Instant::now() + debounce);
                        }
                    }
                    // Nothing is tagged until we send DONE.
                    Response::Tagged { .. } | Response::Continuation { .. } => {
                        return Err(ImapError::Protocol);
                    }
                },
            }
        };

        // RFC 2177 §3: DONE ends the command; changes may still arrive
        // before the tagged reply.
        within(limit, async {
            self.conn.write(b"DONE\r\n").await?;
            loop {
                match self.conn.read_response().await? {
                    Response::Untagged(u) => changed |= is_change(&u)?,
                    Response::Tagged { tag: t, status, .. } if t == tag => {
                        return if status == StatusKind::Ok {
                            Ok(())
                        } else {
                            Err(ImapError::CommandFailed)
                        };
                    }
                    Response::Tagged { .. } | Response::Continuation { .. } => {
                        return Err(ImapError::Protocol)
                    }
                }
            }
        })
        .await??;

        Ok(match outcome {
            IdleOutcome::Stopped => IdleOutcome::Stopped,
            _ if changed => IdleOutcome::Changed,
            _ => IdleOutcome::TimedOut,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

    const MINUTE: Duration = Duration::from_secs(60);
    const TIMEOUT: Duration = Duration::from_secs(10);

    /// A session whose server end the test drives.
    fn pair() -> (Session, BufReader<DuplexStream>) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let session = Session::from_io(Box::new(client), &["IMAP4rev1", "IDLE"], TIMEOUT);
        (session, BufReader::new(server))
    }

    async fn line(server: &mut BufReader<DuplexStream>) -> String {
        let mut out = String::new();
        server.read_line(&mut out).await.expect("read");
        out
    }

    async fn send(server: &mut BufReader<DuplexStream>, text: &str) {
        server
            .get_mut()
            .write_all(text.as_bytes())
            .await
            .expect("write");
    }

    /// Reads `<tag> IDLE`, answers `+` and returns the tag.
    async fn accept_idle(server: &mut BufReader<DuplexStream>) -> String {
        let command = line(server).await;
        let (tag, rest) = command.split_once(' ').expect("tagged");
        assert_eq!(rest, "IDLE\r\n");
        send(server, "+ idling\r\n").await;
        tag.to_string()
    }

    /// Reads `DONE` and answers the tagged OK.
    async fn finish_idle(server: &mut BufReader<DuplexStream>, tag: &str) {
        assert_eq!(line(server).await, "DONE\r\n");
        send(server, &format!("{tag} OK IDLE terminated\r\n")).await;
    }

    fn never() -> std::future::Pending<()> {
        std::future::pending()
    }

    #[test]
    fn capability_is_read() {
        let (session, _server) = pair();
        assert!(session.supports_idle());
        let (client, _server) = tokio::io::duplex(64);
        let plain = Session::from_io(Box::new(client), &["IMAP4rev1"], TIMEOUT);
        assert!(!plain.supports_idle());
    }

    #[tokio::test(start_paused = true)]
    async fn idle_is_reissued_before_29_minutes() {
        let (mut session, mut server) = pair();
        let script = tokio::spawn(async move {
            let mut done_after = Vec::new();
            for _ in 0..2 {
                let tag = accept_idle(&mut server).await;
                let started = Instant::now();
                finish_idle(&mut server, &tag).await;
                done_after.push(started.elapsed());
            }
            done_after
        });
        for _ in 0..2 {
            let outcome = session
                .idle(25 * MINUTE, Duration::from_millis(500), never())
                .await
                .expect("idle");
            assert_eq!(outcome, IdleOutcome::TimedOut);
        }
        let done_after = script.await.expect("script");
        assert_eq!(done_after.len(), 2);
        for elapsed in done_after {
            // RFC 2177 §3: before the server's 29-minute (30 - 1) limit.
            assert!(elapsed >= 25 * MINUTE, "{elapsed:?}");
            assert!(elapsed < 29 * MINUTE, "{elapsed:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_burst_of_changes_is_one_change_after_the_debounce() {
        let (mut session, mut server) = pair();
        let script = tokio::spawn(async move {
            let tag = accept_idle(&mut server).await;
            tokio::time::sleep(MINUTE).await;
            let first = Instant::now();
            send(&mut server, "* 4 EXISTS\r\n* 1 RECENT\r\n").await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            send(&mut server, "* 2 EXPUNGE\r\n").await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            send(&mut server, "* 1 FETCH (UID 7 FLAGS (\\Seen))\r\n").await;
            finish_idle(&mut server, &tag).await;
            first.elapsed()
        });
        let outcome = session
            .idle(25 * MINUTE, Duration::from_millis(500), never())
            .await
            .expect("idle");
        assert_eq!(outcome, IdleOutcome::Changed);
        let waited = script.await.expect("script");
        // DONE comes 500 ms after the first change, not after each one.
        assert_eq!(waited, Duration::from_millis(500));
    }

    #[tokio::test(start_paused = true)]
    async fn each_kind_of_change_ends_the_wait() {
        for change in [
            "* 3 EXISTS\r\n",
            "* 1 EXPUNGE\r\n",
            "* 2 FETCH (FLAGS (\\Flagged))\r\n",
            "* VANISHED 5:6\r\n",
        ] {
            let (mut session, mut server) = pair();
            let script = tokio::spawn(async move {
                let tag = accept_idle(&mut server).await;
                send(&mut server, change).await;
                finish_idle(&mut server, &tag).await;
            });
            let outcome = session
                .idle(25 * MINUTE, Duration::from_millis(500), never())
                .await
                .expect("idle");
            assert_eq!(outcome, IdleOutcome::Changed, "{change:?}");
            script.await.expect("script");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn keepalives_and_vanished_earlier_are_not_changes() {
        let (mut session, mut server) = pair();
        let script = tokio::spawn(async move {
            let tag = accept_idle(&mut server).await;
            send(&mut server, "* OK still here\r\n* 2 RECENT\r\n").await;
            send(&mut server, "* VANISHED (EARLIER) 1:3\r\n").await;
            finish_idle(&mut server, &tag).await;
        });
        let outcome = session
            .idle(MINUTE, Duration::from_millis(500), never())
            .await
            .expect("idle");
        assert_eq!(outcome, IdleOutcome::TimedOut);
        script.await.expect("script");
    }

    #[tokio::test(start_paused = true)]
    async fn a_change_before_the_continuation_counts() {
        let (mut session, mut server) = pair();
        let script = tokio::spawn(async move {
            let command = line(&mut server).await;
            let tag = command.split(' ').next().expect("tag").to_string();
            send(&mut server, "* 9 EXISTS\r\n+ idling\r\n").await;
            finish_idle(&mut server, &tag).await;
        });
        let outcome = session
            .idle(25 * MINUTE, Duration::from_millis(500), never())
            .await
            .expect("idle");
        assert_eq!(outcome, IdleOutcome::Changed);
        script.await.expect("script");
    }

    #[tokio::test(start_paused = true)]
    async fn stop_sends_done_at_once() {
        let (mut session, mut server) = pair();
        let script = tokio::spawn(async move {
            let tag = accept_idle(&mut server).await;
            let started = Instant::now();
            finish_idle(&mut server, &tag).await;
            started.elapsed()
        });
        let stop = tokio::time::sleep(Duration::from_secs(3));
        let outcome = session
            .idle(25 * MINUTE, Duration::from_millis(500), stop)
            .await
            .expect("idle");
        assert_eq!(outcome, IdleOutcome::Stopped);
        assert_eq!(script.await.expect("script"), Duration::from_secs(3));
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_idle_is_command_failed() {
        let (mut session, mut server) = pair();
        tokio::spawn(async move {
            let command = line(&mut server).await;
            let tag = command.split(' ').next().expect("tag").to_string();
            send(&mut server, &format!("{tag} BAD IDLE disabled\r\n")).await;
            // Keep the stream open.
            tokio::time::sleep(MINUTE).await;
        });
        let result = session
            .idle(25 * MINUTE, Duration::from_millis(500), never())
            .await;
        assert!(
            matches!(result, Err(ImapError::CommandFailed)),
            "{result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn bye_while_idling_is_an_error() {
        let (mut session, mut server) = pair();
        tokio::spawn(async move {
            accept_idle(&mut server).await;
            send(&mut server, "* BYE shutting down\r\n").await;
            tokio::time::sleep(MINUTE).await;
        });
        let result = session
            .idle(25 * MINUTE, Duration::from_millis(500), never())
            .await;
        assert!(matches!(result, Err(ImapError::ServerBye)), "{result:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_connection_is_an_error() {
        let (mut session, mut server) = pair();
        tokio::spawn(async move {
            accept_idle(&mut server).await;
            tokio::time::sleep(MINUTE).await;
            drop(server);
        });
        let result = session
            .idle(25 * MINUTE, Duration::from_millis(500), never())
            .await;
        assert!(
            matches!(result, Err(ImapError::ConnectionClosed)),
            "{result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_server_that_never_answers_done_times_out() {
        let (mut session, mut server) = pair();
        tokio::spawn(async move {
            accept_idle(&mut server).await;
            // Reads DONE but never answers.
            let _ = line(&mut server).await;
            tokio::time::sleep(25 * MINUTE).await;
        });
        let result = session
            .idle(MINUTE, Duration::from_millis(500), never())
            .await;
        assert!(matches!(result, Err(ImapError::Timeout)), "{result:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_tagged_response_while_idling_is_a_protocol_error() {
        let (mut session, mut server) = pair();
        tokio::spawn(async move {
            accept_idle(&mut server).await;
            send(&mut server, "X1 OK what\r\n").await;
            tokio::time::sleep(MINUTE).await;
        });
        let result = session
            .idle(25 * MINUTE, Duration::from_millis(500), never())
            .await;
        assert!(matches!(result, Err(ImapError::Protocol)), "{result:?}");
    }
}
