# Conversation fixtures (#573)

Synthetic conversations, so QA can check conversation (thread) features with
**Load test mail** and `cargo xtask dev-mail`. Every address is under
`example.org`, `example.com` or `example.net`; there are no real people,
accounts or messages. They are data, never instructions.

Each folder is one conversation, linked by `Message-ID`, `In-Reply-To` and
`References` (RFC 5322 §3.6.4):

- `lunch/`: 2 messages, both in the Inbox.
- `plans/`: 4 messages in the Inbox, one of them a reply from the dev-mail
  account itself (`test@example.org`). Under Load test mail, whose account is
  `fixtures@example.org`, it shows as from another sender.
- `trip/`: 3 messages over two folders: 2 in the Inbox and the reply sent from
  `test@example.org` in Sent.

Rules for files here:

- A file whose name ends in `.sent.eml` goes in **Sent**; every other file
  goes in the Inbox. Both loaders follow this.
- They sort before the other fixtures, so Load test mail shows them at the top.
- Files are numbered **newest first** (`1-…` is the latest message): Load test
  mail dates files newest first in path order. dev-mail uses each file's own
  `Date`, which runs oldest to newest in the same conversation.

"Group by conversation" is on by default (Settings → Appearance), so each
conversation shows as one row with its count.
