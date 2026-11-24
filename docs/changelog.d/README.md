# Changelog fragments

Each PR adds **one small file** here instead of editing a shared changelog. This means two PRs never conflict over the changelog.

**File name:** `<issue-number>.<type>.md`, where type is one of `feature`, `fix`, `security`, `change` or `internal`.

**Content:** one or two sentences, written for users.

Example, `42.feature.md`:

```
New mail now appears straight away while the app is open (IMAP IDLE).
```

At release time a script gathers the fragments into the release notes and deletes them.
