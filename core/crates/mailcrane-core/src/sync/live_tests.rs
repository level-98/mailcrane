//! Live updates. The loop's timing (re-issue, polling, back-off, stop) is
//! tested in paused time with a scripted connection; the whole path is
//! tested against the testkit's fake IMAP server. Never a real mailbox.

use super::*;
use crate::events::{CoreEvent, EventListener};
use mailcrane_imap::ImapSecurity;
use mailcrane_store::{NewAccount, NewFolder, Security};
use mailcrane_testkit::{FakeImap, Quirk};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Mutex;
use tokio::time::Instant;

const SEC: Duration = Duration::from_secs(1);
const MINUTE: Duration = Duration::from_secs(60);

#[derive(Default)]
struct Events(Mutex<Vec<CoreEvent>>);

impl EventListener for Events {
    fn on_event(&self, event: CoreEvent) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }
}

impl Events {
    fn all(&self) -> Vec<CoreEvent> {
        self.0.lock().expect("lock").clone()
    }

    fn clear(&self) {
        self.0.lock().expect("lock").clear();
    }

    fn sync_errors(&self) -> usize {
        self.all()
            .iter()
            .filter(|e| matches!(e, CoreEvent::SyncError { .. }))
            .count()
    }
}

// ---- The loop, in paused time ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ev {
    Connect,
    ConnectFailed,
    Sync,
    SyncFailed,
    Idle,
    Close,
}

/// What one `idle` call does; with nothing planned it waits out `max`.
#[derive(Debug, Clone, Copy)]
enum IdlePlan {
    ChangeAfter(Duration),
    FailAfter(Duration),
}

struct Mock {
    start: Instant,
    log: RefCell<Vec<(Duration, Ev)>>,
    connect_failures: Cell<usize>,
    connect_error: Cell<SyncErrorKind>,
    sync_failures: Cell<usize>,
    idle: bool,
    idle_plan: RefCell<VecDeque<IdlePlan>>,
    close_hangs: bool,
}

impl Mock {
    fn new() -> Self {
        Self {
            start: Instant::now(),
            log: RefCell::new(Vec::new()),
            connect_failures: Cell::new(0),
            connect_error: Cell::new(SyncErrorKind::Network),
            sync_failures: Cell::new(0),
            idle: true,
            idle_plan: RefCell::new(VecDeque::new()),
            close_hangs: false,
        }
    }

    fn record(&self, ev: Ev) {
        self.log.borrow_mut().push((self.start.elapsed(), ev));
    }

    /// When each event of kind `ev` happened.
    fn times(&self, ev: Ev) -> Vec<Duration> {
        self.log
            .borrow()
            .iter()
            .filter(|(_, e)| *e == ev)
            .map(|(t, _)| *t)
            .collect()
    }

    fn take(counter: &Cell<usize>) -> bool {
        let left = counter.get();
        if left > 0 {
            counter.set(left - 1);
        }
        left > 0
    }
}

struct MockSource(Rc<Mock>);
struct MockConn(Rc<Mock>);

impl Source for MockSource {
    type Conn = MockConn;

    async fn connect(&mut self) -> Result<MockConn, SyncErrorKind> {
        if Mock::take(&self.0.connect_failures) {
            self.0.record(Ev::ConnectFailed);
            return Err(self.0.connect_error.get());
        }
        self.0.record(Ev::Connect);
        Ok(MockConn(self.0.clone()))
    }
}

impl Conn for MockConn {
    fn supports_idle(&self) -> bool {
        self.0.idle
    }

    async fn sync(&mut self) -> Result<(), SyncErrorKind> {
        if Mock::take(&self.0.sync_failures) {
            self.0.record(Ev::SyncFailed);
            return Err(SyncErrorKind::Network);
        }
        self.0.record(Ev::Sync);
        Ok(())
    }

    async fn idle(
        &mut self,
        max: Duration,
        _debounce: Duration,
        stop: impl Future<Output = ()>,
    ) -> Result<IdleOutcome, SyncErrorKind> {
        self.0.record(Ev::Idle);
        let plan = self.0.idle_plan.borrow_mut().pop_front();
        let (after, result) = match plan {
            Some(IdlePlan::ChangeAfter(d)) => (d, Ok(IdleOutcome::Changed)),
            Some(IdlePlan::FailAfter(d)) => (d, Err(SyncErrorKind::Network)),
            None => (max, Ok(IdleOutcome::TimedOut)),
        };
        tokio::select! {
            biased;
            () = stop => Ok(IdleOutcome::Stopped),
            () = tokio::time::sleep(after) => result,
        }
    }

    async fn close(&mut self) {
        if self.0.close_hangs {
            std::future::pending::<()>().await;
        }
        self.0.record(Ev::Close);
    }
}

fn exact() -> LiveConfig {
    LiveConfig {
        jitter: false,
        ..LiveConfig::default()
    }
}

/// Runs the loop, asks it to stop at `stop_at`, and returns when it
/// returned.
async fn run_mock(mock: &Rc<Mock>, config: LiveConfig, stop_at: Duration) -> (Duration, Events) {
    let (stop, signal) = live_stop();
    let events = Events::default();
    let mut source = MockSource(mock.clone());
    let started = Instant::now();
    let stopper = async {
        tokio::time::sleep(stop_at).await;
        stop.stop();
        std::future::pending::<()>().await;
    };
    tokio::select! {
        () = run(&mut source, &events, AccountId(1), config, signal) => {}
        () = stopper => {}
    }
    (started.elapsed(), events)
}

fn gaps(times: &[Duration]) -> Vec<Duration> {
    times.windows(2).map(|w| w[1] - w[0]).collect()
}

#[tokio::test(start_paused = true)]
async fn a_refused_login_is_reported_once_and_not_retried() {
    let mock = Rc::new(Mock::new());
    mock.connect_failures.set(usize::MAX);
    mock.connect_error.set(SyncErrorKind::Auth);
    let (returned, events) = run_mock(&mock, exact(), 60 * MINUTE).await;
    // One attempt, one error event, and the loop ends straight away
    // instead of logging in again every few minutes.
    assert_eq!(mock.times(Ev::ConnectFailed), [Duration::ZERO]);
    assert!(mock.times(Ev::Connect).is_empty());
    assert_eq!(events.sync_errors(), 1);
    assert_eq!(returned, Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn idle_is_reissued_every_25_minutes() {
    let mock = Rc::new(Mock::new());
    let (returned, _) = run_mock(&mock, exact(), 60 * MINUTE).await;
    assert_eq!(mock.times(Ev::Connect), [Duration::ZERO]);
    assert_eq!(mock.times(Ev::Sync), [Duration::ZERO]);
    // RFC 2177 §3: well before the 29-minute limit.
    assert_eq!(
        mock.times(Ev::Idle),
        [Duration::ZERO, 25 * MINUTE, 50 * MINUTE]
    );
    assert_eq!(mock.times(Ev::Close), [60 * MINUTE]);
    assert_eq!(returned, 60 * MINUTE);
}

#[tokio::test(start_paused = true)]
async fn without_idle_it_polls_every_5_minutes() {
    let mut mock = Mock::new();
    mock.idle = false;
    let mock = Rc::new(mock);
    run_mock(&mock, exact(), 16 * MINUTE).await;
    assert_eq!(
        mock.times(Ev::Sync),
        [Duration::ZERO, 5 * MINUTE, 10 * MINUTE, 15 * MINUTE]
    );
    assert!(mock.times(Ev::Idle).is_empty());
    assert_eq!(mock.times(Ev::Connect).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_change_syncs_then_idles_again() {
    let mock = Rc::new(Mock::new());
    mock.idle_plan
        .borrow_mut()
        .extend([IdlePlan::ChangeAfter(MINUTE), IdlePlan::ChangeAfter(SEC)]);
    run_mock(&mock, exact(), 10 * MINUTE).await;
    assert_eq!(mock.times(Ev::Sync), [Duration::ZERO, MINUTE, MINUTE + SEC]);
    assert_eq!(mock.times(Ev::Idle), [Duration::ZERO, MINUTE, MINUTE + SEC]);
    assert_eq!(mock.times(Ev::Connect).len(), 1);
}

#[tokio::test(start_paused = true)]
async fn failed_connections_back_off_from_2_seconds_to_5_minutes() {
    let mock = Rc::new(Mock::new());
    mock.connect_failures.set(11);
    let (_, events) = run_mock(&mock, exact(), 60 * MINUTE).await;
    let mut attempts = mock.times(Ev::ConnectFailed);
    attempts.extend(mock.times(Ev::Connect));
    let expected: Vec<Duration> = [2, 4, 8, 16, 32, 64, 128, 256, 300, 300, 300]
        .into_iter()
        .map(Duration::from_secs)
        .collect();
    assert_eq!(gaps(&attempts), expected);
    // Every failure is reported; then the connection works and idles.
    assert_eq!(events.sync_errors(), 11);
    assert_eq!(mock.times(Ev::Sync).len(), 1);
    assert_eq!(
        mock.times(Ev::Idle).first(),
        mock.times(Ev::Connect).first()
    );
}

#[tokio::test(start_paused = true)]
async fn a_dropped_connection_reconnects_and_idle_resumes() {
    let mock = Rc::new(Mock::new());
    mock.idle_plan
        .borrow_mut()
        .extend([IdlePlan::FailAfter(3 * MINUTE), IdlePlan::FailAfter(MINUTE)]);
    run_mock(&mock, exact(), 10 * MINUTE).await;
    let back = 3 * MINUTE + 2 * SEC;
    // The successful sync after each reconnect resets the back-off to 2 s.
    let again = back + MINUTE + 2 * SEC;
    assert_eq!(mock.times(Ev::Connect), [Duration::ZERO, back, again]);
    assert_eq!(mock.times(Ev::Sync), [Duration::ZERO, back, again]);
    assert_eq!(mock.times(Ev::Idle), [Duration::ZERO, back, again]);
}

#[tokio::test(start_paused = true)]
async fn failed_syncs_keep_backing_off_until_one_succeeds() {
    let mock = Rc::new(Mock::new());
    mock.sync_failures.set(3);
    mock.idle_plan
        .borrow_mut()
        .push_back(IdlePlan::FailAfter(MINUTE));
    run_mock(&mock, exact(), 10 * MINUTE).await;
    let ok = 2 * SEC + 4 * SEC + 8 * SEC;
    assert_eq!(
        mock.times(Ev::Connect),
        [Duration::ZERO, 2 * SEC, 6 * SEC, ok, ok + MINUTE + 2 * SEC]
    );
    assert_eq!(mock.times(Ev::Sync), [ok, ok + MINUTE + 2 * SEC]);
}

#[tokio::test(start_paused = true)]
async fn stopping_while_idle_closes_the_connection() {
    let mock = Rc::new(Mock::new());
    let (returned, _) = run_mock(&mock, exact(), 7 * MINUTE).await;
    assert_eq!(mock.times(Ev::Close), [7 * MINUTE]);
    assert_eq!(returned, 7 * MINUTE);
}

#[tokio::test(start_paused = true)]
async fn stopping_returns_within_2_seconds_when_the_server_hangs() {
    let mut mock = Mock::new();
    mock.close_hangs = true;
    let mock = Rc::new(mock);
    let (returned, _) = run_mock(&mock, exact(), 7 * MINUTE).await;
    assert_eq!(returned, 7 * MINUTE + 2 * SEC);
}

#[tokio::test(start_paused = true)]
async fn stopping_while_backing_off_returns_at_once() {
    let mock = Rc::new(Mock::new());
    mock.connect_failures.set(usize::MAX);
    let (returned, _) = run_mock(&mock, exact(), 10 * MINUTE).await;
    assert_eq!(returned, 10 * MINUTE);
    assert!(mock.times(Ev::Close).is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_stop_before_starting_does_nothing() {
    let mock = Rc::new(Mock::new());
    let (stop, signal) = live_stop();
    stop.stop();
    let mut source = MockSource(mock.clone());
    run(
        &mut source,
        &Events::default(),
        AccountId(1),
        exact(),
        signal,
    )
    .await;
    assert_eq!(mock.start.elapsed(), Duration::ZERO);
    assert!(mock.log.borrow().is_empty());
}

#[tokio::test(start_paused = true)]
async fn dropping_the_stop_handle_stops_the_loop() {
    let mock = Rc::new(Mock::new());
    let (stop, signal) = live_stop();
    let mut source = MockSource(mock.clone());
    let events = Events::default();
    tokio::join!(
        run(&mut source, &events, AccountId(1), exact(), signal),
        async {
            tokio::time::sleep(MINUTE).await;
            drop(stop);
        }
    );
    assert_eq!(mock.times(Ev::Close), [MINUTE]);
}

#[test]
fn jitter_shortens_delays_by_at_most_a_quarter() {
    let config = LiveConfig::default();
    for seed in 0..200u64 {
        let mut backoff = Backoff::new(&config, seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut nominal = config.backoff_min;
        for _ in 0..12 {
            let delay = backoff.next_delay();
            assert!(delay <= nominal, "{delay:?} > {nominal:?}");
            assert!(delay >= config.backoff_min, "{delay:?}");
            assert!(delay >= nominal - nominal / 4, "{delay:?} for {nominal:?}");
            nominal = (nominal * 2).min(config.backoff_max);
        }
    }
}

#[test]
fn backoff_never_overflows() {
    let config = LiveConfig {
        backoff_min: Duration::MAX,
        backoff_max: Duration::MAX,
        ..LiveConfig::default()
    };
    let mut backoff = Backoff::new(&config, 7);
    for _ in 0..100 {
        assert_eq!(backoff.next_delay(), Duration::MAX);
    }
}

// ---- Against the fake IMAP server, in real time ----

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";

fn raw(n: usize) -> Vec<u8> {
    format!("From: bob@example.com\r\nSubject: Message {n}\r\n\r\nBody {n}\r\n").into_bytes()
}

/// A store with the account and no folders.
fn account_only() -> (Store, AccountId) {
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 587,
            smtp_security: Security::Starttls,
            username: USER.into(),
        })
        .expect("account");
    (store, account)
}

fn setup() -> (Store, AccountId, FolderId) {
    let (store, account) = account_only();
    let folder = store
        .upsert_folder(&NewFolder {
            account_id: account,
            remote_name: "INBOX".into(),
            display_name: "INBOX".into(),
            delimiter: Some("/".into()),
            role: Some(FolderRole::Inbox),
            uidvalidity: None,
            uidnext: None,
            highestmodseq: None,
            is_subscribed: true,
            selectable: true,
        })
        .expect("folder");
    (store, account, folder)
}

fn login(imap: &FakeImap) -> impl Fn() -> Result<ImapLogin, SyncErrorKind> {
    let port = imap.addr().port();
    let cert = imap.root_cert_der().to_vec();
    move || {
        let mut config = ImapConfig::new("localhost", port, ImapSecurity::ImplicitTls)
            .with_extra_root_cert(cert.clone());
        config.command_timeout = Duration::from_secs(2);
        Ok(ImapLogin {
            config,
            username: USER.into(),
            password: Secret::new(PASSWORD),
        })
    }
}

fn fast() -> LiveConfig {
    LiveConfig {
        backoff_min: Duration::from_millis(100),
        jitter: false,
        ..LiveConfig::default()
    }
}

/// Waits up to `limit` for `done`; returns how long it took.
async fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> Duration {
    let started = std::time::Instant::now();
    while !done() {
        assert!(started.elapsed() < limit, "not done within {limit:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    started.elapsed()
}

fn stored(store: &Store, folder: FolderId) -> Vec<u32> {
    store.list_uids(folder).expect("uids")
}

fn changed(events: &Events, folder: FolderId) -> bool {
    events
        .all()
        .contains(&CoreEvent::MessagesChanged { folder_id: folder })
}

async fn fake(count: usize, quirk: Option<Quirk>) -> FakeImap {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", (1..=count).map(raw).collect());
    if let Some(quirk) = quirk {
        builder = builder.quirk(quirk);
    }
    builder.start().await.expect("fake imap starts")
}

/// Runs live updates while `body` runs, then stops them and checks that
/// they ended within 2 seconds.
async fn with_live<F: Future<Output = ()>>(
    imap: &FakeImap,
    store: &Store,
    account: AccountId,
    events: &Events,
    config: LiveConfig,
    body: impl FnOnce() -> F,
) {
    let (stop, signal) = live_stop();
    let stopped_at = Cell::new(None);
    tokio::join!(
        run_live_updates(
            store,
            super::super::LiveAccount {
                id: account,
                write_lock: super::super::WriteLock::default(),
            },
            events,
            login(imap),
            SyncConfig::default(),
            config,
            signal,
        ),
        async {
            body().await;
            stopped_at.set(Some(std::time::Instant::now()));
            stop.stop();
        }
    );
    let took = stopped_at.get().expect("stopped").elapsed();
    assert!(took < Duration::from_secs(2), "stopping took {took:?}");
}

#[tokio::test]
async fn a_delivered_message_is_stored_and_announced_within_2_seconds() {
    let imap = fake(3, None).await;
    let (store, account, inbox) = setup();
    let events = Events::default();
    with_live(&imap, &store, account, &events, fast(), || async {
        wait_for(Duration::from_secs(10), || stored(&store, inbox).len() == 3).await;
        // Let the loop get into IDLE.
        wait_for(Duration::from_secs(5), || {
            imap.received_commands()
                .now_or_never()
                .is_some_and(|c| c.iter().any(|c| c == "IDLE"))
        })
        .await;
        events.clear();
        imap.mailbox().deliver("INBOX", raw(4)).await;
        let took = wait_for(Duration::from_secs(2), || {
            stored(&store, inbox).len() == 4 && changed(&events, inbox)
        })
        .await;
        assert!(took < Duration::from_secs(2), "{took:?}");
    })
    .await;
    let commands = imap.received_commands().await;
    assert!(
        commands.ends_with(&["DONE".to_string(), "LOGOUT".to_string()]),
        "{commands:?}"
    );
    assert_eq!(imap.connections_accepted(), 1);
    assert_eq!(events.sync_errors(), 0);
}

#[tokio::test]
async fn an_expunged_message_leaves_the_store_within_2_seconds() {
    let imap = fake(3, None).await;
    let (store, account, inbox) = setup();
    let events = Events::default();
    with_live(&imap, &store, account, &events, fast(), || async {
        wait_for(Duration::from_secs(10), || stored(&store, inbox).len() == 3).await;
        wait_for(Duration::from_secs(5), || idling(&imap)).await;
        imap.mailbox().expunge("INBOX", 2).await;
        let took = wait_for(Duration::from_secs(2), || {
            stored(&store, inbox) == [1, 3] && changed(&events, inbox)
        })
        .await;
        assert!(took < Duration::from_secs(2), "{took:?}");
    })
    .await;
}

#[tokio::test]
async fn without_idle_new_mail_arrives_by_polling() {
    let imap = fake(1, Some(Quirk::NoIdle)).await;
    let (store, account, inbox) = setup();
    let events = Events::default();
    let config = LiveConfig {
        poll_every: Duration::from_millis(300),
        ..fast()
    };
    with_live(&imap, &store, account, &events, config, || async {
        wait_for(Duration::from_secs(10), || stored(&store, inbox).len() == 1).await;
        imap.mailbox().deliver("INBOX", raw(2)).await;
        wait_for(Duration::from_secs(2), || stored(&store, inbox).len() == 2).await;
    })
    .await;
    let commands = imap.received_commands().await;
    assert!(!commands.iter().any(|c| c == "IDLE"), "{commands:?}");
    assert_eq!(imap.connections_accepted(), 1);
}

#[tokio::test]
async fn after_a_dropped_connection_it_reconnects_and_idles_again() {
    let imap = fake(2, None).await;
    let (store, account, inbox) = setup();
    let events = Events::default();
    with_live(&imap, &store, account, &events, fast(), || async {
        wait_for(Duration::from_secs(10), || stored(&store, inbox).len() == 2).await;
        wait_for(Duration::from_secs(5), || idling(&imap)).await;
        imap.drop_connections();
        // Back-off (100 ms here), reconnect, sync, IDLE again.
        wait_for(Duration::from_secs(5), || {
            imap.connections_accepted() == 2 && idle_count(&imap) >= 2
        })
        .await;
        imap.mailbox().deliver("INBOX", raw(3)).await;
        wait_for(Duration::from_secs(2), || stored(&store, inbox).len() == 3).await;
    })
    .await;
}

#[tokio::test]
async fn folders_are_listed_first_when_none_are_stored() {
    let imap = fake(2, None).await;
    // The folders were never listed.
    let (store, account) = account_only();
    assert_eq!(find_inbox(&store, account), Ok(None));
    let events = Events::default();
    with_live(&imap, &store, account, &events, fast(), || async {
        wait_for(Duration::from_secs(10), || {
            find_inbox(&store, account)
                .ok()
                .flatten()
                .is_some_and(|inbox| stored(&store, inbox).len() == 2)
        })
        .await;
    })
    .await;
}

#[tokio::test]
async fn bad_credentials_are_reported_once_and_the_loop_ends() {
    let imap = fake(1, None).await;
    let (store, account, _) = setup();
    let events = Events::default();
    let (_stop, signal) = live_stop();
    let port = imap.addr().port();
    let cert = imap.root_cert_der().to_vec();
    let wrong = move || {
        Ok(ImapLogin {
            config: ImapConfig::new("localhost", port, ImapSecurity::ImplicitTls)
                .with_extra_root_cert(cert.clone()),
            username: USER.into(),
            password: Secret::new("wrong"),
        })
    };
    // Nobody stops it: a refused login ends the loop by itself.
    tokio::time::timeout(
        Duration::from_secs(5),
        run_live_updates(
            &store,
            super::super::LiveAccount {
                id: account,
                write_lock: super::super::WriteLock::default(),
            },
            &events,
            wrong,
            SyncConfig::default(),
            fast(),
            signal,
        ),
    )
    .await
    .expect("the loop ends after a refused login");
    assert_eq!(imap.connections_accepted(), 1);
    assert_eq!(events.sync_errors(), 1);
    assert!(events.all().iter().all(|e| matches!(
        e,
        CoreEvent::SyncError {
            kind: SyncErrorKind::Auth,
            ..
        }
    )));
}

fn idle_count(imap: &FakeImap) -> usize {
    imap.received_commands()
        .now_or_never()
        .map_or(0, |c| c.iter().filter(|c| *c == "IDLE").count())
}

fn idling(imap: &FakeImap) -> bool {
    idle_count(imap) >= 1
}

/// Polls a future once; for reading the fake server's command log from a
/// synchronous check. Its lock is held only briefly, so this rarely misses.
trait NowOrNever: Future + Sized {
    fn now_or_never(self) -> Option<Self::Output> {
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        let mut fut = std::pin::pin!(self);
        match fut.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => Some(v),
            std::task::Poll::Pending => None,
        }
    }
}

impl<F: Future> NowOrNever for F {}

#[tokio::test]
async fn rules_run_on_live_mail_and_idle_returns_to_the_inbox() {
    use crate::rules::{NewRule, RuleAction, RuleCondition};
    let imap = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw(1)])
        .folder("Deals", vec![])
        .start()
        .await
        .expect("fake imap starts");
    // No folders stored: the live loop lists them (INBOX and Deals).
    let (store, account) = account_only();
    let events = Events::default();
    let folder = |name: &str| {
        store
            .list_folders(account)
            .ok()?
            .into_iter()
            .find(|f| f.details.remote_name == name)
            .map(|f| f.id)
    };
    with_live(&imap, &store, account, &events, fast(), || async {
        wait_for(Duration::from_secs(10), || {
            folder("INBOX").is_some_and(|inbox| stored(&store, inbox).len() == 1)
        })
        .await;
        let deals = folder("Deals").expect("Deals listed");
        store
            .insert_rule(&NewRule {
                account_id: None,
                name: "Deals".into(),
                enabled: true,
                match_all: true,
                stop_processing: false,
                conditions: vec![RuleCondition::ListId("deals.shop.example".into())],
                actions: vec![RuleAction::MarkRead, RuleAction::Move(deals)],
            })
            .expect("rule");
        wait_for(Duration::from_secs(5), || idling(&imap)).await;
        imap.mailbox()
            .deliver(
                "INBOX",
                b"From: news@shop.example\r\nSubject: Sale\r\nList-Id: News <deals.shop.example>\r\n\r\nHi\r\n"
                    .to_vec(),
            )
            .await;
        // The rule moved it on the server and marked it read.
        let mailbox = imap.mailbox();
        wait_for(Duration::from_secs(5), || {
            let deals = mailbox.messages("Deals").now_or_never().unwrap_or_default();
            deals.len() == 1 && deals[0].flags.iter().any(|f| f == r"\Seen")
        })
        .await;
        // Replaying the move selected Deals; live updates are back on the
        // inbox, so the next delivery still arrives.
        let inbox = folder("INBOX").expect("inbox");
        imap.mailbox().deliver("INBOX", raw(3)).await;
        wait_for(Duration::from_secs(3), || {
            store
                .list_messages(inbox, 100, 0)
                .map(|m| m.iter().any(|m| m.details.subject == "Message 3"))
                .unwrap_or(false)
        })
        .await;
    })
    .await;
}
