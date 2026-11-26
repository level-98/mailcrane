# App passwords: iCloud, Yahoo and AOL

These providers don't let other apps sign in with your normal password. You make an **app password** for Mailcrane on the provider's website and type that instead. When you enter an address on one of their domains, Mailcrane fills in the server settings itself (no lookup) and shows these steps, with a button that opens the provider's page in your browser.

> These steps and settings still need checking with a real account for each provider (issue #47). The results go in the provider test matrix.

## iCloud Mail (icloud.com, me.com, mac.com)

1. Open your Apple Account page (account.apple.com) and sign in.
2. Under **Sign-In and Security**, choose **App-Specific Passwords**. Two-factor authentication must be on.
3. Create a password, name it Mailcrane, and copy it.
4. Paste it into Mailcrane as the app password.

Settings: IMAP `imap.mail.me.com`, port 993, TLS. SMTP `smtp.mail.me.com`, port 587, STARTTLS. Username: your full address.

## Yahoo Mail (yahoo.com, ymail.com, rocketmail.com and Yahoo's country domains)

1. Open your Yahoo account security page (login.yahoo.com/account/security) and sign in.
2. Choose **Generate app password** (or **Manage app passwords**).
3. Enter Mailcrane as the app name, choose **Generate**, and copy the password.
4. Paste it into Mailcrane as the app password.

Settings: IMAP `imap.mail.yahoo.com`, port 993, TLS. SMTP `smtp.mail.yahoo.com`, port 465, TLS. Username: your full address. Yahoo Japan (`yahoo.co.jp`) is a separate service and isn't included.

## AOL Mail (aol.com, aim.com and AOL's country domains)

1. Open your AOL account security page (login.aol.com/account/security) and sign in.
2. Choose **Generate app password** (or **Manage app passwords**).
3. Enter Mailcrane as the app name, choose **Generate**, and copy the password.
4. Paste it into Mailcrane as the app password.

Settings: IMAP `imap.aol.com`, port 993, TLS. SMTP `smtp.aol.com`, port 465, TLS. Username: your full address.

## For developers

The domains, settings and links are in `core/crates/mailcrane-core/src/account/presets.rs`. The steps shown in the app are in `android/app/src/main/res/values/strings.xml` (`app_password_steps_*`), so they can be translated. Keep the three in step with this page.
