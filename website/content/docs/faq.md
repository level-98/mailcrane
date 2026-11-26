+++
title = "Frequently asked questions"
description = "Answers about downloading Mailcrane, providers and privacy."
template = "page.html"
weight = 4
+++

## Can I download Mailcrane?

Not yet: the first public release is being prepared. On release day the [download page](@/download.md) will link to F-Droid, Google Play and GitHub Releases. It also shows how to get notified when it’s out.

## Is there an iPhone version?

Not before Mailcrane 1.0. Mailcrane is an Android app, and no iOS app is planned until after the 1.0 release ([decision 0005](https://github.com/level-98/mailcrane/blob/main/docs/decisions/0005-no-ios-before-1-0.md)). The mail engine is written in Rust and kept platform-neutral, so an iPhone app stays possible later.

## Which providers will work?

Any provider that offers IMAP and SMTP, or JMAP, which covers most of them. Gmail sign-in with your Google account is coming. See the [provider guides](@/docs/providers/_index.md) for what has been tested.

## Does Mailcrane collect my mail?

No. Mailcrane has no servers and no analytics: your mail goes only between your phone and your mail server. Read the [privacy policy](@/privacy.md).

## Are the screens real?

Yes. The screenshots on the home page come from the app as it is today, filled with made-up test mail.

## How can I help?

Read [Contribute](@/contribute.md) or browse [open issues](https://github.com/level-98/mailcrane/issues).
