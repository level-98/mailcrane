# 0000: App Name, Primary Domain, and Brand Identity

* **Status:** Accepted
* **Date:** 2026-11-24
* **Deciders:** Maintainers

## Context

The project required a distinct, official application name and primary internet domain before public launch. The identity must avoid app store collisions, pass trademark availability checks, and provide an official domain for web hosting, reverse-DNS app identifier namespaces, and contact endpoints (`security@`, `privacy@`).

## Decision

1. The project application name is **Mailcrane**.
2. The official domain for the project will be **`mailcrane.app`**.
3. **Application Identifiers:** The Android application ID and the iOS bundle identifier are both **`app.mailcrane`**. There is no Windows app (see [`docs/architecture.md`](../architecture.md#portability)).

## Key Drivers & Findings

### 1. Domain Registration & Availability (P-01)
* **`mailcrane.app` (Acquired):** Purchased and designated as the primary project domain. Registrar settings configured with project-owned account, 2FA enabled, WHOIS privacy active, and auto-renew enabled.
* **`mailcrane.dev`:** Available (HTTPS-only/HSTS preloaded; reserved as optional secondary endpoint).
* **`mailcrane.com` / `mailcrane.org`:** Taken/Parked by third-party domain brokerages. Non-blocking since `.app` provides an ideal HTTPS-only baseline for app distribution and web content.

### 2. Store & Web Availability Search (P-03)
Searches conducted across UK, US, and EU (DE) storefronts on 2026-11-24 for `Mailcrane`, `Mail Crane`, `MailCrane`, `Crane Mail`, `Crane Email`, and `crane` (Productivity / Business categories):
* **Google Play / Apple App Store / Microsoft Store / F-Droid:** Zero matching or confusingly similar email/messaging applications found.
* **Web Index:** `CraneMail` (`cranemail.com`) identified as a third-party B2B webmail hosting provider. Assessed as low conflict due to distinct product model (hosting provider vs. client app) and reversed naming order.

### 3. Trademark Clearance Search (P-04)
Searches conducted across UKIPO, USPTO, EUIPO, and WIPO databases in Nice Classes 9 (downloadable software), 38 (telecommunications/email), and 42 (SaaS):
* **`MAILCRANE` / `MAIL CRANE` / Phonetic variants:** No registered or pending trademarks found in relevant classes across targeted regions.
* **Registration strategy:** Formal trademark registration will be deferred until post-launch milestones to minimize early expenditure, relying on common-law/first-use rights established via public release.

## Consequences

* **Branding & Assets:** All project branding, logos, documentation, and interface references are finalized around **Mailcrane**.
* **App Infrastructure & IDs (P-05):** The Android package and iOS bundle identifier are `app.mailcrane`; reserving it in the stores and on F-Droid is still open (P-05).
* **Services & Hosting (#74, #75):** Static site deployment (#74) will target `mailcrane.app`. Forwarding for `security@mailcrane.app` and `privacy@mailcrane.app` will be routed directly to maintainers.
* **Refactoring Risk:** Future renaming would require updates across domain routing, app bundle IDs, app store listings, and design assets.
