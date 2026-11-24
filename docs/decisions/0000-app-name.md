# 0000: App Name

* **Status:** Proposed
* **Date:** 2026-11-24

## Context

The project requires a distinct, memorable application name prior to public release. The name must not collide with existing email or messaging clients on major mobile/desktop app stores or cause user confusion during distribution and onboarding. Initial branding assets have already been created around the name **Mailcrane**.

## Decision

The application name will be **Mailcrane**.

## Why

- **App Store Availability:** No standalone mobile or desktop email/messaging client uses the name "Mailcrane" on Google Play, the Apple App Store, F-Droid, or the Microsoft Store.
- **Asset Readiness:** Visual identity and branding assets have already been designed for Mailcrane.
- **Clarity:** Clearly communicates the application's purpose (email) while remaining distinctive.

## Store & Web Availability Search (2026-11-24)

Searches conducted on **2026-11-24** across **UK, US, and EU (DE)** storefronts and general web indices for the terms: `Mailcrane`, `Mail Crane`, `MailCrane`, `Crane Mail`, `Crane Email`, and `crane` (filtered to Productivity / Business categories).

| Store / Source | Regions Checked | Query / Hit Found | Developer / Operator | Category | Email / Messaging App? | Conflict Assessment |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **Google Play** | UK, US, DE | *No exact or confusingly similar email/messaging hits* (only unrelated crane/construction logistics utilities under `"crane"`) | Various | Tools / Business | No | **No conflict** |
| **Apple App Store** | UK, US, DE | *No exact or confusingly similar email/messaging hits* (only unrelated construction/logistics apps under `"crane"`) | Various | Business / Utilities | No | **No conflict** |
| **F-Droid** | Global | *No results* for Mailcrane, Crane Mail, or Crane | N/A | N/A | No | **No conflict** |
| **Microsoft Store** | UK, US, DE | *No exact or confusingly similar email/messaging hits* | N/A | N/A | No | **No conflict** |
| **Web Search** | Global | **CraneMail** (`cranemail.com` / `crane.email`) | NameCrane | Email Hosting / Webmail Suite | Yes (hosted email provider, not a standalone client app) | **Minor** — Reversed word order (`CraneMail` vs `Mailcrane`), operates as a B2B domain email host rather than a multi-provider native client app. Flagged for trademark review in #78. |
| **Web Search** | US | **The Crane Handwriting app** (`crane.com`) | Crane Stationery | Physical stationery / postal note service | No (physical mail fulfillment) | **No conflict** — Different domain (physical stationery vs digital email client). |

### Search Conclusion

There are **no blocking conflicts** in Google Play, the Apple App Store, F-Droid, or the Microsoft Store across the UK, US, and EU regions. The closest existing service is **CraneMail** (by NameCrane), which is a web-based business email hosting provider rather than a native mobile/desktop email client. Proceeding with **Mailcrane** is viable, subject to formal trademark clearance in **#78**.

## Consequences

- **Positive:** Development and release preparation can proceed using the **Mailcrane** name and existing visual assets without rebranding delays.
- **Negative / Risks:** Users searching for "CraneMail" (or misremembering it as "Mailcrane") may occasionally encounter NameCrane's hosting service in web search results.
- **Follow-up:** Formal trademark checks for "Mailcrane" will be tracked separately in issue **#78**.
