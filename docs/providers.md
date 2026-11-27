# Mail providers

Which mail providers Mailcrane has been tried with, by whom and when, and what worked (M3-06, issue #48). Check here before you try the app with your provider. If you test one, please [send a report](https://github.com/level-98/mailcrane/issues/new?template=provider_report.yml), even when everything worked.

Mailcrane is **pre-alpha**. An empty row means nobody has reported on that provider yet, not that it fails.

## The checklist

A test goes through as many of these as the tester can:

1. Add the account (say whether the settings were found automatically)
2. Initial sync: folders and recent mail appear
3. Read plain-text and HTML mail
4. New mail appears while the app is open
5. New mail notification while the app is closed
6. Send
7. Reply and forward
8. Send an attachment, and open a received one
9. Archive, delete and move
10. Drafts sync with the provider's webmail or another app
11. The sent message appears in Sent exactly once
12. Search

## Tested by people

Result: ✅ works · ⚠️ works with issues · ❌ broken · — not tested yet. "Sign-in" is password, app password, OAuth or JMAP.

| Provider | Protocol and sign-in | Platform and OS | Mailcrane version | Tested by | Date | Result | Notes and issues |
|---|---|---|---|---|---|---|---|
| Gmail | | | | | | — | Sign in with Google is built (#376) and waiting for Google's verification (#44); an app password works too |
| Outlook.com | | | | | | — | May refuse passwords; Sign in with Microsoft is #45 |
| Microsoft 365 | | | | | | — | Depends on the organisation's settings; #45 |
| iCloud Mail | | | | | | — | Needs an app-specific password |
| Yahoo Mail | | | | | | — | Needs an app password |
| AOL Mail | | | | | | — | Needs an app password |
| Fastmail (IMAP) | | | | | | — | |
| Fastmail (JMAP) | | | | | | — | |
| Proton Mail Bridge | | | | | | — | IMAP and SMTP through the Bridge on a computer |
| Posteo | | | | | | — | |
| mailbox.org | | | | | | — | |
| GMX / WEB.DE | | | | | | — | |
| Zoho Mail | | | | | | — | |
| Self-hosted Dovecot + Postfix | | | | | | — | |
| Stalwart (IMAP and JMAP) | | | | | | — | |

## What the automated tests cover

Tests never use a real mailbox (AGENTS.md §7). They run against the fake IMAP, SMTP, JMAP, ManageSieve and OAuth servers in `core/crates/mailcrane-testkit`, which can copy known provider behaviour. Passing these tests means the app handles that behaviour as documented. It doesn't prove a real provider behaves that way today, which is what the table above is for.

| Behaviour | Seen at | Where it's tested |
|---|---|---|
| `[Gmail]` folder that can't be opened, special folders with spaces in their names, labels (`X-GM-EXT-1`) | Gmail | testkit `gmail_layout()`, `Quirk::GmailLabels` |
| Sent mail saved by the server itself, so the app mustn't upload another copy (`ServerQuirk::AutoSavesSent`) | Gmail, Outlook.com, Microsoft 365 | `mailcrane-imap` `quirks.rs` tests |
| Several actions at once on one account (bug #282) | Gmail; fixed in #301 and checked on a real phone | one-writer-per-account tests in `mailcrane-ffi` (`accounts_tests.rs`) and `mailcrane-core` sync |
| Personal folders under a prefix such as `INBOX.` (RFC 2342 namespace) | Courier, Cyrus, older Dovecot | `mailcrane-imap` parser tests, testkit namespace option (#325) |
| `\Noselect` parent folders | Gmail, many self-hosted servers | testkit `noselect()` |
| No CONDSTORE/QRESYNC, IDLE, MOVE, UIDPLUS or SPECIAL-USE | older or minimal servers | testkit `Quirk::NoCondstore`, `NoQresync`, `NoIdle`, `NoMove`, `NoUidplus`, `NoSpecialUse` |
| Custom keywords not kept (`PERMANENTFLAGS` without `\*`) | some servers | testkit `Quirk::NoCustomKeywords` |
| Send later on the server (`FUTURERELEASE`, RFC 4865) | some SMTP servers | testkit `Quirk::FutureRelease` |
| JMAP mail, sending, folders and a refused submission (RFC 8620, 8621) | Fastmail, Stalwart | testkit JMAP server (`Fault::RefuseSubmissions`) |
| Server-side rules over ManageSieve (RFC 5804) | Dovecot Pigeonhole, Stalwart and others | testkit Sieve server (ADR 0004) |
| OAuth sign-in and token refresh (RFC 6749, PKCE RFC 7636) | Google (built in #376, waiting for verification, #44); Microsoft not built yet (#45) | testkit OAuth server, `mailcrane-core` `tests/oauth.rs` |
| Dropped connections, slow or malformed replies, refused login, temporary and permanent errors | any server | testkit `Fault` |
| No STARTTLS is an error, never plain text | misconfigured servers | testkit `Fault::NoStartTls` |
| Server settings from the provider's autoconfig file, DNS SRV and MX records, and the bundled Mozilla ISPDB (ADR 0003) | most providers above | `mailcrane-core` autoconfig tests |

## Adding a row

- **Testers:** fill in the [provider test report](https://github.com/level-98/mailcrane/issues/new?template=provider_report.yml) form. Never include your address, password, app password or message content; the provider's name is enough.
- **Maintainers:** turn each report into a row in a PR (a `docs` commit, with no changelog fragment needed), and link the report in the notes.
  - Fill in the row of a provider listed above, or add a new row in alphabetical order before the self-hosted ones.
  - Keep earlier rows when a newer version is tested, so the history stays visible; add a new row below the old one.
  - "Tested by" is a GitHub handle, and the date is when the test was done, as `YYYY-MM-DD`.
  - Link any `bug` issues the test found in the notes.
- A new provider behaviour found this way gets a named quirk with a test (AGENTS.md §7) and a line in the table above.
