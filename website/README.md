# Mailcrane website

The site is built with [Zola](https://www.getzola.org/) and lives entirely in this directory. It has no Node toolchain, analytics, cookies, externally hosted fonts, or runtime third-party requests, with one exception described below.

## The `/admin` exception

`/admin` is a live dashboard for the maintainer (#417; the maintainer's decision of 26 November 2026). It is the **only** page that makes requests at run time. `static/js/admin.js` calls GitHub's public REST API at `https://api.github.com/repos/level-98/mailcrane/...` straight from the browser: releases, open and recently closed pull requests, Actions runs, open issues, and the comments on open fleet pull requests. It needs no token, stores nothing that grants access, and keeps its cached responses only in that browser's `localStorage`. It sends conditional requests and refreshes once a minute, only while the tab is visible.

Every other page keeps the rule. `/admin` isn't linked from the site, is left out of `sitemap.xml` (`templates/sitemap.xml`), and is marked `noindex,nofollow` and disallowed in `robots.txt`.

## Run locally

Install Zola 0.23.6 or later, then:

```sh
zola serve
```

Open the local address printed by Zola. The checked-in `data/roadmap.json` is the latest generated progress snapshot. To refresh it from the public GitHub issues before building:

```sh
python scripts/update_roadmap.py
zola build
lychee --offline public/
```

The companion workflow on `main` ([PR #131](https://github.com/level-98/mailcrane/pull/131)) rebuilds and redeploys the site when a roadmap issue changes, and nightly. It checks out this `website` branch, regenerates the roadmap and builds the site. A manual run is available from the `Sync roadmap` workflow in GitHub Actions. Pushes to `website` also refresh the snapshot through the site workflow. The browser reads only generated local data and does not call GitHub (except on `/admin`, above).

## Deployment

The `website` branch is the source and deployment branch. Pull requests targeting it run the build and offline link check. Merges to `website` deploy the generated `public/` directory through GitHub Pages Actions.

The site uses `https://mailcrane.app` as its canonical URL and includes a `CNAME` file. A repository owner still needs to select **GitHub Actions** as the Pages build source and configure the registrar's DNS records. Keep DNS and registrar credentials out of the repository.
