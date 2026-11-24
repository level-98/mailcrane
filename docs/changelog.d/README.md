# Changelog fragments

Each PR adds **one small file** here instead of editing a shared changelog. This means two PRs never conflict over the changelog.

**File name:** `<issue-number>.<type>.md`, where type is one of `feature`, `fix`, `security`, `change` or `internal`.

**Content:** one or two sentences, written for users.

Example, `42.feature.md`:

```
New mail now appears straight away while the app is open (IMAP IDLE).
```

At release time, `python scripts/release_notes.py <version>` prints the release notes grouped by type (a dry run). Adding `--apply` puts them at the top of `CHANGELOG.md` and removes the fragments it used. `--include-internal` also lists `internal` fragments. A slug id (`m0-01-licence`) is allowed instead of an issue number.
