# 0007: JMAP storage schema

Status: accepted
Date: 2026-11-27

## Context

The store was built around IMAP: each message row has a folder and a per-folder integer UID, and `(folder_id, uid)` is unique. JMAP (RFC 8620, RFC 8621) has string ids, and an email can be in several mailboxes at once. Migrations can't be undone once released, so the schema needed a decision before JMAP accounts shipped. It shipped without one, in schema v14 (#266) and v21 (#374). This records what shipped and why. The fleet lead made the decision under docs/fleet.md, "Who decides" (#479).

## Decision

The schema as merged:

- **v14, `0014_jmap.sql` (#266):**
  - `accounts.protocol` is `'imap'` (the default for every existing account) or `'jmap'`. A JMAP account keeps its session URL in `accounts.jmap_session_url`; its `imap_*` and `smtp_*` columns hold empty values and are never used.
  - `jmap_mailboxes` maps each folder to its JMAP mailbox id. JMAP folders use the mailbox's name path as `remote_name`, so the rest of the app sees an ordinary folder.
  - `jmap_emails` maps each message row to its JMAP email id and blob id. An email in several mailboxes has **one row per mailbox**, as Gmail labels look over IMAP. Each row gets a per-folder number in `messages.uid`, taken from `folders.uidnext`, so `(folder_id, uid)` stays unique.
  - `jmap_state` keeps the last state string per data type (`Mailbox`, `Email`; RFC 8620 §5.2), the JMAP counterpart of HIGHESTMODSEQ.
- **v21, `0021_jmap_push.sql` (#374):** `jmap_push_subscriptions` holds at most one PushSubscription per account (RFC 8620 §7.2): the server's id, this install's `deviceClientId`, the push URL, the public key when one was sent (never the auth secret), its expiry and whether verification finished. Removing the account removes the row.

## Why

- **One row per mailbox** reuses everything built for IMAP and Gmail labels: folders, the message list, threading, search, rules, notifications and unified views all work unchanged. That's the roadmap's rule that every account type gets the same features.
- **Mapping tables** instead of new columns on `messages` and `folders` keep the IMAP tables as they were, and `ON DELETE CASCADE` cleans up with the folder or message.
- **Local numbers in `uid`** keep the `(folder_id, uid)` key and every query built on it.

Options we didn't pick:

- **A separate set of JMAP tables** (messages, mailboxes): every feature would need two code paths.
- **One row per email with a mailbox list:** simpler for JMAP, but it breaks the folder model the whole app is built on, and Gmail over IMAP would still need the other shape.

## Consequences

- An email in N mailboxes has N message rows, so each folder's list and counts are right without special cases.
- Code that acts on a JMAP email has to think about all its rows; `jmap_emails.email_id` is indexed so they can be found together.
- Changing this shape later needs a new migration and a decision that replaces this one.
