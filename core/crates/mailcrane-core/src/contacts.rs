//! Recipient suggestions from past correspondents (M2-08).
//!
//! The store counts who the user hears from (triggers on stored messages)
//! and [`record_sent`] counts who they write to. [`suggest`] matches a
//! typed prefix against the start of the name, any word of the name, or
//! the start of the address, ignoring case and accents, and ranks by
//! `3 × sent + received`, halved for every 90 days since they were last
//! seen. Device contacts are merged in by the apps, never stored here.

use mailcrane_mime::address::parse_address_list;
use mailcrane_store::{AccountId, Correspondent, OutboxMessage, Store, StoreError};

/// Most suggestions returned.
pub const MAX_SUGGESTIONS: usize = 50;

/// Days for the score to halve.
const HALF_LIFE_DAYS: f64 = 90.0;

/// One suggested recipient.
#[derive(Debug, Clone, PartialEq)]
pub struct ContactSuggestion {
    /// Lower-case address.
    pub email: String,
    /// Display name; may be empty.
    pub name: String,
    /// Ranking score (higher first).
    pub score: f64,
}

/// Counts a sent message for its To, Cc and Bcc recipients.
pub fn record_sent(store: &Store, message: &OutboxMessage, now: i64) -> Result<(), StoreError> {
    let f = &message.fields;
    let mut recipients: Vec<(String, String)> = Vec::new();
    for entry in f.to.iter().chain(&f.cc).chain(&f.bcc) {
        let Ok(list) = parse_address_list(entry) else {
            continue;
        };
        for a in list {
            let email = a.email.to_lowercase();
            if !recipients.iter().any(|(e, _)| *e == email) {
                recipients.push((email, a.name.unwrap_or_default()));
            }
        }
    }
    store.record_sent_to(message.account_id, &recipients, now)
}

/// Up to `limit` (at most 50) suggestions for `prefix`, best first, from
/// `account`'s correspondents (`None`: every account's, merged by address).
pub fn suggest(
    store: &Store,
    account: Option<AccountId>,
    prefix: &str,
    limit: usize,
    now: i64,
) -> Result<Vec<ContactSuggestion>, StoreError> {
    let query = fold(prefix.trim());
    if query.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let mut merged: Vec<Correspondent> = Vec::new();
    let mut index = std::collections::HashMap::new();
    for c in store.list_correspondents(account)? {
        if !matches(&query, &c) {
            continue;
        }
        match index.get(&c.email) {
            Some(&i) => {
                let m: &mut Correspondent = &mut merged[i];
                m.sent_to_count = m.sent_to_count.saturating_add(c.sent_to_count);
                m.received_from_count = m.received_from_count.saturating_add(c.received_from_count);
                if c.last_seen_at > m.last_seen_at {
                    m.last_seen_at = c.last_seen_at;
                    if !c.name.is_empty() {
                        m.name = c.name;
                    }
                }
            }
            None => {
                index.insert(c.email.clone(), merged.len());
                merged.push(c);
            }
        }
    }
    let mut out: Vec<ContactSuggestion> = merged
        .into_iter()
        .map(|c| ContactSuggestion {
            score: score(&c, now),
            email: c.email,
            name: c.name,
        })
        .collect();
    out.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.email.cmp(&b.email))
    });
    out.truncate(limit.min(MAX_SUGGESTIONS));
    Ok(out)
}

fn matches(query: &str, c: &Correspondent) -> bool {
    if fold(&c.email).starts_with(query) {
        return true;
    }
    let name = fold(&c.name);
    name.starts_with(query)
        || name
            .split(|ch: char| !ch.is_alphanumeric())
            .any(|word| !word.is_empty() && word.starts_with(query))
}

/// `(3 × sent + received) × 0.5^(days since last seen / 90)`.
pub fn score(c: &Correspondent, now: i64) -> f64 {
    let base = 3.0 * f64::from(c.sent_to_count) + f64::from(c.received_from_count);
    let age_days = (now.saturating_sub(c.last_seen_at)).max(0) as f64 / 86_400.0;
    base * 0.5_f64.powf(age_days / HALF_LIFE_DAYS)
}

/// Lower-case, with accents and other marks removed from Latin letters
/// (`Élodie` → `elodie`, `Łukasz` → `lukasz`, `Straße` → `strasse`).
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase) {
        match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => out.push('a'),
            'æ' => out.push_str("ae"),
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => out.push('c'),
            'ď' | 'đ' | 'ð' => out.push('d'),
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => out.push('e'),
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => out.push('g'),
            'ĥ' | 'ħ' => out.push('h'),
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => out.push('i'),
            'ĵ' => out.push('j'),
            'ķ' => out.push('k'),
            'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => out.push('l'),
            'ñ' | 'ń' | 'ņ' | 'ň' => out.push('n'),
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => out.push('o'),
            'œ' => out.push_str("oe"),
            'ŕ' | 'ŗ' | 'ř' => out.push('r'),
            'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => out.push('s'),
            'ß' => out.push_str("ss"),
            'ţ' | 'ť' | 'ŧ' | 'ț' => out.push('t'),
            'þ' => out.push_str("th"),
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => out.push('u'),
            'ŵ' => out.push('w'),
            'ý' | 'ÿ' | 'ŷ' => out.push('y'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            // Combining marks left over from decomposed input.
            '\u{0300}'..='\u{036f}' => {}
            other => out.push(other),
        }
    }
    out
}
