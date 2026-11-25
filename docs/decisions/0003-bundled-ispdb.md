# 0003: Bundle a snapshot of Mozilla's ISPDB for automatic setup

Status: accepted
Date: 2026-11-25

## Context

Automatic account setup (M1-15, issue #31) finds server settings from the email address. The provider's own sources come first: its autoconfig file at its well-known URLs, then DNS SRV records. Many big providers publish neither. Gmail, Outlook.com, Yahoo and GMX are examples, and so are many regional ISPs. For them, the usual answer is Mozilla's ISPDB: the database Thunderbird uses, at <https://github.com/thunderbird/autoconfig>.

Thunderbird asks the ISPDB live at `autoconfig.thunderbird.net`. That sends the user's domain (not the address) to Mozilla on every setup. AGENTS.md §4.7 allows no network calls except to the user's own provider unless an accepted decision says otherwise, so issue #31 asked for this decision first.

## Decision

- Mailcrane **bundles a snapshot of the ISPDB in the app** and never queries `autoconfig.thunderbird.net`.
- The snapshot lives in `core/crates/mailcrane-core/data/ispdb/`. It holds the upstream XML files unchanged, the upstream `LICENSE`, and `SOURCE.md`, which records the commit and date. The core compiles it in through a generated, sorted domain index (`src/account/autoconfig/ispdb_index.rs`).
- `cargo xtask ispdb <checkout>` imports a snapshot from a local checkout of `thunderbird/autoconfig`. The script downloads nothing; whoever updates it checks out the commit they mean to ship. It refuses unexpected file names, non-UTF-8 files, DOCTYPEs and files over 64 KB.
- **Lookup order:** the provider's own autoconfig file and SRV records come first. If they find nothing, the ISPDB is checked for the user's domain. If that has nothing either, the MX provider's domain is tried: its autoconfig file, then the ISPDB.
- ISPDB entries get the same rules as any autoconfig file: plain-text servers are dropped, non-password-only entries are dropped, and the result is shown for the user to confirm before any login.
- **Refresh the snapshot before each release** (a line in the release checklist when M6 adds one).

## Why

- **Nothing leaves the device.** No third party learns which provider the user is setting up, and setup works the same offline or behind a firewall.
- **It covers the providers most people use.** The snapshot is 163 files covering 965 domains, including Gmail and Google-hosted domains (through MX), Outlook.com and Hotmail, Yahoo, AOL, GMX and WEB.DE.
- **It's small.** About 250 KB of XML, which is little next to the app's native libraries.
- **The licence allows it.** The repository is MPL-2.0, the same licence as Mailcrane, so the files can ship unchanged with their licence kept next to them. A 2009 Mozilla wiki page also calls the data public domain, which is even less restrictive.

Options we didn't pick:

- **Query `autoconfig.thunderbird.net` live.** It's always current, but it sends the domain to Mozilla on every setup and needs this exception to §4.7 for a third-party request.
- **No ISPDB.** It's the simplest option, but users of the biggest providers would type their server details by hand.

## Consequences

- The snapshot can be out of date between releases. A provider that changed servers since the snapshot gets the old settings; the login check then fails and the user can edit the settings. Updating is one command.
- Upstream's MPL-2.0 notice must stay with the files (`data/ispdb/LICENSE`).
- Entries that need OAuth2 only, such as Gmail's for accounts without app passwords, can't log in until M3 adds OAuth. Setup still shows the settings, and the login check reports the failure.
- If Mailcrane ever wants the live service as well, that needs a new decision that replaces this one.
