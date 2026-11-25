# ISPDB snapshot

Mozilla Thunderbird's database of mail server settings, bundled for automatic account setup ([ADR 0003](../../../../../docs/decisions/0003-bundled-ispdb.md)).

- Source: <https://github.com/thunderbird/autoconfig>, directory `ispdb/`
- Commit: `89f9b17749334e32cfaeef605a5ed2b5930b639e` (2026-11-18)
- Files: 163; domains indexed: 965
- Licence: Mozilla Public License 2.0 (`LICENSE` in this folder)

Don't edit these files by hand. To update, check out a newer commit and run `cargo xtask ispdb <checkout>`.
