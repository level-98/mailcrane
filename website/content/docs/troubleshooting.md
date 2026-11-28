+++
title = "Troubleshooting"
description = "Fixes for common problems, and how to report one safely."
template = "page.html"
weight = 3
+++

## Adding an account fails

Mailcrane says which server failed (incoming or outgoing) and why. Check the password first: some providers need an app password (see the [provider guides](@/docs/providers/_index.md)). If automatic setup found the wrong settings, set up manually with the details your provider publishes; see [Add an account](@/docs/add-account.md).

## New mail arrives late

Without always-on sync, Mailcrane checks about every 15 minutes. Turn on **Always-on sync** under Notifications in Settings for mail within a minute. If notifications are still late, Android may be saving battery: Settings explains how to turn off battery optimisation for Mailcrane.

## Pictures are missing from a message

Remote images are hidden until you tap **Load remote images**, because loading them tells the sender you opened the message. Images from known tracking services stay blocked even then.

## Reporting a problem

For a project or website issue, search [open GitHub issues](https://github.com/mailcrane/mailcrane/issues) or start a [Discussion](https://github.com/mailcrane/mailcrane/discussions). Include the app version and platform when relevant, but never include passwords, account details, message contents, or private screenshots.

For security problems, follow the private reporting instructions in [SECURITY.md](https://github.com/mailcrane/mailcrane/blob/main/SECURITY.md), not a public issue.
