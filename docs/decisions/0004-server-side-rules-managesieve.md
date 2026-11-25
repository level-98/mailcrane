# 0004: Server-side rules through ManageSieve

Status: accepted
Date: 2026-11-25

## Context

Local rules (M4-09) run on the phone after each sync. They don't run while the phone is off, asleep or offline, and mail read on another device meanwhile isn't sorted. Most IMAP providers that run their own servers (Dovecot with Pigeonhole, Cyrus, Stalwart and others) let clients store filter scripts in the Sieve language (RFC 5228) through ManageSieve (RFC 5804), and run them at delivery. Issue #259 (M4-23) asks to use that, keeping local rules for servers without it.

This adds a crate to the workspace and opens a connection to a new port (4190) on the user's provider. Both are in CONTRIBUTING.md's "Decisions that only maintainers make". The maintainer delegated these decisions to the steward on 25 Nov 2026, and the steward decided as below.

## Decision

- **A new crate, `mailcrane-sieve`,** beside `-imap` and `-smtp`: our own ManageSieve client and a small Sieve script writer. Its only dependencies are ones the workspace already uses (`rustls`, `rustls-platform-verifier`, `tokio`, `tokio-rustls`, `base64`, `thiserror`). No Sieve library is added.
- **The client only talks over TLS.** It connects to port 4190, requires `STARTTLS` (RFC 5804 §2.2) and stops if the server doesn't offer it, checks the certificate exactly as IMAP does, refuses bytes sent between the STARTTLS reply and the handshake, and logs in only with `AUTHENTICATE "PLAIN"` using the account's IMAP username and password. Accounts without a stored password (OAuth) keep their rules local.
- **Hostile input:** every server response goes through one parser with limits (64 KiB per line, 1 MiB per literal, 10 000 lines, 4 MiB per response), with property tests and a fuzz target (`sieve_response`). Errors never carry server text.
- **Discovery:** a `_sieve._tcp` SRV record for the address's domain (RFC 5804 §1.8), through the platform resolver the app already uses for setup, then the IMAP host on port 4190. DNS answers aren't authenticated, so an SRV target only counts if it is the IMAP host, or a host in the address's domain or the IMAP host's parent domain; any other target is ignored and never gets the password. "No usable server" (no connection, no STARTTLS, no PLAIN, a refused password) is remembered per account and checked again after 3 days, so syncs don't keep waiting on it and a wrong password isn't sent again and again.
- **One script, `mailcrane`.** Mailcrane compiles the account's enabled rules into it and never writes, deletes or deactivates any other script. If another script is active, the rules stay on the device and the rules screen shows one line offering to switch. Only the user's tap (after a confirmation) makes `mailcrane` the active script; the other script is kept on the server.
- **What goes to the server:** From, To/Cc, Subject and List-Id "contains" tests; mark read and flag (`imap4flags`, RFC 5232), move and archive (`fileinto`), and delete (`fileinto` the Trash folder, or `discard` when there is none, as locally). Only extensions the server advertises are used, plus `comparator-i;unicode-casemap` (RFC 5051) when offered. "Has an attachment" and "older than" rules, moves to folders that can't be opened, and anything the server lacks stay on the device and show "On this device". After such a rule that moves mail or stops processing, later rules stay on the device too, so rules still run in the order shown.
- **The script keeps local semantics:** flags add up, the first matching move wins (one `if … elsif` chain), and "stop processing" guards later rules. Every string and folder name is escaped (`"` and `\`; CR, LF and NUL dropped).
- **No double work:** while the script is active, the device doesn't apply the rules in it again. A rule edited since the last upload runs on the device until the new script is up. `CHECKSCRIPT` and `HAVESPACE` run before `PUTSCRIPT`; if any step fails, the server keeps its old script and nothing changes on the device.
- **When:** the check runs inside an inbox sync, after the new mail is stored and before local rules run. It connects only when it's due: on first use, when the rules changed (at most every 15 minutes after a failure), after the user asks to switch, and once a day to notice a script changed elsewhere.

## Why

- **Rules work when the phone is off,** which is what people expect from "rules", and other devices see sorted mail.
- **RFC 5804 and RFC 5228 are small and stable.** A client for the handful of commands needed is a few hundred lines, written to the same never-crash rules as our IMAP client. The Rust Sieve crates that exist are server-side interpreters and would add far more code than we use.
- **One owned script is safe.** Users and providers often have their own scripts (vacation replies, spam filters). Never touching them, and asking before activating ours, means Mailcrane can't silently turn off someone's filtering.
- **Port 4190 on the user's own provider** is not a third party, so AGENTS.md §4.7 holds; it is recorded here because it is a new destination port.

Options we didn't pick:

- **Edit the user's active script** (append our rules to it). It would work alongside other filters, but parsing and rewriting arbitrary Sieve safely is a large job, and a mistake breaks the user's own filtering.
- **JMAP Sieve (RFC 9661)** for JMAP accounts. Possible later; JMAP accounts keep local rules for now.
- **Implicit-TLS ManageSieve.** Not standardised; servers offer STARTTLS on 4190.

## Consequences

- Server and device matching can differ slightly: Sieve's `:contains` ignores case (beyond ASCII only with `i;unicode-casemap`) but not accents, while local rules ignore both.
- Users with another active script get local rules until they switch. Switching turns their other script off (it is kept). The confirmation says so.
- The first check on a server without ManageSieve can add up to 10 seconds per candidate host to one sync, once every 3 days.
- New conditions or actions need both a local and a server version, or stay device-only.
