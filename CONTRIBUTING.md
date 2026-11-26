# Contributing to Mailcrane

Thanks for helping. Mailcrane is a small project built by people and AI agents together. The process is deliberately light: **issues are the to-do list, and a comment is all it takes to claim one.**

You're responsible for what you submit, whoever or whatever wrote it, so check it works.

## The short version

1. Find an open issue that nobody's assigned to. Start with [`good first issue`](../../labels/good%20first%20issue) or [`agent-ready`](../../labels/agent-ready).
2. Comment **`/claim`** on it. A bot assigns it to you.
3. Branch, build, run `cargo xtask ci`, and open a PR with `Closes #<issue>`.

That's it. There's no sign-up, CLA or coordination thread.

## Rules that aren't negotiable

1. **No real personal data.** Never put real email addresses, messages, passwords, tokens or keys in code, tests, logs, issues or PRs. Use `example.org` and the built-in fake servers.
2. **Never crash on bad input.** Email is hostile input. See [AGENTS.md](AGENTS.md#3-never-crash).
3. **Be honest about testing.** "Didn't test on a real device" is fine. Claiming you tested something you didn't is not.

## Claiming work

- Comment `/claim` on an unassigned issue. The first claim stands.
- A claim lapses after **14 days** with no comment, commit or draft PR. A bot unassigns it automatically, and anyone can claim it again.
- Changed your mind? Comment `/unclaim`.
- Hold at most **2 claims** at once.
- An open PR holds its issue. If a review asks for changes and there's no reply within 14 days, the issue becomes free again.

### Found a bug and already fixed it?

Skip claiming. **Open the bug report and the PR together.** Bug reports without a fix are welcome too.

### Want to build something that isn't an issue yet?

Open a **feature request** first and wait for a maintainer to label it `accepted`. Small, obvious improvements (typos, docs, tests) don't need this.

## Pull requests

- **One issue, one PR**, branched from current `main`. Small enough to review in one sitting, ideally under 400 changed lines.
- Branch name: `<issue-number>-<short-name>`, for example `42-imap-idle`.
- Run **`cargo xtask ci`** before pushing. It runs every check CI runs.
- Big feature? Split it into **stacked PRs**. Each later one says `Builds on #<n>` and stays a draft until the one before it lands.
- Add a **changelog fragment**: one small file in `docs/changelog.d/` (see the README there). Never edit a shared changelog table, which avoids merge conflicts.
- Leave **"Allow edits by maintainers"** ticked, so small fixes don't need a round trip.
- We squash-merge, so the PR title becomes the commit message: `feat(imap): support IDLE (#42)`.

## CI

Every pull request and every push to `main` runs [`ci.yml`](.github/workflows/ci.yml), with no path filters, so docs-only PRs get the same checks. Each job only installs tools and runs `cargo xtask ci`:

| Job | Runs on | Checks |
|---|---|---|
| `linux` | Ubuntu | Every step (`--strict`): fmt, clippy, tests, `cargo deny`, the design check and the release-notes script tests, and the Android lint, unit tests, debug and release builds (the core cross-compiled for arm64-v8a and x86_64), with a check that each APK packages its own core. The debug APK is uploaded as the `android-debug-apk` artifact for 7 days, so you can install it with `adb install` without building. |
| `windows` | Windows | fmt, clippy and tests of the core (`--strict --only fmt,lint,test`), so it stays portable. |
| `ci-ok` | Ubuntu | Passes only if every job above passed. Look here first. |

## Review

Mailcrane may have only one maintainer, so review is set up not to block:

| Change | What's needed to merge |
|---|---|
| Docs, tests, UI, most code | CI green + one maintainer approval. A maintainer may merge their own PR once CI is green and it has been open 24 hours. |
| Labelled `security` (sign-in, OAuth, credential storage, TLS, the HTML sanitiser, encryption) | CI green + a maintainer's review. If there is a second maintainer, they must approve too. Never self-merged in under 72 hours. |

## Decisions that only maintainers make

Ask in an issue **before** you build any of these:

- A new dependency, or a new crate in the workspace
- Anything that sends data anywhere other than the user's own mail server
- New permissions or scopes requested from Google, Microsoft or other providers
- Loosening the HTML sanitiser's allow-list
- Changes to AGENTS.md, SECURITY.md, the licence, or an accepted decision in `docs/decisions/`
- New brand assets

## Using AI tools

AI-written contributions are welcome on the same terms as any other.

- Your agent should read [AGENTS.md](AGENTS.md). Most tools load it automatically.
- Say in the PR that it was AI-assisted, and which tool and model.
- Read the diff yourself before marking it ready.
- Please don't open lots of low-effort PRs at once. Maintainers may close them without review.

Maintainers can also hand an `agent-ready` issue straight to the project's own agent by adding the `agent-go` label. It opens a draft PR for a human to review.

The agent runs on a maintainer's Claude token, so:

- **Read the issue before adding `agent-go`.** The agent reads the issue text, so only label issues you'd be happy to hand to a stranger's script.
- New maintainers get **triage** access first. Anyone with triage can start runs; anyone with write can push workflows, so give write only to people you trust.
- The token is stored as a secret of the `agent` environment, which only `main` can use. Don't move it back to a repository secret, and don't remove `environment: agent` from `.github/workflows/agent.yml`.

## Labels

| Label | Meaning |
|---|---|
| `good first issue` | Small, with the files named. Good for newcomers. |
| `agent-ready` | Has a goal, files, acceptance criteria and test steps, so an AI agent can do it unsupervised |
| `agent-go` | Maintainers only: starts the project's agent on this issue |
| `needs-human` | Needs real devices, real accounts or a judgement call |
| `needs-decision` | Blocked until a maintainer decides |
| `accepted` | A feature request maintainers have agreed to |
| `security` | Touches sign-in, credentials, TLS, encryption or the sanitiser |
| `post-1.0` | Not until after 1.0. Please don't claim yet. |
| `m0` … `m8` | Milestone (see ROADMAP.md) |
| `area:core` · `area:android` · `area:design` · `area:docs` · `area:ci` | Which part of the project |

## Other ways to help

- **Try it with your own provider** and fill in a [provider test report](../../issues/new?template=provider_report.yml), even when everything works. Results are collected in [docs/providers.md](docs/providers.md). Never share your account details.
- **Design**: mockups and UX feedback, following Material 3.
- **Write docs.**
- **Translate** the app on Weblate. See [Translations](#translations).

## Translations

Translations are done on Weblate, not in pull requests. Weblate opens a pull request with its changes, and CI checks it like any other. The Weblate project is being set up; [docs/translations.md](docs/translations.md) will link it and has the guide for translators (placeholders, plurals, licence) and for developers adding strings. Please don't edit `values-*/strings.xml` by hand.

## Code of conduct

Be kind and assume good faith. This project follows the [Contributor Covenant 2.1](CODE_OF_CONDUCT.md).
