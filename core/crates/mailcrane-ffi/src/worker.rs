//! Runs the async exports' work on a thread of its own.
//!
//! UniFFI polls an exported `async fn` on whichever thread the app awaits
//! it from, which on Android is often the main thread. Network work must
//! never run there: Android forbids it, and the platform certificate check
//! fetches revocation lists over the network during the TLS handshake, so
//! sign-in failed with "couldn't set up a secure connection" (issue #243).
//! Doing the work here makes every caller safe, whatever thread it uses.

use std::future::Future;

/// Runs the future that `work` makes to completion on a new thread with its
/// own runtime, and waits for its result without blocking the caller.
/// `None` if the thread couldn't start or stopped without an answer.
///
/// If the caller stops waiting, the work still finishes (each network step
/// has its own timeout); its result is dropped.
pub(crate) async fn on_worker_thread<T, F, Fut>(work: F) -> Option<T>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = T>,
    T: Send + 'static,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new()
        .name("mailcrane-net".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            // The receiver may be gone if the caller gave up; nothing to do.
            let _ = tx.send(runtime.block_on(work()));
        });
    if spawned.is_err() {
        return None;
    }
    rx.await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn work_runs_on_another_thread() {
        let caller = std::thread::current().id();
        let worker = on_worker_thread(|| async { std::thread::current().id() }).await;
        assert!(worker.is_some());
        assert_ne!(worker, Some(caller));
    }

    #[tokio::test]
    async fn the_result_comes_back() {
        let got = on_worker_thread(|| async {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            41 + 1
        })
        .await;
        assert_eq!(got, Some(42));
    }

    #[tokio::test]
    async fn a_failed_worker_is_none_not_a_hang() {
        #[allow(clippy::panic)]
        let got: Option<u8> = on_worker_thread(|| async { panic!("worker failed") }).await;
        assert_eq!(got, None);
    }
}
