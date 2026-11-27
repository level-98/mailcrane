//! The fault table (#459, #450; the lead's spec for G6): a wake at each
//! step, with each thing the user can do to the message between the
//! sync's read of the row and its writes, on each kind of server, against
//! the fake IMAP server. It drives the real order of a sync's local
//! changes ([`push_local_changes`]: queued actions, then snoozes), with
//! the snooze rows as read before the user acted.
//!
//! Every cell is checked, after the syncs that follow have settled it,
//! against the invariants in the `server.rs` doc and the expected end
//! state for its action. A failure names its cell, e.g.
//! `step1-sent/move/no-MOVE`.

use super::*;
use crate::actions::{apply_action, Action};
use crate::events::{CoreEvent, EventListener};
use crate::snooze::{list_snoozed, snooze, unsnooze, wake_due, SNOOZED_FOLDER_NAME};
use crate::sync::{push_local_changes, FolderSyncer, SyncConfig};
use mailcrane_imap::{ImapConfig, ImapSecurity, Secret};
use mailcrane_store::{MessageId, NewAccount, Security};
use mailcrane_testkit::{FakeImap, Quirk};
use std::time::Duration;

const USER: &str = "alice@example.org";
const PASSWORD: &str = "s3cret-value";
const MID: &str = "lunch@example.net";
const NOW: i64 = 1_795_550_400;
const WAKE: i64 = NOW + 12 * 3600;
const LATER_WAKE: i64 = WAKE + 72 * 3600;
/// When the user acts, and the stale sync runs: the wake is due.
const ACT: i64 = WAKE + 60;

struct Quiet;

impl EventListener for Quiet {
    fn on_event(&self, _event: CoreEvent) {}
}

/// The kinds of server the table runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Server {
    /// MOVE and UIDPLUS (IMAP4rev2).
    Full,
    /// UIDPLUS without MOVE: COPY, `\Deleted`, UID EXPUNGE.
    NoMove,
    /// MOVE without UIDPLUS: no COPYUID, no UID EXPUNGE.
    NoUidplus,
    /// Neither: COPY and `\Deleted`, and the original stays flagged.
    Bare,
}

impl Server {
    const ALL: [Server; 4] = [
        Server::Full,
        Server::NoMove,
        Server::NoUidplus,
        Server::Bare,
    ];

    fn quirks(self) -> &'static [Quirk] {
        match self {
            Server::Full => &[],
            Server::NoMove => &[Quirk::NoMove],
            Server::NoUidplus => &[Quirk::NoUidplus],
            Server::Bare => &[Quirk::NoMove, Quirk::NoUidplus],
        }
    }

    fn has_move(self) -> bool {
        matches!(self, Server::Full | Server::NoUidplus)
    }

    fn id(self) -> &'static str {
        match self {
            Server::Full => "MOVE+UIDPLUS",
            Server::NoMove => "no-MOVE",
            Server::NoUidplus => "no-UIDPLUS",
            Server::Bare => "neither",
        }
    }
}

/// How far the wake got when the sync read its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    /// Due, nothing sent.
    Step0,
    /// UIDNEXT recorded; the MOVE or COPY never left (a drop just before).
    Step1Unsent,
    /// UIDNEXT recorded; the MOVE or COPY happened, its reply was lost.
    Step1Sent,
    /// Copied (no MOVE); the original still in Snoozed.
    Step2,
}

impl At {
    const ALL: [At; 4] = [At::Step0, At::Step1Unsent, At::Step1Sent, At::Step2];

    fn id(self) -> &'static str {
        match self {
            At::Step0 => "step0",
            At::Step1Unsent => "step1-unsent",
            At::Step1Sent => "step1-sent",
            At::Step2 => "step2",
        }
    }
}

/// What the user does between the sync's read and its writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum User {
    Nothing,
    Resnooze,
    Unsnooze,
    Delete,
    Move,
    Read,
}

impl User {
    const ALL: [User; 6] = [
        User::Nothing,
        User::Resnooze,
        User::Unsnooze,
        User::Delete,
        User::Move,
        User::Read,
    ];

    fn id(self) -> &'static str {
        match self {
            User::Nothing => "nothing",
            User::Resnooze => "re-snooze",
            User::Unsnooze => "unsnooze",
            User::Delete => "delete",
            User::Move => "move",
            User::Read => "read",
        }
    }
}

/// Where the message is expected to end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Loc {
    Inbox,
    Snoozed,
    Archive,
    Trash,
}

impl Loc {
    fn name(self) -> &'static str {
        match self {
            Loc::Inbox => "INBOX",
            Loc::Snoozed => SNOOZED_FOLDER_NAME,
            Loc::Archive => "Archive",
            Loc::Trash => "Trash",
        }
    }
}

const FOLDERS: [Loc; 4] = [Loc::Inbox, Loc::Snoozed, Loc::Archive, Loc::Trash];

/// The expected end state of a cell.
#[derive(Debug)]
struct Expect {
    /// The user's action is refused, and nothing changes for it: a
    /// re-snooze, move or delete while the wake is past step 0 (#431, and
    /// the lead's decision on G6).
    refused: bool,
    /// The one live copy's folder.
    loc: Loc,
    /// A snooze row is left, with this wake time.
    row: Option<i64>,
    /// Whether the live copy is read; `None` where it doesn't matter.
    seen: Option<bool>,
}

fn expect(at: At, user: User) -> Expect {
    let moving = at != At::Step0;
    // The copy exists once the MOVE or COPY went out: a read on the
    // original in Snoozed doesn't reach it (a known limit, not this fix's).
    let copy_made = matches!(at, At::Step1Sent | At::Step2);
    match user {
        // Woken (the message was unread when snoozed, so it stays unread).
        User::Nothing | User::Unsnooze => Expect {
            refused: false,
            loc: Loc::Inbox,
            row: None,
            seen: Some(false),
        },
        // Read after the wake started: the read wins over the wake's
        // "mark unread" (#459), where it reached the message being moved.
        User::Read => Expect {
            refused: false,
            loc: Loc::Inbox,
            row: None,
            seen: if copy_made { None } else { Some(true) },
        },
        User::Resnooze if moving => Expect {
            refused: true,
            ..expect(at, User::Nothing)
        },
        User::Resnooze => Expect {
            refused: false,
            loc: Loc::Snoozed,
            row: Some(LATER_WAKE),
            seen: None,
        },
        User::Delete | User::Move if moving => Expect {
            refused: true,
            ..expect(at, User::Nothing)
        },
        User::Delete => Expect {
            refused: false,
            loc: Loc::Trash,
            row: None,
            seen: None,
        },
        User::Move => Expect {
            refused: false,
            loc: Loc::Archive,
            row: None,
            seen: None,
        },
    }
}

async fn connect(imap: &FakeImap) -> Session {
    let mut config = ImapConfig::new("localhost", imap.addr().port(), ImapSecurity::ImplicitTls)
        .with_extra_root_cert(imap.root_cert_der().to_vec());
    config.command_timeout = Duration::from_secs(60);
    Session::connect(&config, USER, &Secret::new(PASSWORD))
        .await
        .expect("connects")
}

async fn sync_all(imap: &FakeImap, store: &Store, account: AccountId) {
    let mut session = connect(imap).await;
    let listed = session.list_folders().await.expect("list");
    let ids = save_folders(store, account, &listed).expect("save");
    for id in ids {
        let mut syncer =
            FolderSyncer::new(store, &mut session, &Quiet, account, SyncConfig::default());
        Box::pin(syncer.sync_changes(id)).await.expect("sync");
    }
    session.logout().await;
}

/// One sync's local changes, as the app sends them.
async fn push(imap: &FakeImap, store: &Store, account: AccountId, now: i64) {
    let mut session = connect(imap).await;
    Box::pin(push_local_changes(
        store,
        &mut session,
        &Quiet,
        account,
        now,
    ))
    .await;
    session.logout().await;
}

/// The same, but with the snooze rows as read before the user acted: the
/// race the table is about.
async fn push_stale(
    imap: &FakeImap,
    store: &Store,
    account: AccountId,
    now: i64,
    read: Vec<Snooze>,
) {
    let mut session = connect(imap).await;
    let _ = Box::pin(crate::actions::replay(
        store,
        &mut session,
        &Quiet,
        account,
        now,
    ))
    .await;
    let work = read
        .into_iter()
        .filter(|s| s.state != SnoozeState::Snoozed)
        .collect();
    Box::pin(sync_snooze_rows(store, &mut session, &Quiet, now, work))
        .await
        .expect("sync snoozes");
    session.logout().await;
}

fn raw() -> Vec<u8> {
    format!(
        "From: Bob <bob@example.net>\r\nTo: {USER}\r\nSubject: Lunch\r\n\
         Message-ID: <{MID}>\r\n\r\nHello\r\n"
    )
    .into_bytes()
}

fn folder(store: &Store, account: AccountId, name: &str) -> Option<FolderId> {
    store
        .list_folders(account)
        .expect("folders")
        .into_iter()
        .find(|f| f.details.remote_name == name)
        .map(|f| f.id)
}

fn shown(store: &Store, folder: FolderId) -> Vec<MessageId> {
    store
        .list_messages(folder, 50, 0)
        .expect("list")
        .into_iter()
        .map(|m| m.id)
        .collect()
}

/// A server with Lunch, unread, in the Inbox, snoozed by this install and
/// now due: a `to_wake` row at step 0, the message in Snoozed with its
/// keyword.
async fn due(server: Server) -> (FakeImap, Store, AccountId) {
    let mut builder = FakeImap::builder()
        .user(USER, PASSWORD)
        .folder("INBOX", vec![raw()])
        .folder("Archive", Vec::new())
        .folder("Trash", Vec::new())
        .special_use("Archive", "\\Archive")
        .special_use("Trash", "\\Trash");
    for q in server.quirks() {
        builder = builder.quirk(*q);
    }
    let imap = builder.start().await.expect("imap");
    let store = Store::open_in_memory().expect("store");
    let account = store
        .insert_account(&NewAccount {
            display_name: "Alice".into(),
            email: USER.into(),
            imap_host: "localhost".into(),
            imap_port: 993,
            imap_security: Security::Tls,
            smtp_host: "localhost".into(),
            smtp_port: 465,
            smtp_security: Security::Tls,
            username: USER.into(),
        })
        .expect("account");
    Box::pin(sync_all(&imap, &store, account)).await;
    let inbox = folder(&store, account, "INBOX").expect("inbox");
    snooze(&store, &shown(&store, inbox), WAKE, &Quiet).expect("snooze");
    Box::pin(push(&imap, &store, account, NOW)).await;
    Box::pin(sync_all(&imap, &store, account)).await;
    assert_eq!(wake_due(&store, ACT, &Quiet).expect("wake").len(), 1);
    (imap, store, account)
}

/// Puts the server and the row where a wake at `at` leaves them.
async fn go_to(imap: &FakeImap, store: &Store, account: AccountId, server: Server, at: At) {
    let row = list_snoozed(store, Some(account)).expect("rows")[0].clone();
    if at == At::Step0 {
        return;
    }
    let uidnext = imap.mailbox().uidnext("INBOX").await;
    assert!(store
        .start_snooze_move(row.id, SnoozeState::ToWake, uidnext)
        .expect("step 1"));
    let mailbox = imap.mailbox();
    match at {
        At::Step0 | At::Step1Unsent => {}
        At::Step1Sent if server.has_move() => {
            mailbox
                .move_messages(SNOOZED_FOLDER_NAME, |_| true, "INBOX")
                .await;
        }
        At::Step1Sent => {
            mailbox.copy(SNOOZED_FOLDER_NAME, |_| true, "INBOX").await;
        }
        At::Step2 => {
            mailbox.copy(SNOOZED_FOLDER_NAME, |_| true, "INBOX").await;
            store.set_snooze_step(row.id, 2, None).expect("step 2");
        }
    }
}

/// Does `user` to the message as it shows in Snoozed. Returns whether it
/// was refused.
fn act(store: &Store, account: AccountId, user: User) -> bool {
    let snoozed = folder(store, account, SNOOZED_FOLDER_NAME).expect("Snoozed");
    let Some(message) = store.find_message_by_header(snoozed, MID).expect("find") else {
        panic!("no local row in Snoozed");
    };
    let archive = folder(store, account, "Archive").expect("Archive");
    match user {
        User::Nothing => false,
        User::Resnooze => snooze(store, &[message], LATER_WAKE, &Quiet).is_err(),
        User::Unsnooze => unsnooze(store, &[message], ACT, &Quiet).is_err(),
        User::Delete => apply_action(store, &[message], Action::Delete, ACT, &Quiet).is_err(),
        User::Move => {
            apply_action(store, &[message], Action::Move { to: archive }, ACT, &Quiet).is_err()
        }
        User::Read => apply_action(store, &[message], Action::MarkRead, ACT, &Quiet).is_err(),
    }
}

/// Whether a server message is this one.
fn is_lunch(m: &mailcrane_testkit::Message) -> bool {
    String::from_utf8_lossy(&m.raw).contains(MID)
}

fn deleted(m: &mailcrane_testkit::Message) -> bool {
    m.flags.iter().any(|f| f == "\\Deleted")
}

/// Checks one cell's end state; returns what's wrong, if anything.
async fn check(
    imap: &FakeImap,
    store: &Store,
    account: AccountId,
    server: Server,
    want: &Expect,
    refused: bool,
) -> Vec<String> {
    let mut wrong = Vec::new();
    if refused != want.refused {
        wrong.push(format!(
            "the user's action was {}refused",
            if refused { "" } else { "not " }
        ));
    }
    let mailbox = imap.mailbox();
    // I1: exactly one live copy, where expected.
    let mut live = Vec::new();
    let mut flagged_deleted = Vec::new();
    for loc in FOLDERS {
        for m in mailbox
            .messages(loc.name())
            .await
            .iter()
            .filter(|m| is_lunch(m))
        {
            if deleted(m) {
                flagged_deleted.push(loc);
            } else {
                live.push((loc, m.flags.clone()));
            }
        }
    }
    let places: Vec<Loc> = live.iter().map(|(l, _)| *l).collect();
    if places != [want.loc] {
        wrong.push(format!(
            "live copies in {places:?}, want one in {:?}",
            want.loc
        ));
    }
    // A MOVE is never degraded to a COPY, and UID EXPUNGE removes the
    // original: only a server with neither can keep a flagged original.
    if server != Server::Bare && !flagged_deleted.is_empty() {
        wrong.push(format!("a \\Deleted copy left in {flagged_deleted:?}"));
    }
    // The read state of the live copy.
    if let (Some(seen), Some((_, flags))) = (want.seen, live.first()) {
        let is_seen = flags.iter().any(|f| f == "\\Seen");
        if is_seen != seen {
            wrong.push(format!(
                "the live copy is {}read",
                if is_seen { "" } else { "un" }
            ));
        }
    }
    // I2: a snooze keyword in Snoozed only while a row exists.
    let rows = list_snoozed(store, Some(account)).expect("rows");
    let keyword_in_snoozed = mailbox
        .messages(SNOOZED_FOLDER_NAME)
        .await
        .iter()
        .filter(|m| is_lunch(m) && !deleted(m))
        .any(|m| m.flags.iter().any(|f| f.starts_with(KEYWORD_PREFIX)));
    if keyword_in_snoozed && rows.is_empty() {
        wrong.push("a snooze keyword in Snoozed with no row".into());
    }
    // The row left, if any.
    let left: Option<i64> = rows.first().map(|r| r.wake_at);
    if rows.len() > 1 || left != want.row {
        wrong.push(format!(
            "rows {:?}, want {:?}",
            rows.iter()
                .map(|r| (r.state, r.step, r.wake_at))
                .collect::<Vec<_>>(),
            want.row
        ));
    }
    // Locally: shown in exactly the folder the server has it in; a
    // \Deleted copy never shows.
    let mut shown_in = Vec::new();
    for loc in FOLDERS {
        if let Some(f) = folder(store, account, loc.name()) {
            if let Some(id) = store.find_message_by_header(f, MID).expect("find") {
                if shown(store, f).contains(&id) {
                    shown_in.push(loc);
                }
            }
        }
    }
    let local_want: Vec<Loc> = if want.row.is_some() && want.loc == Loc::Snoozed {
        // Snoozed: in no folder list (the snoozed view lists it).
        Vec::new()
    } else {
        vec![want.loc]
    };
    // A server with neither MOVE nor UIDPLUS keeps every moved original
    // flagged \Deleted, and the folder sync lists such messages: a sync
    // gap outside the snooze code, #519.
    if server != Server::Bare && shown_in != local_want {
        wrong.push(format!(
            "shown locally in {shown_in:?}, want {local_want:?}"
        ));
    }
    wrong
}

/// Runs one cell and returns what's wrong with it.
async fn run_cell(server: Server, at: At, user: User) -> Vec<String> {
    let (imap, store, account) = Box::pin(due(server)).await;
    Box::pin(go_to(&imap, &store, account, server, at)).await;
    let read = list_snoozed(&store, Some(account)).expect("rows");
    let refused = act(&store, account, user);
    // The sync that read the row before the user acted.
    Box::pin(push_stale(&imap, &store, account, ACT, read)).await;
    // Then ordinary syncs until it settles, never reaching LATER_WAKE.
    for round in 1..=3_i64 {
        let now = ACT + round * 6 * 3600;
        Box::pin(sync_all(&imap, &store, account)).await;
        let _ = wake_due(&store, now, &Quiet);
        Box::pin(push(&imap, &store, account, now)).await;
    }
    Box::pin(sync_all(&imap, &store, account)).await;
    Box::pin(check(
        &imap,
        &store,
        account,
        server,
        &expect(at, user),
        refused,
    ))
    .await
}

/// The whole table. Every failing cell is listed, by id, with what's
/// wrong in it.
#[tokio::test]
async fn every_wake_step_with_every_user_action_on_every_server() {
    let mut failures = Vec::new();
    for server in Server::ALL {
        for at in At::ALL {
            // Step 2 is the COPY fallback's.
            if at == At::Step2 && server.has_move() {
                continue;
            }
            for user in User::ALL {
                let wrong = Box::pin(run_cell(server, at, user)).await;
                if !wrong.is_empty() {
                    failures.push(format!(
                        "{}/{}/{}: {}",
                        at.id(),
                        user.id(),
                        server.id(),
                        wrong.join("; ")
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} cell(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// #450: install A records step 1 and drops before its MOVE; install B
/// wakes the message and the user reads it. A's next sync must not mark
/// B's copy unread, on any server.
#[tokio::test]
async fn another_installs_woken_copy_keeps_its_read_state() {
    for server in Server::ALL {
        let (imap, store, account) = Box::pin(due(server)).await;
        Box::pin(go_to(&imap, &store, account, server, At::Step1Unsent)).await;
        // Install B: moves it back to the Inbox and strips the keyword; the
        // user reads it there.
        let mailbox = imap.mailbox();
        mailbox
            .move_messages(SNOOZED_FOLDER_NAME, |_| true, "INBOX")
            .await;
        for m in mailbox
            .messages("INBOX")
            .await
            .iter()
            .filter(|m| is_lunch(m))
        {
            for k in m.flags.iter().filter(|f| f.starts_with(KEYWORD_PREFIX)) {
                mailbox.remove_flag("INBOX", m.uid, k).await;
            }
            mailbox.add_flag("INBOX", m.uid, "\\Seen").await;
        }
        Box::pin(push(&imap, &store, account, ACT + 3600)).await;
        let read_state: Vec<bool> = mailbox
            .messages("INBOX")
            .await
            .iter()
            .filter(|m| is_lunch(m) && !deleted(m))
            .map(|m| m.flags.iter().any(|f| f == "\\Seen"))
            .collect();
        assert_eq!(
            read_state,
            [true],
            "{}: B's copy marked unread",
            server.id()
        );
        assert!(
            list_snoozed(&store, Some(account))
                .expect("rows")
                .is_empty(),
            "{}: the row is done",
            server.id()
        );
    }
}

/// The lead's condition on G6: a wake the server refuses goes back to
/// step 0 (and in the end is given up), so a move the user makes then
/// is not refused, and nothing stays locked.
#[tokio::test]
async fn after_a_refused_wake_the_user_can_move_the_message() {
    let (imap, store, account) = Box::pin(due(Server::Full)).await;
    Box::pin(go_to(&imap, &store, account, Server::Full, At::Step1Unsent)).await;
    let row = list_snoozed(&store, Some(account)).expect("rows")[0].clone();
    // What a refused wake leaves: back at step 0, deferred.
    store.reset_snooze_step(row.id, ACT).expect("reset");
    assert!(!act(&store, account, User::Move), "the move is allowed");
}

/// #459 (invariant 5): every write in the sync that finishes, advances or
/// deletes a snooze row goes through a guarded `_if` helper. The plain
/// ones are left for the store's own tests; the sync calls none of them.
#[test]
fn the_sync_makes_no_unguarded_snooze_writes() {
    let source = include_str!("../server.rs");
    for raw in [
        ".finish_snooze(",
        ".delete_snooze(",
        ".set_snooze_step(",
        ".start_snooze_move(",
        ".start_snooze_rollback(",
        ".reset_snooze_step(",
    ] {
        assert!(!source.contains(raw), "server.rs calls {raw}");
    }
}
