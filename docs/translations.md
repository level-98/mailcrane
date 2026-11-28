# Translations

Mailcrane is translated on [Weblate](https://weblate.org) (M6-02, issue #69). Translators work in the browser, without git. Weblate sends their work back as pull requests, which go through CI and review like any other change.

**Status:** the repository is ready, but the Weblate project doesn't exist yet. Until a maintainer creates it (below), there's no link to translate at.

## For translators

1. Open the Mailcrane project on Weblate *(link added when the project exists)* and sign in.
2. Pick your language, or ask for it to be added.
3. Translate. The English text, a comment explaining where it appears, and the checks for each string are shown next to it.

Please keep in mind:

- **Placeholders** such as `%1$s` or `%d` are filled in by the app (a name, a number). Keep every one of them, unchanged. You may move them, and change their order when they're numbered (`%2$s … %1$s`). CI refuses a translation that drops, adds or changes one, because the app would show the wrong text or fail at that point.
- **A percent sign** that isn't a placeholder is written `%%`, unless Weblate shows the English string without placeholders.
- **Plurals:** Weblate shows one box per plural form your language has (for example `one`, `few` and `other`). The `other` form must contain the number placeholder; other forms may leave it out ("One message").
- **Apostrophes:** the English text uses typographic ones (’). Weblate escapes a plain `'` for Android for you.
- **"Mailcrane"** is the app's name; keep it unless your language normally writes names in another script.
- Short is good: many strings are buttons and menu items on a phone screen.
- Anything you translate is published under the project's licence, MPL-2.0.

Unfinished languages are fine. Missing strings show in English, and a language appears in the phone's per-app language settings (Android 13 and newer) as soon as its file exists.

## For developers

- User-facing text lives only in `android/app/src/main/res/values/strings.xml` (AGENTS.md §9). Add a `<!-- comment -->` above a string when its place in the app isn't obvious from the name.
- Use `<plurals>` for anything with a count, never "message(s)", and numbered placeholders (`%1$s`) when a string has more than one.
- Never build a sentence from pieces in code; word order differs between languages.
- Mark text that must not be translated (a sample address, a format code) with `translatable="false"`.
- Don't edit `values-*/strings.xml` by hand. Weblate owns them, and hand edits cause merge conflicts on its branch. Fix the English text instead, or comment on the string in Weblate.
- `cargo xtask lint` (part of `cargo xtask ci`) compares every `values-*/strings.xml` with the English file. It checks placeholders (argument number and type), plurals, malformed `%` signs and names that no longer exist in English. A string marked `formatted="false"` or `translatable="false"` in English is skipped, in every language.
- Lint treats `MissingTranslation` as a warning, so a half-translated language doesn't fail the build.
- The per-app language list is generated from the `values-<locale>` folders (`androidResources.generateLocaleConfig`). `res/resources.properties` says the default folder is English.

## Setting up Weblate (maintainer, once)

1. Apply for free hosting for libre projects at <https://hosted.weblate.org/hosting/> (or use another Weblate server, and change the URL in `.weblate`).
2. Create the project `mailcrane` with the licence **MPL-2.0** and a short note for contributors that links this page.
3. Add the component `android`. It must match `.weblate` in the repository, which the `wlc` command-line client reads.

   | Setting | Value |
   |---|---|
   | Source code repository | `https://github.com/mailcrane/mailcrane.git`, branch `main` |
   | Repository push URL | through the Weblate GitHub app or a bot account with access to forks only |
   | Push branch | `weblate` (never `main`) |
   | File format | Android String Resource |
   | File mask | `android/app/src/main/res/values-*/strings.xml` |
   | Monolingual base language file | `android/app/src/main/res/values/strings.xml` |
   | Edit base file | off (English is changed through pull requests) |
   | Source language | English |
   | Language code style | Android style (`values-pt-rBR`) |
   | Translation licence | MPL-2.0 |

4. Add-ons: **Squash Git commits** (per language), **Cleanup translation files**, and **Add missing languages** off. Set the commit message to start with `chore(i18n):` so the commits follow Conventional Commits.
5. Under *Version control*, turn on pull requests to `main` from the `weblate` branch. Each one runs CI, including the placeholder check. A translation PR needs no changelog fragment, and a maintainer reviews it like any other.
6. Set up the GitHub webhook Weblate shows, so it picks up new English strings as soon as they're merged.
7. Add the badge to README.md under "Want to help?":

   ```markdown
   [![Translation status](https://hosted.weblate.org/widget/mailcrane/android/svg-badge.svg)](https://hosted.weblate.org/engage/mailcrane/)
   ```

   Put the project link in step 1 of "For translators" above, and in CONTRIBUTING.md.
8. Test it: translate one string into a new language, check that the PR builds, install the build, and pick the language under *Settings → Apps → Mailcrane → Language* on Android 13 or newer.

The store listing (`fastlane/metadata`) and the website can become further components later.
