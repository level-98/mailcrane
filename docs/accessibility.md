# Accessibility

Mailcrane aims to be fully usable with TalkBack, at the largest text sizes and with animations off, and to meet WCAG 2.2 AA contrast (M6-01, issue #68). Automated checks keep that from regressing; a person checks the real experience on a device before each release.

## Automated checks (run on every build)

These run in `./gradlew testDebugUnitTest` (Robolectric, no device needed) and in `cargo xtask ci`:

| Check | Where | What it catches |
|---|---|---|
| Labels on controls | `android/app/src/test/java/app/mailcrane/a11y/A11yScreensTest.kt` | Any clickable control on the main screens (message list, message, compose, settings, manage folders) that TalkBack would read without a label |
| Touch targets | same | Clickable controls smaller than 48 × 48 dp |
| Headings | same | A main screen without a heading to jump to |
| Text at 200 % | same, each screen at font scale 2.0 | Text cut off vertically at the largest font size |
| Contrast, Android | `android/app/src/test/java/app/mailcrane/ui/theme/ContrastTest.kt` | Any text/background pair in the light or dark colour scheme under 4.5:1 |
| Contrast, palette | `design/gen_theme.py` (`cargo xtask ci`) | The same, for the brand palette in `design/colors.json`, before the theme is generated |
| Sender pictures | `SenderAvatarTest` | Initials on their coloured circles under 4.5:1 |

They can't judge whether a label makes sense, whether the reading order is natural, or how it feels to use. That's what the walk-through below is for.

## Walk-through before a release (a person, on a real device)

Record each run in the table at the end. Open an issue labelled `area:android` for every problem.

### Setup

- A real Android phone, recent Android version.
- TalkBack on (Settings → Accessibility → TalkBack).
- A test account only, never a personal one.

### 1. With TalkBack, without looking at the screen

Do each flow by swiping and double-tapping only:

- [ ] Add an account (automatic setup and manual setup).
- [ ] Read the inbox: each row reads sender, subject, time, read or unread, flagged, attachment.
- [ ] Open a message; read the header, the body and the attachments.
- [ ] Reply, and send.
- [ ] Attach a file to a new message.
- [ ] Archive a message (from the message, and with the row's actions).
- [ ] Search, and open a result.
- [ ] Change a setting (for example "Confirm links before opening").
- [ ] Open "Manage folders", create a folder, rename it, delete it.
- [ ] Check that dialogs (link preview, delete folder, unsubscribe) are announced and can be dismissed.
- [ ] Check that the phishing hints are read out: the "Verified"/"Failed" chip, the display-name banner, link warnings.

### 2. Largest text sizes

Settings → Display → Font size at the maximum (200 %), Display size larger, and Bold text on:

- [ ] Repeat the flows above, looking at the screen this time.
- [ ] Nothing is cut off or overlapping on the message list, message, compose, settings and manage folders screens, or in any dialog.
- [ ] The list rows and the message header still read in a sensible order.

### 3. Reduced motion

Settings → Accessibility → Remove animations:

- [ ] Opening messages, the drawer and dialogs doesn't animate distractingly.

### 4. Accessibility Scanner

Install Google's Accessibility Scanner and scan every main screen in light and dark mode:

- [ ] No unresolved suggestions, or each has an issue.

## Results

| Date | Device and Android version | Who | Flows passed | Problems (issues) |
|---|---|---|---|---|
| | | | | |
