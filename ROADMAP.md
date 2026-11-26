# Mailcrane roadmap

This is the plan. **The live to-do list is the GitHub issues**: `scripts/roadmap_to_issues.py` turns each task below into an issue once, and after that the issue is the place to claim it (comment `/claim`), discuss it and track it. Each issue keeps the task ID in its title, e.g. `[M1-04] IMAP connection, login and folder list`.

**Markers:** 🟢 good first issue · 🤖 agent-ready (clear spec, easy to test) · 🔒 security-sensitive · 🧑 needs a human (real devices, accounts or judgement)

**Android first, iOS after release.** Mailcrane ships on Android first. Until the Android 1.0 release (M6), app work in M1–M6 means the Android app. The iOS app stays at its M0 shell and catches up in **M7**, after release (see [decision 0002](docs/decisions/0002-android-first-ios-after-release.md)). The Rust core is still shared, so mail logic built now will already be there for iOS.

Task IDs never change when a task moves milestone: `M1-11` keeps its ID (and its issue) now that it sits under M7.

**Product principles.** Check new features against these:

- Mail only: no bundled chat, video calls or ads, ever.
- Mail goes straight between the device and the user's mail server. No credentials or message content on our servers; any push relay must be self-hostable and carry no content.
- Every account type gets the same features: search, rules, notifications and unified views work the same for IMAP, JMAP, Gmail and Microsoft.
- Simple defaults, with power-user options under "Advanced".
- Automatic sorting or categorising is opt-in. Badge counts and notifications follow the user's settings, not a category.
- If AI is ever added: off by default, on-device or bring-your-own-key, small tasks the user can check, and never acting silently.
- Free, with nothing behind a subscription.

---

## M0 — Foundations

Goal: anyone can clone, build an empty Android app and run CI. (The iOS shell from M0-07 also builds, on a Mac.)

- [x] **M0-01** Add LICENSE (MPL-2.0), CODE_OF_CONDUCT.md (Contributor Covenant 2.1), branch protection on `main` 🧑
- [x] **M0-02** Run `scripts/roadmap_to_issues.py` to create the labels and one issue per task; enable Discussions 🧑
- [x] **M0-03** Add the agent's Claude token (`CLAUDE_CODE_OAUTH_TOKEN`, or `ANTHROPIC_API_KEY`) as a secret of the `main`-only `agent` environment so the `agent-go` workflow works (or delete `.github/workflows/agent.yml`) 🧑
- [x] **M0-04** Rust workspace skeleton in `core/` with empty crates: `mailcrane-core`, `-imap`, `-smtp`, `-jmap`, `-mime`, `-store`, `-ffi`, `-testkit`; `#![forbid(unsafe_code)]` and the no-panic clippy lints 🤖
- [x] **M0-05** UniFFI setup: expose a `hello()` function, generate Kotlin and Swift bindings 🤖
- [x] **M0-06** Android app shell: Compose + Material 3, Hilt, calls `hello()` through the bindings, ktlint 🤖
- [x] **M0-07** iOS app shell: SwiftUI, XCFramework build of the core, calls `hello()`, SwiftLint 🤖
- [x] **M0-08** `cargo xtask ci`: one command for fmt, clippy, tests, cargo-deny, and Android/iOS checks when installed; CI (GitHub Actions) just runs it 🤖
- [x] **M0-09** `cargo deny` config allowing only MPL-2.0-compatible licences 🟢🤖
- [x] **M0-10** Generate the Compose theme and SwiftUI colour assets from `design/colors.json` (the brand pack itself is already in `design/`) 🤖
- [x] **M0-11** `mailcrane-testkit`: in-process fake IMAP, SMTP and JMAP servers that can misbehave on purpose (drop connections, malformed replies, Gmail-style quirks) 🤖
- [x] **M0-12** Write `docs/decisions/0001-tech-stack.md` recording the Rust core + native UI decision, including the portability rule in `docs/architecture.md` (the core must also build on Windows) 🧑
- [x] **M0-13** Dependabot or Renovate for Cargo, Gradle and Swift packages 🟢🤖
- [x] **M0-14** Fuzz targets (`cargo fuzz`) for the MIME and IMAP parsers, run nightly in CI 🤖
- [x] **M0-15** Release script that gathers `docs/changelog.d/` fragments into release notes 🟢🤖
- [x] **M0-17** Cross-platform CI on every PR: Android on Linux and the core on Windows (iOS on macOS comes back in M7-02) 🤖

## M1 — Read mail

Goal: add an IMAP account and read your inbox, online and offline.

- [x] **M1-01** Store schema v1: accounts, folders, messages, threads, attachments metadata, with migrations 🤖
- [x] **M1-02** Secure credential storage: Android Keystore and iOS Keychain, behind one FFI interface 🔒
- [x] **M1-03** Manual account setup: IMAP/SMTP host, port, security, username, password 🤖
- [x] **M1-04** IMAP connection, login and folder list (TLS via rustls) 🔒🤖
- [x] **M1-05** Initial sync: fetch headers for recent messages, paginate older ones on demand 🤖
- [x] **M1-06** Incremental sync with CONDSTORE/QRESYNC, falling back to UID comparison 🤖
- [x] **M1-07** IMAP IDLE for near-real-time updates while the app is open 🤖
- [x] **M1-08** MIME parsing: multipart, charsets, encoded words, inline and attached files, with fixtures 🤖
- [x] **M1-09** HTML sanitiser: strip scripts and trackers, block remote content by default 🔒🤖
- [x] **M1-10** Message list screen, Android 🤖
- [x] **M1-12** Message view screen with "load remote images" button, Android 🤖
- [x] **M1-13** Folder drawer, Android 🟢🤖
- [x] **M1-14** Read, unread, flag, archive, delete and move, synced back to the server 🤖
- [x] **M1-15** Automatic account setup: Mozilla ISPDB (Thunderbird autoconfig), `/.well-known/autoconfig`, SRV records 🤖
- [x] **M1-16** Empty states using the illustrations in `design/` 🟢🤖

## M2 — Send mail

- [x] **M2-01** SMTP sending with STARTTLS and implicit TLS 🔒🤖
- [x] **M2-02** Compose screen: to/cc/bcc, subject, plain-text body 🤖
- [x] **M2-03** Reply, reply-all and forward, with correct `In-Reply-To` and `References` headers 🤖
- [x] **M2-04** Attachments: add, send, preview and save 🤖
- [x] **M2-05** Drafts: autosave locally and to the server's Drafts folder 🤖
- [x] **M2-06** Offline outbox with retry 🤖
- [x] **M2-07** Save sent messages to Sent (skip for providers that do it themselves, such as Gmail) 🤖
- [x] **M2-08** Contact autocomplete from the device's contacts and past correspondents 🤖
- [x] **M2-09** Rich-text compose (bold, italic, lists, links) 🤖
- [x] **M2-10** Signatures, per account 🟢🤖

## M3 — Providers and sign-in

- [x] **M3-01** OAuth 2.0 framework with PKCE, using the system browser (AppAuth on Android; iOS sign-in is M7-09) 🔒
- [ ] **M3-02** Gmail: OAuth, XOAUTH2, and labels mapped to folders 🔒🧑 (needs Google app verification; code done, real-account check and verification still to do)
  - Start verification early and budget for the yearly CASA security assessment: other open-source clients have been shown "App blocked" while waiting. Keep the app-password IMAP route working as a fallback.
- [ ] **M3-03** Microsoft 365 and Outlook.com: OAuth and XOAUTH2 🔒🧑
- [x] **M3-04** JMAP client (Fastmail, Stalwart and others) 🤖
- [ ] **M3-05** iCloud, Yahoo and AOL guided app-password setup 🧑
- [ ] **M3-06** Provider test matrix: a doc recording which providers have been tested, by whom, and when 🧑

## M4 — The features that make it good

- [x] **M4-01** Multiple accounts and a unified inbox 🤖
- [x] **M4-02** Conversation threading (JWZ algorithm plus Gmail thread IDs) 🤖
- [x] **M4-03** Offline full-text search (SQLite FTS5), plus server-side IMAP SEARCH 🤖
- [x] **M4-04** Configurable swipe actions 🟢🤖
- [ ] **M4-05** Push notifications on Android: foreground IDLE service and UnifiedPush (iOS is M7-10) 🧑
  - Notification settings per account and per folder. Folders that rules move mail into notify by default.
  - Quiet hours per account, which can also pause syncing (e.g. work account at weekends).
  - The user picks the notification actions (mark as read, archive, delete, reply).
  - Opening from a notification shows the cached message at once and syncs behind it.
  - Battery and sync tested on real devices; sync status visible in the app.
- [x] **M4-06** Snooze (stored locally, synced via a hidden folder or IMAP keyword) 🤖
- [x] **M4-07** Scheduled send 🤖
- [x] **M4-08** Undo send (short delay before sending) 🟢🤖
- [x] **M4-09** Local rules and filters 🤖
- [x] **M4-10** One-tap unsubscribe (`List-Unsubscribe`, RFC 8058) 🤖
- [x] **M4-11** Home-screen widgets (Android Glance) 🤖
- [x] **M4-12** Dark mode and dynamic colour (Material You) 🟢🤖
- [x] **M4-13** Tablet layouts (list and detail side by side) 🤖
- [x] **M4-14** Multi-select and bulk actions in the message list: long-press to start, then read, unread, flag, archive, delete and move on all selected 🤖
- [ ] **M4-15** Mark-as-read setting (on open, after a few seconds, or only by hand) and an "after delete or move" setting (next message, previous message, or back to the list) 🟢🤖
- [ ] **M4-16** Choose which view the app opens on: unified inbox, a chosen account's inbox, or the last one used 🟢🤖
- [ ] **M4-17** Templates and canned replies: save, insert while composing, edit and delete 🟢🤖
- [ ] **M4-18** Settings backup and restore to an encrypted, password-protected file: accounts (server settings only, never passwords or tokens), signatures, rules, templates and app settings 🔒🤖
- [x] **M4-19** Date headers in the message list: Today, Yesterday, This week, then by month 🟢🤖
- [x] **M4-20** Let users choose each account's colour (the automatic colours from M4-01 stay the default) 🟢🤖
- [x] **M4-21** Search filter chips (from, has attachment, unread, flagged, date range, account) built on the existing search syntax, adding `account:` to it 🟢🤖
- [x] **M4-22** Create, rename, move and delete folders from the phone 🤖
- [x] **M4-23** Server-side rules through ManageSieve (RFC 5804), so rules run when the phone is off; local rules (M4-09) stay for servers without it 🤖
- [x] **M4-24** Sender pictures in the message list (initials on a stable colour, contact photos only after opting in, never from the network), and a Compact mailbox density without them 🤖
- [x] **M4-25** Download recent inbox messages in the background after each sync (unmetered network by default, not in Battery Saver, last 14 days, newest 200, up to 1 MB each), so they open at once and offline; setting under Advanced 🤖
- [x] **M4-26** Manage folders on JMAP accounts (create, rename, move, delete through Mailbox/set) 🤖

## M5 — Privacy and security

- [x] **M5-01** Tracking-pixel detection and a "trackers blocked" indicator 🤖
- [x] **M5-02** Strip tracking parameters from links (utm_ and similar) before opening 🟢🤖
- [x] **M5-03** App lock with biometrics or PIN, Android 🔒
- [ ] **M5-04** OpenPGP: read and send encrypted and signed mail, Autocrypt, key management 🔒
- [ ] **M5-05** S/MIME 🔒
- [ ] **M5-06** Independent security review before 1.0 🧑
- [x] **M5-07** Phishing help: show a link's real destination before opening it, show SPF/DKIM/DMARC results from `Authentication-Results`, and warn when the display name pretends to be a different address 🔒🤖
- [x] **M5-08** Per-account provider authentication server name, so Postfix/OpenDMARC users get the SPF/DKIM/DMARC chip and one-click unsubscribe 🔒🤖

## M6 — Release

Goal: Mailcrane 1.0 on Android, through F-Droid and Google Play.

- [ ] **M6-01** Accessibility pass on Android: TalkBack, font scaling, contrast 🧑
- [ ] **M6-02** Translations through Weblate 🧑
- [ ] **M6-03** Android release builds and signing in CI 🔒🧑
- [ ] **M6-04** F-Droid listing (fully free build, no Google services) 🧑
- [ ] **M6-05** Google Play beta and release 🧑
- [x] **M6-07** Website and user docs (mailcrane.app) 🤖
- [ ] **M6-08** Privacy policy (short and honest: we collect nothing) 🧑

## M7 — iOS app

Goal: bring the iOS app up to the Android 1.0 feature set and ship it on the App Store. Starts after M6. The mail logic is already in the shared core, so this is mostly SwiftUI screens and Apple platform features. Day-to-day iOS work needs macOS (Xcode and the simulator), on a Mac, a rented cloud Mac or CI.

- [ ] **M7-01** Decide how iOS gets built and tested (own Mac, cloud Mac, or CI only) and record it as an ADR 🧑
- [ ] **M7-02** Turn the iOS check back on for every PR (`.github/workflows/ios.yml`) and bring the iOS shell up to date with the core: bindings, SwiftLint, tests 🤖
- [ ] **M0-16** TestFlight internal builds from CI (macOS runner), so iOS can be tested on real devices without a Mac 🔒🧑
- [ ] **M1-11** Message list screen, iOS 🤖
- [ ] **M7-03** Message view screen with "load remote images" button, iOS 🤖
- [ ] **M7-04** Folder sidebar, iOS 🟢🤖
- [ ] **M7-05** Account setup screens, iOS: manual and automatic (the iOS side of M1-03 and M1-15) 🤖
- [ ] **M7-06** Compose screens, iOS: compose, reply and forward, attachments, drafts, rich text and signatures (the iOS side of M2) 🤖
- [ ] **M7-07** Everyday features, iOS: multiple accounts, threading, search, swipe actions, snooze, scheduled and undo send, rules, unsubscribe and empty states (the iOS side of M1 and M4) 🤖
- [ ] **M7-08** Privacy features, iOS: trackers-blocked indicator, link cleaning, OpenPGP and S/MIME screens (the iOS side of M5) 🤖
- [ ] **M7-09** OAuth sign-in on iOS with ASWebAuthenticationSession, using the M3-01 framework (Gmail and Microsoft) 🔒
- [ ] **M7-10** Push notifications on iOS: background fetch. Write an ADR on whether iOS needs a self-hostable push relay first 🧑
- [ ] **M7-11** App lock with Face ID, Touch ID or passcode, iOS 🔒
- [ ] **M7-12** Home-screen widgets (WidgetKit) 🤖
- [ ] **M7-13** iPad layout (list and detail side by side) 🤖
- [ ] **M7-14** Accessibility pass on iOS: VoiceOver, Dynamic Type, contrast 🧑
- [ ] **M7-15** iOS release builds and signing in CI 🔒🧑
- [ ] **M6-06** TestFlight beta and App Store release 🧑

## M8 — After 1.0

Good ideas that wait until Android 1.0 is out. Issues get the `post-1.0` label: please don't claim them yet.

- [ ] **M8-01** Attachments browser: every file across all mail, filterable by type, sender and account 🤖
- [ ] **M8-02** Multi-colour flags, synced through IMAP keywords (Thunderbird-compatible where possible) 🤖
- [ ] **M8-03** Calendar invites: show an `.ics` invite (`METHOD:REQUEST`) and reply accept, tentative or decline, without building a calendar app 🤖
- [ ] **M8-04** Read receipts (MDN, RFC 8098): ask before sending one, optionally request one; show and set the priority header 🤖

## Before going public (from the name decision)

- [x] **P-01** Check and register domains: mailcrane.app / .com / .org / .dev 🧑
- [x] **P-02** GitHub repo created: `level-98/mailcrane` 🧑
- [x] **P-03** Google Play and App Store name search 🧑
- [x] **P-04** Trademark search: UKIPO, USPTO, EUIPO (classes 9 and 38) 🧑
- [ ] **P-05** Reserve app IDs (`app.mailcrane` / `org.mailcrane.android`) and the F-Droid package name 🧑
