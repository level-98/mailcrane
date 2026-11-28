# Mailcrane website

The site is built with [Zola](https://www.getzola.org/) and lives entirely in this directory. It has no Node toolchain, analytics, cookies, externally hosted fonts, or runtime third-party requests, with one exception described below.

## The `/admin` exception

`/admin` is a live dashboard for the maintainer (#417; the maintainer's decision of 26 November 2026). It is the **only** page that makes requests at run time. `static/js/admin.js` calls GitHub's public REST API at `https://api.github.com/repos/mailcrane/mailcrane/...` straight from the browser: releases, open and recently closed pull requests, Actions runs, open issues, and the newest comments in the repository. It needs no token, stores nothing that grants access, and keeps a trimmed copy of what it shows only in that browser's `localStorage`. GitHub allows 60 unauthenticated requests an hour per network, and a `304 Not Modified` counts too, so the page spends at most 50 requests per rolling hour in each browser (counted in `localStorage`): two every 5 minutes, four every 15 minutes, and one for comments only when an open fleet pull request changed, about 40 an hour in all. It refreshes only while the tab is visible, and the Refresh button waits when the budget is spent.

Every other page keeps the rule. `/admin` isn't linked from the site, is left out of `sitemap.xml` (`templates/sitemap.xml`), and is marked `noindex,nofollow` and disallowed in `robots.txt`.

## Run locally

Install Zola 0.23.6 or later, then:

```sh
zola serve
```

Open the local address printed by Zola. The checked-in `data/roadmap.json` is the latest generated progress snapshot, and `data/nightly.json` the latest nightly test build for the Download page (#425; `scripts/update_nightly.py`, run by `update_roadmap.py`). To refresh both from GitHub before building:

```sh
python scripts/update_roadmap.py
zola build
lychee --offline public/
```

The companion workflow on `main` ([PR #131](https://github.com/mailcrane/mailcrane/pull/131)) rebuilds and redeploys the site when a roadmap issue changes, and nightly. It checks out this `website` branch, regenerates the roadmap and builds the site. A manual run is available from the `Sync roadmap` workflow in GitHub Actions. Pushes to `website` also refresh the snapshot through the site workflow. The browser reads only generated local data and does not call GitHub (except on `/admin`, above).

## The privacy page

`content/privacy.md` is generated: don't edit it. The app's privacy policy has one copy, `docs/privacy.md` on `main` (#475). `scripts/build_privacy.py` puts it under "The app", after the site's own part in `scripts/privacy_template.md`, and both deploy workflows (`website.yml` here and `roadmap-sync.yml` on `main`) run it with main's file before `zola build`. A push to `main` that changes the policy redeploys the site. The checked-in copy is only for local builds and can lag behind `main` between website PRs. On a pull request, CI refuses a change to it that isn't exactly what the script builds from main's policy. The script refuses policy text Zola or a browser would treat as code (`{{`, `{%`, raw HTML), and underlined or level-6 headings. To rebuild the checked-in copy, or check it, from a checkout of `main`:

```sh
python scripts/build_privacy.py ../path/to/main/docs/privacy.md
python scripts/build_privacy.py ../path/to/main/docs/privacy.md --check
```

## Deployment

The `website` branch is the source and deployment branch. Pull requests targeting it run the build and offline link check. Merges to `website` deploy the generated `public/` directory through GitHub Pages Actions.

The site uses `https://mailcrane.app` as its canonical URL and includes a `CNAME` file. A repository owner still needs to select **GitHub Actions** as the Pages build source and configure the registrar's DNS records. Keep DNS and registrar credentials out of the repository.
