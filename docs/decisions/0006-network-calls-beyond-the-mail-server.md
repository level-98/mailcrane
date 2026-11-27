# 0006: Network calls beyond the mail server

Status: accepted
Date: 2026-11-27

## Context

[AGENTS.md §4.7](../../AGENTS.md#4-privacy-and-hostile-content) allows no network calls except to the user's own mail provider unless an accepted decision allows them. The bundled ISPDB has one ([ADR 0003](0003-bundled-ispdb.md)), and it exists so that setup never sends the address's domain to Mozilla. Four other kinds of call shipped without one:

- **Automatic setup** fetches the provider's autoconfig file from its well-known URLs for the address's own domain, and, when nothing else matches, for the domain that handles its mail according to MX (#236, #237).
- **One-click unsubscribe** (RFC 8058) sends an HTTPS POST to the sender's unsubscribe URL (#215).
- **UnifiedPush wake-ups** register with the distributor app the user installed and chose; its push server can then wake the app (#359).
- **JMAP push** gives the user's own JMAP server a PushSubscription with the push endpoint and the keys to encrypt what it sends there (RFC 8620 §7.2; #374, #387).

Not covered here, because they're already allowed elsewhere: OAuth sign-in and token refresh at the user's own provider (Google, #376), which is the mail provider; remote images, which [AGENTS.md §4.6](../../AGENTS.md#4-privacy-and-hostile-content) allows only when the user asks, without cookies or referrer; and links, which open in the user's browser.

The maintainer handed this decision to the fleet lead (docs/fleet.md, "Who decides"), who made it on 2026-11-27 (#479).

## Decision

These four calls are allowed, with limits:

1. Autoconfig and well-known lookups for the address's own domain (and its MX provider's domain), during account setup only.
2. One-click unsubscribe POSTs (RFC 8058), only when the user taps Unsubscribe and confirms.
3. The UnifiedPush distributor the user installed and chose, only while the user has push wake-ups turned on.
4. A JMAP PushSubscription on the user's own JMAP server, only while push wake-ups are on.

For each one:

- no cookies and no tracking;
- nothing sent beyond what the protocol needs: no mail content and no passwords. Setup lookups never send the email address itself (autoconfig placeholders are filled in on the phone);
- never in the background without a user action or a setting the user turned on;
- TLS always, as for mail ([ADR 0008](0008-android-tls-platform-verifier.md)).

Any other call still needs a new decision before it ships. The [privacy policy](../privacy.md) names every call the app makes, and changes with it.

## Why

- Each call does something the user asked for (set up this account, unsubscribe from this list, wake me when mail arrives), and the protocol is the only way to do it.
- None of them goes to a server the project runs, and the project still has none.
- Writing the limits down keeps them from widening quietly. For example, a call "only during setup" can't become a background check without a new decision.

Options we didn't pick:

- **Ask the user before each call.** Setup and unsubscribe are already user actions, and push is a setting. One more prompt would add nothing.
- **Leave them unrecorded.** That breaks AGENTS.md §4.7, and the next call would have nothing to be measured against.
- **Remove them.** Automatic setup, unsubscribe and push are 1.0 features, and every alternative (a manual-only setup, a gateway server for push) is worse for privacy or usability.

## Consequences

- A PR that adds or widens a network call needs a decision first, and must update the privacy policy in the same PR.
- Reviewers check new HTTP or socket code against this list.
- The F-Droid listing can keep saying the app has no non-free network services ([docs/fdroid.md](../fdroid.md)): every call goes to the user's own provider, a sender they chose to unsubscribe from, or a distributor they installed.
