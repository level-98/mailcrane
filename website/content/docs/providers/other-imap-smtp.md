+++
title = "Other IMAP and SMTP providers"
description = "Setting up any provider that offers IMAP and SMTP."
template = "page.html"
weight = 7
+++

Providers without their own guide work through IMAP and SMTP. Try automatic setup first: enter your address and password and Mailcrane looks up the settings. If it can’t find them, set up manually with the server names, ports and security your provider publishes (see [Add an account](@/docs/add-account.md)). Connections always use TLS or STARTTLS.

Provider-specific settings will be documented only after they have been tested. See the [provider test matrix](https://github.com/mailcrane/mailcrane/issues/48) for coverage status.
