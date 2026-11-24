# Mailcrane website

The site is built with [Zola](https://www.getzola.org/) and lives entirely in this directory. It has no Node toolchain, analytics, cookies, externally hosted fonts, or runtime third-party requests.

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

After [PR #131](https://github.com/level-98/mailcrane/pull/131) is merged into `main`, its issue and nightly triggers will check out this `website` branch, regenerate the roadmap, build the site, and deploy it. A manual run is available from the `Sync roadmap` workflow in GitHub Actions. Until that companion change is merged, pushes to `website` still refresh the snapshot through the site workflow. The browser reads only generated local data and does not call GitHub.

## Deployment

The `website` branch is the source and deployment branch. Pull requests targeting it run the build and offline link check. Merges to `website` deploy the generated `public/` directory through GitHub Pages Actions.

The site uses `https://mailcrane.app` as its canonical URL and includes a `CNAME` file. A repository owner still needs to select **GitHub Actions** as the Pages build source and configure the registrar's DNS records. Keep DNS and registrar credentials out of the repository.
