# Push wake-ups with UnifiedPush (M4-05)

Mailcrane has three ways to learn about new mail when it isn't on screen:

| Mode | How | Delay | Cost |
|---|---|---|---|
| Periodic check (default) | WorkManager, about every 15 minutes | 15 min or more; Android may wait longer | Least battery |
| Always-on sync | A foreground service keeps an IMAP IDLE connection open | Under a minute | A permanent notification and an open connection |
| Push wake-ups (optional) | A [UnifiedPush](https://unifiedpush.org) distributor app wakes Mailcrane, which then checks | Seconds, if a gateway sends them | No connection of Mailcrane's own |

Push wake-ups are off by default and sit under Settings → Notifications. The periodic check keeps running beside them, so nothing is missed if the gateway is down.

## What the app does

- Uses the free UnifiedPush connector library (`org.unifiedpush.android:connector`, Apache-2.0). No Google Play Services or Firebase, so the F-Droid check (`checkFreeDependencies`) still passes.
- Registers with the distributor the user chose (or the phone's default) and shows the address (endpoint) it gets, with a Copy button.
- On any push message it runs one expedited inbox check (`PushCheckWorker`). The message's content is never read, parsed or logged: it only means "check now". Wake-ups that arrive while a check is queued are merged into it.
- Talks to no new server. The distributor app holds its own connection to its push server; Mailcrane's check talks only to the user's mail servers, as always.

## The gap: nothing sends the push messages yet

IMAP servers don't send UnifiedPush (Web Push) messages. Something on the user's side has to watch the mailbox and POST to the endpoint when mail arrives. Mailcrane doesn't run or ship such a server, and this change doesn't invent one. Options for people who host their own:

- **Dovecot:** the `push_notification` plugin with a small HTTP hook (for example a Lua script) that POSTs to the endpoint on `MessageNew`.
- **Any server:** a small always-on process (on a home server or VPS) that holds an IMAP IDLE connection and POSTs an empty body to the endpoint on `EXISTS`.

The POST body can be empty: Mailcrane only needs the wake-up. Don't put message details in it.

Planned follow-ups, not in this change:

- **JMAP push:** JMAP servers can push to a Web Push URL themselves (`PushSubscription`, RFC 8620 §7.2). Registering the UnifiedPush endpoint with the user's JMAP server would give push with no gateway at all.
- **VAPID:** some distributors require the sender to identify itself with a VAPID key (RFC 8292). Without a server Mailcrane has no key to give, so those distributors report "needs a server key".

## Security notes

- The endpoint is a capability address: anyone who has it can wake the phone into a mail check, nothing more. It's kept in the app's private preferences, copied with Android 13's "sensitive" clipboard flag, and never logged.
- The connector library's exported receiver checks each event's registration token before handing it to `MailcranePushService` (not exported), so other apps can't fake wake-ups.
