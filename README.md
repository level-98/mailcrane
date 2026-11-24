# Mailcrane website

This is the GitHub Pages branch for the Mailcrane project website. The static site lives in `index.html`, `styles.css`, and `script.js`; roadmap progress is fetched from the public GitHub issues and refreshed automatically.

The app source and full project documentation are maintained on [`main`](https://github.com/level-98/mailcrane/tree/main). This branch keeps the Markdown project documents and the brand assets used by the site.

## Project

Mailcrane is a free, open-source email app for Android and iOS. It is in early development; there is no app to download yet. The project is designed around:

- IMAP and SMTP for broad provider support, with JMAP and guided sign-in planned.
- Privacy by default: no ads, analytics, or Mailcrane servers reading your mail.
- Native Android and iOS apps backed by a shared Rust engine.
- Offline access and a public, issue-driven roadmap.

Read the [roadmap](ROADMAP.md), [architecture](docs/architecture.md), and [contributor guide](CONTRIBUTING.md), or [browse open issues](https://github.com/level-98/mailcrane/issues).

## Website deployment

The site has no build step or external runtime dependencies. In the repository’s GitHub Pages settings, publish the `website` branch from the repository root. The live progress tracker uses the public GitHub REST API and will show an error instead of stale data if that service is unavailable.

## Brand

The website uses the original Mailcrane wordmark, crane mark, and colour palette from [`design/`](design/). The brand assets and project are licensed under the [Mozilla Public License 2.0](LICENSE).
