# Mailcrane roadmap

This is the plan. **The live to-do list is the GitHub issues**: `scripts/roadmap_to_issues.py` turns each task below into an issue once, and after that the issue is the place to claim it (comment `/claim`), discuss it and track it. Each issue keeps the task ID in its title, e.g. `[M1-04] IMAP connection, login and folder list`.

**Markers:** 🟢 good first issue · 🤖 agent-ready (clear spec, easy to test) · 🔒 security-sensitive · 🧑 needs a human (real devices, accounts or judgement)

---

## M0 — Foundations

Goal: anyone can clone, build an empty app on both platforms and run CI.

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
- [ ] **M0-14** Fuzz targets (`cargo fuzz`) for the MIME and IMAP parsers, run nightly in CI 🤖
- [x] **M0-15** Release script that gathers `docs/changelog.d/` fragments into release notes 🟢🤖
- [ ] **M0-16** TestFlight internal builds from CI (macOS runner), so iOS can be tested on real devices without a Mac 🔒🧑
- [ ] **M0-17** Cross-platform CI on every PR: Android on Linux, iOS on macOS, the core on Windows 🤖

## M1 — Read mail

Goal: add an IMAP account and read your inbox, online and offline.

- [x] **M1-01** Store schema v1: accounts, folders, messages, threads, attachments metadata, with migrations 🤖
- [x] **M1-02** Secure credential storage: Android Keystore and iOS Keychain, behind one FFI interface 🔒
- [ ] **M1-03** Manual account setup: IMAP/SMTP host, port, security, username, password 🤖
- [ ] **M1-04** IMAP connection, login and folder list (TLS via rustls) 🔒🤖
- [ ] **M1-05** Initial sync: fetch headers for recent messages, paginate older ones on demand 🤖
- [ ] **M1-06** Incremental sync with CONDSTORE/QRESYNC, falling back to UID comparison 🤖
- [ ] **M1-07** IMAP IDLE for near-real-time updates while the app is open 🤖
- [ ] **M1-08** MIME parsing: multipart, charsets, encoded words, inline and attached files, with fixtures 🤖
- [ ] **M1-09** HTML sanitiser: strip scripts and trackers, block remote content by default 🔒🤖
- [ ] **M1-10** Message list screen, Android 🤖
- [ ] **M1-11** Message list screen, iOS 🤖
- [ ] **M1-12** Message view screen with "load remote images" button, both platforms 🤖
- [ ] **M1-13** Folder drawer / sidebar, both platforms 🟢🤖
- [ ] **M1-14** Read, unread, flag, archive, delete and move, synced back to the server 🤖
- [ ] **M1-15** Automatic account setup: Mozilla ISPDB (Thunderbird autoconfig), `/.well-known/autoconfig`, SRV records 🤖
- [ ] **M1-16** Empty states using the illustrations in `design/` 🟢🤖

## M2 — Send mail

- [x] **M2-01** SMTP sending with STARTTLS and implicit TLS 🔒🤖
- [ ] **M2-02** Compose screen: to/cc/bcc, subject, plain-text body 🤖
- [ ] **M2-03** Reply, reply-all and forward, with correct `In-Reply-To` and `References` headers 🤖
- [ ] **M2-04** Attachments: add, send, preview and save 🤖
- [ ] **M2-05** Drafts: autosave locally and to the server's Drafts folder 🤖
- [ ] **M2-06** Offline outbox with retry 🤖
- [ ] **M2-07** Save sent messages to Sent (skip for providers that do it themselves, such as Gmail) 🤖
- [ ] **M2-08** Contact autocomplete from the device's contacts and past correspondents 🤖
- [ ] **M2-09** Rich-text compose (bold, italic, lists, links) 🤖
- [ ] **M2-10** Signatures, per account 🟢🤖

## M3 — Providers and sign-in

- [ ] **M3-01** OAuth 2.0 framework with PKCE, using the system browser (AppAuth on Android, ASWebAuthenticationSession on iOS) 🔒
- [ ] **M3-02** Gmail: OAuth, XOAUTH2, and labels mapped to folders 🔒🧑 (needs Google app verification)
- [ ] **M3-03** Microsoft 365 and Outlook.com: OAuth and XOAUTH2 🔒🧑
- [ ] **M3-04** JMAP client (Fastmail, Stalwart and others) 🤖
- [ ] **M3-05** iCloud, Yahoo and AOL guided app-password setup 🧑
- [ ] **M3-06** Provider test matrix: a doc recording which providers have been tested, by whom, and when 🧑

## M4 — The features that make it good

- [ ] **M4-01** Multiple accounts and a unified inbox 🤖
- [ ] **M4-02** Conversation threading (JWZ algorithm plus Gmail thread IDs) 🤖
- [ ] **M4-03** Offline full-text search (SQLite FTS5), plus server-side IMAP SEARCH 🤖
- [ ] **M4-04** Configurable swipe actions 🟢🤖
- [ ] **M4-05** Push notifications: Android foreground IDLE service and UnifiedPush; iOS background fetch. Write an ADR on whether iOS needs a self-hostable push relay first 🧑
- [ ] **M4-06** Snooze (stored locally, synced via a hidden folder or IMAP keyword) 🤖
- [ ] **M4-07** Scheduled send 🤖
- [ ] **M4-08** Undo send (short delay before sending) 🟢🤖
- [ ] **M4-09** Local rules and filters 🤖
- [ ] **M4-10** One-tap unsubscribe (`List-Unsubscribe`, RFC 8058) 🤖
- [ ] **M4-11** Home-screen widgets (Android Glance, iOS WidgetKit) 🤖
- [ ] **M4-12** Dark mode and dynamic colour (Material You) 🟢🤖
- [ ] **M4-13** Tablet and iPad layouts (list and detail side by side) 🤖

## M5 — Privacy and security

- [ ] **M5-01** Tracking-pixel detection and a "trackers blocked" indicator 🤖
- [ ] **M5-02** Strip tracking parameters from links (utm_ and similar) before opening 🟢🤖
- [ ] **M5-03** App lock with biometrics or PIN 🔒
- [ ] **M5-04** OpenPGP: read and send encrypted and signed mail, Autocrypt, key management 🔒
- [ ] **M5-05** S/MIME 🔒
- [ ] **M5-06** Independent security review before 1.0 🧑

## M6 — Release

- [ ] **M6-01** Accessibility pass: TalkBack, VoiceOver, dynamic text sizes, contrast 🧑
- [ ] **M6-02** Translations through Weblate 🧑
- [ ] **M6-03** Release builds and signing in CI 🔒🧑
- [ ] **M6-04** F-Droid listing (fully free build, no Google services) 🧑
- [ ] **M6-05** Google Play beta and release 🧑
- [ ] **M6-06** TestFlight beta and App Store release 🧑
- [ ] **M6-07** Website and user docs (mailcrane.app) 🤖
- [ ] **M6-08** Privacy policy (short and honest: we collect nothing) 🧑

## Before going public (from the name decision)

- [ ] **P-01** Check and register domains: mailcrane.app / .com / .org / .dev 🧑
- [x] **P-02** GitHub repo created: `level-98/mailcrane` 🧑
- [ ] **P-03** Google Play and App Store name search 🧑
- [ ] **P-04** Trademark search: UKIPO, USPTO, EUIPO (classes 9 and 38) 🧑
- [ ] **P-05** Reserve app IDs (`app.mailcrane` / `org.mailcrane.android`) and the F-Droid package name 🧑
