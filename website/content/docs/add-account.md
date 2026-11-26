+++
title = "Adding an account"
description = "Add an IMAP/SMTP or JMAP account, automatically or by hand."
template = "page.html"
weight = 2
+++

Open the app and tap **Add account**. If you already have an account, open **Settings → Add account**.

## Automatic setup

Type your email address and password. Mailcrane finds your server settings from your provider’s own configuration, your domain’s DNS records, or a list of known providers built into the app. Your address isn’t sent anywhere else. Check the settings it found, then continue.

Some providers want an app password instead of your normal password. The [provider guides](@/docs/providers/_index.md) say which ones.

## Manual setup

If automatic setup can’t find your settings, or you’d rather type them in, tap **Set up manually** and enter the details your provider gives you:

- **IMAP and SMTP:** each server’s name, port and security (TLS or STARTTLS), your username and your password. There is no unencrypted option.
- **JMAP** (for example Fastmail or Stalwart): choose JMAP, then enter your domain or the server’s session URL.

## Before it’s saved

Mailcrane signs in to both servers before saving anything. If one fails, it says which server and why, so you can fix it. Passwords stay only in Android’s secure storage. An account that’s already in the app can’t be added a second time.

## Getting help

Ask in [GitHub Discussions](https://github.com/level-98/mailcrane/discussions) or see [troubleshooting](@/docs/troubleshooting.md). Never post your password, app password, sign-in code or private mail.
