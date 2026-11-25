//! Deciding whether a rule matches a message.

use crate::contacts::fold;
use mailcrane_store::{MessageHeader, NewRule, RuleCondition};

/// What conditions look at, prepared once per message (folded text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFacts {
    from: String,
    to_cc: String,
    subject: String,
    has_attachment: bool,
    list_id: Option<String>,
    age_secs: i64,
    new_mail: bool,
}

impl MessageFacts {
    /// `new_mail`: rules on newly arriving mail, where "older than" never
    /// matches.
    pub fn new(header: &MessageHeader, list_id: Option<&str>, now: i64, new_mail: bool) -> Self {
        let d = &header.details;
        let from = format!(
            "{} {}",
            d.from_name.as_deref().unwrap_or_default(),
            d.from_addr.as_deref().unwrap_or_default()
        );
        let mut to_cc = json_string_values(&d.to_json);
        to_cc.extend(json_string_values(&d.cc_json));
        Self {
            from: fold(&from),
            to_cc: fold(&to_cc.join(" \u{1f} ")),
            subject: fold(&d.subject),
            has_attachment: d.has_attachments,
            list_id: list_id.map(str::to_ascii_lowercase),
            age_secs: now.saturating_sub(d.date),
            new_mail,
        }
    }
}

fn one(c: &RuleCondition, f: &MessageFacts) -> bool {
    let contains = |hay: &str, needle: &str| {
        let n = fold(needle.trim());
        !n.is_empty() && hay.contains(&n)
    };
    match c {
        RuleCondition::From(v) => contains(&f.from, v),
        RuleCondition::ToCc(v) => contains(&f.to_cc, v),
        RuleCondition::Subject(v) => contains(&f.subject, v),
        RuleCondition::HasAttachment => f.has_attachment,
        RuleCondition::ListId(v) => {
            let want = v
                .trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_ascii_lowercase();
            !want.is_empty() && f.list_id.as_deref() == Some(want.as_str())
        }
        RuleCondition::OlderThanDays(days) => {
            !f.new_mail && f.age_secs > i64::from(*days).saturating_mul(86_400)
        }
    }
}

/// Whether `rule` matches. A rule without conditions or actions never
/// matches (so an empty rule can't move all mail).
pub fn evaluate(rule: &NewRule, facts: &MessageFacts) -> bool {
    if rule.conditions.is_empty() || rule.actions.is_empty() {
        return false;
    }
    if rule.match_all {
        rule.conditions.iter().all(|c| one(c, facts))
    } else {
        rule.conditions.iter().any(|c| one(c, facts))
    }
}

/// Every string value in a JSON text (keys left out), unescaped. Enough
/// for the store's `[{"name": …, "email": …}]` address lists.
fn json_string_values(json: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = json.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '"' {
            continue;
        }
        let mut s = String::new();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        if let Some(ch) =
                            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                        {
                            s.push(ch);
                        }
                    }
                    Some(other) => s.push(other),
                    None => break,
                },
                other => s.push(other),
            }
        }
        // A string followed by `:` is a key.
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek() != Some(&':') {
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{BodyState, Flags, FolderId, MessageId, NewMessageHeader, RuleAction};

    const NOW: i64 = 1_795_550_400;

    fn header(
        from: (&str, &str),
        to: &str,
        subject: &str,
        attach: bool,
        date: i64,
    ) -> MessageHeader {
        MessageHeader {
            id: MessageId(1),
            details: NewMessageHeader {
                folder_id: FolderId(1),
                uid: 1,
                message_id_header: None,
                in_reply_to: None,
                references_json: "[]".into(),
                thread_id: None,
                subject: subject.into(),
                from_name: Some(from.0.into()),
                from_addr: Some(from.1.into()),
                to_json: to.into(),
                cc_json: "[]".into(),
                date,
                internal_date: date,
                size: 1,
                flags: Flags::default(),
                keywords_json: "[]".into(),
                has_attachments: attach,
                snippet: String::new(),
            },
            body_state: BodyState::None,
        }
    }

    fn rule(match_all: bool, conditions: Vec<RuleCondition>) -> NewRule {
        NewRule {
            account_id: None,
            name: "r".into(),
            enabled: true,
            match_all,
            stop_processing: false,
            conditions,
            actions: vec![RuleAction::MarkRead],
        }
    }

    fn facts(new_mail: bool) -> MessageFacts {
        let h = header(
            ("Élodie Martin", "news@Shop.example"),
            r#"[{"name":"Team \"A\"","email":"team@work.example"},{"name":null,"email":"bob@example.net"}]"#,
            "Your WEEKLY Résumé",
            true,
            NOW - 10 * 86_400,
        );
        MessageFacts::new(&h, Some("Deals.Shop.Example"), NOW, new_mail)
    }

    #[test]
    fn each_condition() {
        let f = facts(false);
        let yes = |c: RuleCondition| evaluate(&rule(true, vec![c]), &f);
        assert!(yes(RuleCondition::From("elodie".into())));
        assert!(yes(RuleCondition::From("SHOP.EXAMPLE".into())));
        assert!(!yes(RuleCondition::From("work".into())));
        assert!(yes(RuleCondition::ToCc("team@work".into())));
        assert!(yes(RuleCondition::ToCc("team \"a\"".into())));
        assert!(yes(RuleCondition::ToCc("bob@".into())));
        assert!(
            !yes(RuleCondition::ToCc("email".into())),
            "keys aren't searched"
        );
        assert!(yes(RuleCondition::Subject("weekly resume".into())));
        assert!(!yes(RuleCondition::Subject("monthly".into())));
        assert!(yes(RuleCondition::HasAttachment));
        assert!(yes(RuleCondition::ListId("<deals.shop.example>".into())));
        assert!(
            !yes(RuleCondition::ListId("shop.example".into())),
            "exact, not contains"
        );
        assert!(yes(RuleCondition::OlderThanDays(7)));
        assert!(!yes(RuleCondition::OlderThanDays(30)));
        // Empty text never matches.
        assert!(!yes(RuleCondition::Subject("  ".into())));
    }

    #[test]
    fn older_than_never_matches_new_mail() {
        let f = facts(true);
        assert!(!evaluate(
            &rule(true, vec![RuleCondition::OlderThanDays(1)]),
            &f
        ));
    }

    #[test]
    fn all_and_any() {
        let f = facts(false);
        let hit = RuleCondition::From("shop".into());
        let miss = RuleCondition::Subject("invoice".into());
        assert!(!evaluate(&rule(true, vec![hit.clone(), miss.clone()]), &f));
        assert!(evaluate(&rule(false, vec![hit.clone(), miss.clone()]), &f));
        assert!(!evaluate(&rule(false, vec![miss]), &f));
        assert!(!evaluate(&rule(true, vec![]), &f), "no conditions: never");
        let mut no_actions = rule(true, vec![hit]);
        no_actions.actions.clear();
        assert!(!evaluate(&no_actions, &f), "no actions: never");
    }
}
