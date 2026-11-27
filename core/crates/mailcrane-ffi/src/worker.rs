//! Runs the async exports' work on a thread of its own.
//!
//! UniFFI polls an exported `async fn` on whichever thread the app awaits
//! it from, which on Android is often the main thread. Network work must
//! never run there: Android forbids it, and the platform certificate check
//! fetches revocation lists over the network during the TLS handshake, so
//! sign-in failed with "couldn't set up a secure connection" (issue #243).
//! Doing the work here makes every caller safe, whatever thread it uses.
//!
//! The blocking exports do the same with [`on_job_thread`] (#462): the core's
//! work never runs on the caller's stack, whose size the app chooses, but on
//! a thread whose stack size this crate sets ([`JOB_STACK_SIZE`]).

use std::cell::Cell;
use std::future::Future;

/// Stack size of every thread this crate starts to run the core's work on
/// (#462). The apps call in on threads with small stacks: Android gives a
/// Kotlin `Dispatchers.Default` worker about 1 MiB. In debug builds an async
/// poll function's frame can be very large (the futures nested inside it are
/// laid out in it), and a snooze's IMAP MOVE overflowed such a thread,
/// crashing the app on every launch. This is reserved address space; only
/// the pages a call actually touches take memory.
pub(crate) const JOB_STACK_SIZE: usize = 8 * 1024 * 1024;

thread_local! {
    /// Whether this thread was started by [`job_thread`], so its stack is
    /// [`JOB_STACK_SIZE`].
    static ON_JOB_THREAD: Cell<bool> = const { Cell::new(false) };
}

/// Starts a thread named `name` with a [`JOB_STACK_SIZE`] stack that runs
/// `f`. Every thread this crate starts for the core's work is started here.
pub(crate) fn job_thread<T, F>(name: String, f: F) -> std::io::Result<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    std::thread::Builder::new()
        .name(name)
        .stack_size(JOB_STACK_SIZE)
        .spawn(move || {
            ON_JOB_THREAD.with(|on| on.set(true));
            f()
        })
}

/// Runs `work` on a thread with a [`JOB_STACK_SIZE`] stack and waits for it;
/// for blocking exports, which the apps call on whatever thread they like.
/// On a thread [`job_thread`] started (a nested call), `work` runs right here.
/// `None` if the thread couldn't start or `work` panicked: the caller turns
/// that into its own error, never a crash or a hang.
pub(crate) fn on_job_thread<T: Send>(work: impl FnOnce() -> T + Send) -> Option<T> {
    if ON_JOB_THREAD.with(Cell::get) {
        return Some(work());
    }
    std::thread::scope(|scope| {
        let worker = std::thread::Builder::new()
            .name("mailcrane-job".into())
            .stack_size(JOB_STACK_SIZE)
            .spawn_scoped(scope, || {
                ON_JOB_THREAD.with(|on| on.set(true));
                work()
            })
            .ok()?;
        // Joined here, so a panic comes back as `Err`, not out of `scope`.
        worker.join().ok()
    })
}

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
    let spawned = job_thread("mailcrane-net".into(), move || {
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

    #[test]
    fn job_work_runs_on_a_thread_with_the_set_stack() {
        let caller = std::thread::current().id();
        let (worker, nested) = on_job_thread(|| {
            let worker = std::thread::current().id();
            // A nested call runs where it is: that thread already has the stack.
            let nested = on_job_thread(|| std::thread::current().id());
            (worker, nested)
        })
        .expect("ran");
        assert_ne!(worker, caller);
        assert_eq!(nested, Some(worker));
        assert!(!ON_JOB_THREAD.with(Cell::get), "the caller isn't marked");
    }

    #[test]
    fn a_job_that_panics_is_none_not_a_crash() {
        #[allow(clippy::panic)]
        let got: Option<u8> = on_job_thread(|| panic!("job failed"));
        assert_eq!(got, None);
    }

    #[tokio::test]
    async fn a_failed_worker_is_none_not_a_hang() {
        #[allow(clippy::panic)]
        let got: Option<u8> = on_worker_thread(|| async { panic!("worker failed") }).await;
        assert_eq!(got, None);
    }
}
