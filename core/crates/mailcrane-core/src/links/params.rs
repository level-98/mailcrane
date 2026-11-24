//! Query parameters that only exist to track clicks. [`super::clean_url`]
//! removes them from links before they are opened.

/// Any parameter whose name starts with one of these, compared
/// case-insensitively, is removed.
pub(super) const PREFIXES: &[&str] = &[
    // Google Analytics campaign tags (utm_source, utm_medium, utm_campaign,
    // utm_term, utm_content, utm_id and any others).
    "utm_",
];

/// Parameters removed when the name matches exactly (case-sensitive).
pub(super) const EXACT: &[&str] = &[
    // Ad click identifiers: Facebook, Google Ads (and its iOS variants),
    // Google Display & Video 360, Microsoft Ads, Yandex, X (Twitter),
    // TikTok, LinkedIn, Instagram.
    "fbclid",
    "gclid",
    "gclsrc",
    "dclid",
    "gbraid",
    "wbraid",
    "msclkid",
    "yclid",
    "twclid",
    "ttclid",
    "li_fat_id",
    "igshid",
    // Mailchimp.
    "mc_cid",
    "mc_eid",
    // HubSpot.
    "_hsenc",
    "_hsmi",
    "__hssc",
    "__hstc",
    "__hsfp",
    "hsCtaTracking",
    // Marketo.
    "mkt_tok",
    // Omeda Olytics.
    "oly_anon_id",
    "oly_enc_id",
    // Vero.
    "vero_id",
    "vero_conv",
    // Openstat.
    "_openstat",
    // Adobe Analytics campaigns.
    "s_cid",
    // Squarespace email campaigns.
    "ss_email_id",
    // Other click identifiers.
    "rb_clickid",
    "wickedid",
];

/// Whether a parameter called `name` is a tracking parameter.
pub(super) fn is_tracking(name: &str) -> bool {
    EXACT.contains(&name)
        || PREFIXES.iter().any(|prefix| {
            name.get(..prefix.len())
                .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
        })
}
