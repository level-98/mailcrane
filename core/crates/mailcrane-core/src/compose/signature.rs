//! Signatures (M2-10): added to new messages, replies and forwards below a
//! `-- ` line, which other mail clients recognise and trim (RFC 3676 §4.3).

/// The signature separator: dash, dash, **space** (RFC 3676 §4.3).
pub const SEPARATOR: &str = "-- ";

/// What kind of draft a signature goes into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftKind {
    /// A new message: the signature goes at the end.
    New,
    /// A reply: the signature goes below the cursor, above the quote.
    Reply,
    /// A forward: like a reply, above the forwarded message.
    Forward,
}

/// The signature text as it goes into a body: CRLF and CR become LF and
/// trailing whitespace goes. `None` for an empty signature.
fn clean(signature: &str) -> Option<String> {
    let text = signature.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim_end();
    if text.trim().is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

/// The block added for `signature`: an empty line, the separator line, then
/// the signature.
fn block(signature: &str) -> Option<String> {
    clean(signature).map(|s| format!("\n\n{SEPARATOR}\n{s}"))
}

/// Adds `signature` to `body`. An empty (or `None`) signature adds nothing,
/// not even the separator: pass `None` when the signature is turned off.
///
/// For a new message the signature goes after the body. For a reply or
/// forward, `body` is the draft from `prepare_reply` (a blank line for the
/// cursor, then the quote) and the signature goes between the two.
pub fn apply_signature(body: &str, signature: Option<&str>, kind: DraftKind) -> String {
    let Some(block) = signature.and_then(block) else {
        return body.to_owned();
    };
    match kind {
        DraftKind::New => format!("{body}{block}"),
        DraftKind::Reply | DraftKind::Forward => format!("{block}\n{body}"),
    }
}

/// Replaces the `old` signature block in `body` with the `new` one (or
/// removes it when `new` is `None` or empty), for when the user changes the
/// From account. Only an exact, unedited copy of the old block is replaced;
/// if the user changed it, or there is none, `body` comes back unchanged.
///
/// `quote_header` is the line a reply's quote or a forward starts with (the
/// `quote_header` given to `prepare_reply`). Only the part above it is the
/// user's own (#449): a forwarded mail of theirs can hold the same block,
/// and is never touched. Without it (a new message, or a draft reopened
/// later) the whole body is searched.
pub fn swap_signature(
    body: &str,
    old: Option<&str>,
    new: Option<&str>,
    quote_header: Option<&str>,
) -> String {
    let new_block = new.and_then(block).unwrap_or_default();
    let header_at = quote_header
        .filter(|h| !h.trim().is_empty())
        .and_then(|h| body.find(&format!("\n{h}\n")));
    let own = body.get(..header_at.unwrap_or(body.len())).unwrap_or(body);
    let rest = body.get(own.len()..).unwrap_or_default();
    match old.and_then(block) {
        Some(old_block) => match whole_block_at(own, &old_block) {
            Some(at) => {
                let mut out = String::with_capacity(body.len() + new_block.len());
                out.push_str(own.get(..at).unwrap_or_default());
                out.push_str(&new_block);
                out.push_str(own.get(at + old_block.len()..).unwrap_or_default());
                out.push_str(rest);
                out
            }
            None => body.to_owned(),
        },
        // There was no old signature: add the new one only to an empty body
        // (a fresh new message), or just above a reply's quote or a forward,
        // where `apply_signature` puts it; anywhere else it might land in the
        // wrong place.
        None if body.is_empty() => new_block,
        None if header_at.is_some() && !new_block.is_empty() => {
            format!("{}{new_block}\n\n{rest}", own.trim_end_matches('\n'))
        }
        None => body.to_owned(),
    }
}

/// Where `block` appears in `body` as a whole signature: followed by the end
/// of the body or a line break, so "Ann" doesn't match an edited "Ann (on
/// holiday)". The last such place (#449): the signature goes below what the
/// user writes, so an earlier copy is text they pasted, such as an old mail
/// of theirs, and is left alone. A reply's quote can't hold one, as its
/// lines start with "> ".
fn whole_block_at(body: &str, block: &str) -> Option<usize> {
    body.match_indices(block)
        .map(|(at, _)| at)
        .filter(|at| {
            let rest = body.get(at + block.len()..).unwrap_or_default();
            rest.is_empty() || rest.starts_with('\n')
        })
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_go_where_each_kind_of_draft_needs_them() {
        let quote = "\n\nAna wrote:\n> Hi\n";
        for (body, signature, kind, want) in [
            ("", Some("Ann"), DraftKind::New, "\n\n-- \nAnn"),
            (
                "Hello",
                Some("Ann\nExample Ltd"),
                DraftKind::New,
                "Hello\n\n-- \nAnn\nExample Ltd",
            ),
            (
                quote,
                Some("Ann"),
                DraftKind::Reply,
                "\n\n-- \nAnn\n\n\nAna wrote:\n> Hi\n",
            ),
            (
                quote,
                Some("Ann"),
                DraftKind::Forward,
                "\n\n-- \nAnn\n\n\nAna wrote:\n> Hi\n",
            ),
            ("", Some(""), DraftKind::New, ""),
            ("", Some("  \n\t"), DraftKind::New, ""),
            ("", None, DraftKind::New, ""),
            (quote, None, DraftKind::Reply, quote),
            (
                "",
                Some("Ann\r\nLtd  \r\n\r\n"),
                DraftKind::New,
                "\n\n-- \nAnn\nLtd",
            ),
        ] {
            assert_eq!(
                apply_signature(body, signature, kind),
                want,
                "{body:?} {signature:?} {kind:?}"
            );
        }
    }

    #[test]
    fn the_separator_is_dash_dash_space() {
        assert_eq!(SEPARATOR.as_bytes(), b"-- ");
        let body = apply_signature("", Some("Ann"), DraftKind::New);
        assert!(body.as_bytes().windows(4).any(|w| w == b"-- \n"));
    }

    #[test]
    fn switching_from_swaps_only_an_unedited_signature() {
        let new_message = apply_signature("Hello", Some("Ann"), DraftKind::New);
        assert_eq!(
            swap_signature(&new_message, Some("Ann"), Some("Bob"), None),
            "Hello\n\n-- \nBob"
        );
        let reply = apply_signature("\n\nq\n> x\n", Some("Ann"), DraftKind::Reply);
        assert_eq!(
            swap_signature(&reply, Some("Ann"), Some("Bob"), None),
            "\n\n-- \nBob\n\n\nq\n> x\n"
        );
        // To an account without a signature: the block goes, separator too.
        assert_eq!(
            swap_signature(&new_message, Some("Ann"), None, None),
            "Hello"
        );
        assert_eq!(
            swap_signature(&new_message, Some("Ann"), Some(""), None),
            "Hello"
        );
        // Edited by the user: left alone.
        let edited = new_message.replace("Ann", "Ann (on holiday)");
        assert_eq!(
            swap_signature(&edited, Some("Ann"), Some("Bob"), None),
            edited
        );
        let separator_gone = new_message.replace("-- \n", "");
        assert_eq!(
            swap_signature(&separator_gone, Some("Ann"), Some("Bob"), None),
            separator_gone
        );
        // From an account without one: only an empty body gets the new one.
        assert_eq!(swap_signature("", None, Some("Bob"), None), "\n\n-- \nBob");
        assert_eq!(swap_signature("Typed", None, Some("Bob"), None), "Typed");
    }

    #[test]
    fn repeated_switches_find_the_block_by_content_and_never_duplicate_it() {
        // #430: the block is matched as a whole `-- ` block with the old signature's text, wherever it is.
        let mut body = apply_signature("Notes", Some("Ann"), DraftKind::New);
        for _ in 0..5 {
            body = swap_signature(&body, Some("Ann"), Some("Bea"), None);
            assert_eq!(body, "Notes\n\n-- \nBea");
            body = swap_signature(&body, Some("Bea"), Some("Ann"), None);
            assert_eq!(body, "Notes\n\n-- \nAnn");
        }
        assert_eq!(body.lines().filter(|l| *l == SEPARATOR).count(), 1);
    }

    #[test]
    fn text_around_the_signature_stays_where_it_is() {
        // Typed above it, and lines typed below it (a P.S.), are both kept; only the block changes.
        let body = "Hi there\n\n-- \nAnn\nP.S. see you Friday";
        assert_eq!(
            swap_signature(body, Some("Ann"), Some("Bea"), None),
            "Hi there\n\n-- \nBea\nP.S. see you Friday"
        );
        // A reply: the block between the cursor line and the quote.
        let reply = apply_signature("\n\nAna wrote:\n> Hi\n", Some("Ann"), DraftKind::Reply);
        let typed = format!("Thanks!{reply}");
        assert_eq!(
            swap_signature(&typed, Some("Ann"), Some("Bea"), None),
            "Thanks!\n\n-- \nBea\n\n\nAna wrote:\n> Hi\n"
        );
    }

    #[test]
    fn a_pasted_copy_above_the_signature_is_left_alone() {
        // #449: an earlier mail of the user's, pasted unquoted, holds the same block; the swap changes the
        // real signature at the end, not the copy.
        let body = "Hi\n\n-- \nAnn\nold text\n\n-- \nAnn";
        assert_eq!(
            swap_signature(body, Some("Ann"), Some("Bea"), None),
            "Hi\n\n-- \nAnn\nold text\n\n-- \nBea"
        );
    }

    #[test]
    fn forwarding_your_own_signed_mail_swaps_only_your_signature() {
        // #449 review: a forward is unquoted, so a forwarded mail of the
        // user's holds the same block as their signature. Only the part above
        // the forward's intro line is theirs.
        let intro = "---------- Forwarded message ---------";
        let forwarded = format!(
            "\n\n{intro}\nFrom: Ann <ann@example.org>\nSubject: Plans\n\nSee you\n\n-- \nAnn\n"
        );
        let draft = apply_signature(&forwarded, Some("Ann"), DraftKind::Forward);
        let typed = format!("FYI{draft}");
        let swapped = swap_signature(&typed, Some("Ann"), Some("Bea"), Some(intro));
        assert_eq!(
            swapped,
            format!("FYI\n\n-- \nBea\n\n\n{intro}\nFrom: Ann <ann@example.org>\nSubject: Plans\n\nSee you\n\n-- \nAnn\n")
        );
        // Without the boundary, the last block (the forwarded copy) would be the one changed.
        assert_ne!(
            swap_signature(&typed, Some("Ann"), Some("Bea"), None),
            swapped
        );
        // Edited above the intro: nothing changes, below it included.
        let edited = typed.replacen("Ann\n", "Ann (away)\n", 1);
        assert_eq!(
            swap_signature(&edited, Some("Ann"), Some("Bea"), Some(intro)),
            edited
        );
    }

    #[test]
    fn with_no_old_signature_a_reply_gets_the_new_one_above_its_quote() {
        // #449: From switched to an account with a signature, from one without.
        let header = "Ana wrote:";
        let reply = format!("Thanks!\n\n{header}\n> Hi\n");
        assert_eq!(
            swap_signature(&reply, None, Some("Bea"), Some(header)),
            format!("Thanks!\n\n-- \nBea\n\n\n{header}\n> Hi\n")
        );
        // The same layout apply_signature gives a fresh reply.
        let fresh = format!("\n\n{header}\n> Hi\n");
        assert_eq!(
            swap_signature(&fresh, None, Some("Bea"), Some(header)),
            apply_signature(&fresh, Some("Bea"), DraftKind::Reply)
        );
        // And nothing when the new account has none either.
        assert_eq!(swap_signature(&reply, None, None, Some(header)), reply);
    }
}
