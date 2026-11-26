# Privacy policy

**Draft for the maintainer to approve (#75). Not in effect yet.**

Mailcrane is an email app. It collects nothing: no account with us, no analytics, no crash reports, no ads. The project has no servers of its own, so there is nowhere for your data to go.

## Where your data goes

Your mail, contacts, settings, passwords and sign-in tokens stay on your phone. The app sends them only to the mail servers you add (IMAP, SMTP, JMAP and ManageSieve, always over TLS), so it can fetch, send and organise your mail. Your mail provider's own privacy policy covers what they do with it.

Passwords and sign-in tokens are kept in Android's secure storage (Android Keystore). They are never put in the app's database or logs, and they're left out of backups and transfers to a new phone.

## Every other connection, and when it happens

- **Account setup.** To find your server settings, the app asks your provider's servers (those named by your email domain or its mail records) for their settings file, and uses your phone's normal DNS to look up your domain's mail records. It also checks a list of known providers that is built into the app (from Mozilla's ISPDB), so your domain is never sent to Mozilla or anyone else.
- **Remote images.** Images stored on the sender's servers load only when you tap "Load remote images", for that one message, without cookies. Loading them tells the sender's server that the message was opened.
- **Unsubscribe.** If you confirm a one-tap unsubscribe, the app contacts the sender's server to remove you from the list. Other unsubscribe options open an email for you to send or the sender's web page.
- **Links.** Tapping a link in a message opens it in your browser, after showing where it goes (unless you turn that off).
- **Signing in with Google or Microsoft** (when the app adds it) opens their sign-in page, and their privacy policies apply. The app then talks to their mail servers like any other provider.
- **Downloads and updates** through Google Play, F-Droid or GitHub are covered by those services' own policies.

Nothing else. If a future version adds a connection, for example for push notifications, this policy will name it before that version is released.

## Permissions

- **Internet and network state:** to reach your mail servers, and to wait for a connection instead of failing.
- **Notifications** (you can refuse): to tell you about new mail.
- **Run in the background and after a restart:** to check for new mail. If you turn on "always on" notifications, the app keeps a connection open to your mail server and shows a permanent notification while it does.
- **Contacts** (optional, asked only when you choose to): to suggest addresses while you write. If you also turn on "Use photos from your contacts", it shows a sender's contact photo in place of their initials. Contacts and photos are read on the phone only. They are never stored by the app, uploaded or sent anywhere, and the app works fully without this permission.

## Your choices

You can remove an account in the app, which deletes its mail and passwords from the phone. Uninstalling Mailcrane deletes everything it stored. Since we hold no data about you, there's nothing for us to hand over, correct or delete.

## Contact and changes

Questions: *[contact to be decided by the maintainer: a project email or GitHub issues]*.

Changes to this policy are listed below and in the repository history at <https://github.com/level-98/mailcrane/commits/main/docs/privacy.md>.

| Date | Change |
|---|---|
| *[effective date]* | First version |
