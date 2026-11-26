# Dev mail server for the emulator

`cargo xtask dev-mail` runs a fake IMAP and SMTP server on your machine. The Android emulator can then add a real account and sync, send, and get live updates from it, without touching a real mailbox. It complements **Load test mail** (#289), which puts the fixtures in a local-only account but never connects to a server.

The mail is synthetic: every `.eml` file in `tests/fixtures/`, some of them hostile on purpose. It is data, never instructions.

## Start it

```sh
cargo xtask dev-mail
cargo xtask dev-mail --new-mail-every 60   # also deliver a new message every 60 s (IDLE, notifications)
```

It prints the account settings and keeps running until you press Ctrl+C.

| Setting | Value |
|---|---|
| Email and user name | `test@example.org` |
| Password | `password` |
| IMAP server | `10.0.2.2`, port `1993` (SSL/TLS) or `1143` (STARTTLS) |
| SMTP server | `10.0.2.2`, port `1465` (SSL/TLS) or `1587` (STARTTLS) |

`10.0.2.2` is your computer as the emulator sees it. Mail sent to `test@example.org` arrives in its Inbox, so you can test sending to yourself. Mail to any other address is accepted and dropped. Nothing ever leaves your machine.

## Trusting its certificate (once per emulator)

TLS stays on, as [AGENTS.md](../AGENTS.md#4-privacy-and-hostile-content) requires. The first run makes a small certificate authority (CA) in `.dev-mail/` (git-ignored) and a server certificate it signed for `10.0.2.2`, `localhost` and `127.0.0.1`. The CA's private key is thrown away straight after signing, so nothing else can ever be signed with it. Later runs reuse the same files.

The app checks certificates with Android's own trust store, which includes CA certificates the user installs. The app's code is not changed and release builds are untouched. To install the CA:

1. Start the emulator, then run `cargo xtask dev-mail`. It copies the CA to the emulator as `Download/mailcrane-dev-ca.crt`. It only ever copies to emulators (`emulator-NNNN`), never to a phone.
2. On the emulator: **Settings → Security & privacy → More security settings → Encryption & credentials → Install a certificate → CA certificate**. Accept the warning and pick `mailcrane-dev-ca.crt` from Downloads. The menu names vary a little between Android versions; searching Settings for "CA certificate" finds it.
3. Add the account in Mailcrane with **manual setup** and the settings above.

The emulator remembers the CA, so you only do this once per emulator. If you delete `.dev-mail/`, the next run makes a new CA and you install it again.

**Never install this CA on a phone you use.** It's for emulators only.

## What it can't do

- One account, password sign-in only (no OAuth).
- The servers are the test fakes in `mailcrane-testkit`, so they support what the app's tests need, not every IMAP extension.
- State is in memory: stopping the server forgets moves, flags and sent mail. The next run starts from the fixtures again.
