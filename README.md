# Mailcrane website

The Zola website and user documentation live in [`website/`](website/). The site uses the `mailcrane.app` domain, self-hosts its fonts and design assets, and has no analytics, cookies, or browser-side third-party requests.

## Local development

Install Zola 0.23.6 or later. See [`website/README.md`](website/README.md) for local preview, roadmap refresh, and offline link-check commands.

## Deployment

The `website` branch is the Pages source. Pull requests targeting it build the Zola site and check generated links offline; merges to `website` deploy through GitHub Pages Actions. The roadmap snapshot is refreshed from public GitHub issues during each build, so the browser only loads a local JSON file.

The site artifact includes `CNAME` for `mailcrane.app`. A repository owner still needs to set the Pages source to **GitHub Actions** and configure DNS at the domain registrar.

## Project

Mailcrane is a free, open-source email app for Android and iOS. It is in early development; there is no app to download yet. Read the [roadmap](ROADMAP.md), [architecture](docs/architecture.md), and [contributor guide](CONTRIBUTING.md), or [browse open issues](https://github.com/level-98/mailcrane/issues).

The site reuses the project brand from [`design/`](design/) and includes the proposed app screens. Its light/dark theme follows the system preference by default and remembers a manual selection.
