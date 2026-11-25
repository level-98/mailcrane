//! Acting on emails with `Email/set` (RFC 8621 §4.6, RFC 8620 §5.3):
//! keywords such as read and flagged, moving between mailboxes, and
//! destroying.

use crate::http::{Auth, ClientConfig};
use crate::request::{call, take, MethodCall};
use crate::session::Session;
use crate::JmapError;
use serde_json::{json, Map, Value};

/// Most emails changed in one `Email/set`.
pub const MAX_SET: usize = 500;

/// What happened to each email (RFC 8620 §5.3).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SetOutcome {
    /// The email state after the change.
    pub new_state: String,
    /// Ids changed or destroyed.
    pub done: Vec<String>,
    /// Ids the server refused, with the SetError type (for example
    /// `notFound`, `forbidden`).
    pub failed: Vec<(String, String)>,
}

/// One change to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmailAction {
    /// Sets or clears a keyword (RFC 8621 §4.1.1), for example `$seen`.
    Keyword { keyword: String, on: bool },
    /// Moves out of `from` into `to`; other mailboxes are kept (RFC 8621
    /// §4.1.1 `mailboxIds`).
    Move { from: String, to: String },
    /// Destroys the emails (RFC 8620 §5.3 `destroy`).
    Destroy,
}

/// A keyword is lowercase ASCII 0x21–0x7E without `( ) { ] % * " \`
/// (RFC 8621 §4.1.1, from RFC 3501 `atom`), at most 255 characters.
pub fn valid_keyword(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= 255
        && k.bytes().all(|b| {
            (0x21..=0x7e).contains(&b) && !b"(){]%*\"\\".contains(&b) && !b.is_ascii_uppercase()
        })
}

/// Escapes one JSON Pointer token (RFC 6901 §3), as patch paths need
/// (RFC 8620 §5.3).
fn pointer_token(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}

/// The `Email/set` arguments for `action` on `ids`.
pub fn set_args(
    account_id: &str,
    ids: &[String],
    action: &EmailAction,
) -> Result<Value, JmapError> {
    if ids.is_empty() || ids.len() > MAX_SET {
        return Err(JmapError::TooLarge);
    }
    let patch = match action {
        EmailAction::Keyword { keyword, on } => {
            if !valid_keyword(keyword) {
                return Err(JmapError::Malformed);
            }
            // A patch sets true, or removes with null.
            let mut p = Map::new();
            p.insert(
                format!("keywords/{}", pointer_token(keyword)),
                if *on { Value::Bool(true) } else { Value::Null },
            );
            p
        }
        EmailAction::Move { from, to } => {
            if from.is_empty() || to.is_empty() {
                return Err(JmapError::Malformed);
            }
            let mut p = Map::new();
            if from != to {
                p.insert(format!("mailboxIds/{}", pointer_token(from)), Value::Null);
            }
            p.insert(
                format!("mailboxIds/{}", pointer_token(to)),
                Value::Bool(true),
            );
            p
        }
        EmailAction::Destroy => {
            return Ok(json!({"accountId": account_id, "destroy": ids}));
        }
    };
    let update: Map<String, Value> = ids
        .iter()
        .map(|id| (id.clone(), Value::Object(patch.clone())))
        .collect();
    Ok(json!({"accountId": account_id, "update": update}))
}

/// Reads an `Email/set` response into a [`SetOutcome`].
pub fn parse_set(args: &Value) -> Result<SetOutcome, JmapError> {
    let mut out = SetOutcome {
        new_state: args
            .get("newState")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        ..SetOutcome::default()
    };
    if let Some(updated) = args.get("updated").and_then(Value::as_object) {
        out.done.extend(updated.keys().take(MAX_SET).cloned());
    }
    if let Some(destroyed) = args.get("destroyed").and_then(Value::as_array) {
        out.done.extend(
            destroyed
                .iter()
                .take(MAX_SET)
                .filter_map(Value::as_str)
                .map(str::to_owned),
        );
    }
    for key in ["notUpdated", "notDestroyed"] {
        if let Some(failed) = args.get(key).and_then(Value::as_object) {
            out.failed
                .extend(failed.iter().take(MAX_SET).map(|(id, e)| {
                    let kind = e.get("type").and_then(Value::as_str).unwrap_or("unknown");
                    (id.clone(), kind.chars().take(64).collect())
                }));
        }
    }
    Ok(out)
}

/// Applies `action` to `ids` in one `Email/set`.
pub async fn apply(
    session: &Session,
    auth: &Auth,
    config: &ClientConfig,
    ids: &[String],
    action: &EmailAction,
) -> Result<SetOutcome, JmapError> {
    let calls = [MethodCall {
        name: "Email/set".into(),
        args: set_args(&session.mail_account_id, ids, action)?,
        id: "s0".into(),
    }];
    parse_set(&take(call(session, auth, config, &calls).await?, "s0")?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_are_checked() {
        for good in ["$seen", "$flagged", "$junk", "work-2026"] {
            assert!(valid_keyword(good), "{good}");
        }
        for bad in [
            "",
            "$Seen",
            "a b",
            "a(b",
            "a\\b",
            "a*b",
            "é",
            &"x".repeat(256),
        ] {
            assert!(!valid_keyword(bad), "{bad}");
        }
    }

    #[test]
    fn patches() {
        let ids = vec!["M1".to_owned(), "M2".to_owned()];
        let seen = set_args(
            "A1",
            &ids,
            &EmailAction::Keyword {
                keyword: "$seen".into(),
                on: true,
            },
        )
        .expect("args");
        assert_eq!(seen["update"]["M2"]["keywords/$seen"], json!(true));
        let unflag = set_args(
            "A1",
            &ids[..1],
            &EmailAction::Keyword {
                keyword: "$flagged".into(),
                on: false,
            },
        )
        .expect("args");
        assert_eq!(unflag["update"]["M1"]["keywords/$flagged"], Value::Null);
        let moved = set_args(
            "A1",
            &ids[..1],
            &EmailAction::Move {
                from: "in/box".into(),
                to: "arch~ive".into(),
            },
        )
        .expect("args");
        assert_eq!(moved["update"]["M1"]["mailboxIds/in~1box"], Value::Null);
        assert_eq!(moved["update"]["M1"]["mailboxIds/arch~0ive"], json!(true));
        let gone = set_args("A1", &ids, &EmailAction::Destroy).expect("args");
        assert_eq!(gone["destroy"], json!(["M1", "M2"]));
        assert_eq!(
            set_args("A1", &[], &EmailAction::Destroy),
            Err(JmapError::TooLarge)
        );
        assert_eq!(
            set_args(
                "A1",
                &ids,
                &EmailAction::Keyword {
                    keyword: "Bad Word".into(),
                    on: true
                }
            ),
            Err(JmapError::Malformed)
        );
    }

    #[test]
    fn outcomes() {
        let out = parse_set(&json!({
            "newState": "e2",
            "updated": {"M1": null},
            "destroyed": ["M2"],
            "notUpdated": {"M3": {"type": "notFound"}}
        }))
        .expect("outcome");
        assert_eq!(out.new_state, "e2");
        assert_eq!(out.done, ["M1", "M2"]);
        assert_eq!(out.failed, [("M3".to_owned(), "notFound".to_owned())]);
    }
}
