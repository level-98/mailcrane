//! Folder roles: SPECIAL-USE attributes (RFC 6154 §2) first, then
//! well-known names for servers that do not mark their folders.

use mailcrane_store::{FolderRole, VirtualRole};

/// The role a SPECIAL-USE attribute names, if any (RFC 6154 §2).
/// `\Flagged` is a virtual folder and has no role here: see [`virtual_role`].
pub fn role_from_attribute(attribute: &str) -> Option<FolderRole> {
    const TABLE: [(&str, FolderRole); 6] = [
        ("\\All", FolderRole::All),
        ("\\Archive", FolderRole::Archive),
        ("\\Drafts", FolderRole::Drafts),
        ("\\Junk", FolderRole::Spam),
        ("\\Sent", FolderRole::Sent),
        ("\\Trash", FolderRole::Trash),
    ];
    TABLE
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(attribute))
        .map(|(_, role)| *role)
}

/// The virtual folder a folder's attributes mark, if any (#390): RFC 6154
/// `\Flagged` is starred mail on any server. `\Important` isn't in RFC 6154:
/// it's Gmail's own attribute for its Important folder, so it counts only on
/// Gmail (`gmail`: [`crate::ServerQuirk::GmailLabels`]). Kept out of
/// [`role_from_attribute`] so these folders never get a [`FolderRole`]:
/// nothing moves, archives or deletes into them.
pub fn virtual_role(attributes: &[String], gmail: bool) -> Option<VirtualRole> {
    attributes.iter().find_map(|a| {
        if a.eq_ignore_ascii_case("\\Flagged") {
            Some(VirtualRole::Starred)
        } else if gmail && a.eq_ignore_ascii_case("\\Important") {
            Some(VirtualRole::Important)
        } else {
            None
        }
    })
}

/// Names that mean a role, in order of preference.
const FALLBACK_NAMES: [(FolderRole, &[&str]); 5] = [
    (FolderRole::Sent, &["Sent", "Sent Items", "Sent Messages"]),
    (FolderRole::Drafts, &["Drafts"]),
    (
        FolderRole::Trash,
        &["Trash", "Deleted Items", "Deleted Messages"],
    ),
    (FolderRole::Spam, &["Junk", "Spam"]),
    (FolderRole::Archive, &["Archive"]),
];

/// The parts of a listed folder that decide its role.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// Decoded name.
    pub name: &'a str,
    /// Hierarchy delimiter, if any.
    pub delimiter: Option<&'a str>,
    /// Attributes from `LIST`, e.g. `\Sent`.
    pub attributes: &'a [String],
}

/// Assigns a role to each folder.
///
/// `INBOX` (any case, RFC 9051 §5.1) is always the inbox. A SPECIAL-USE
/// attribute wins next. For each role no attribute claimed, the first
/// folder with a well-known name gets it, either at the top level or
/// directly under `INBOX` (`INBOX.Sent`), so each role is used at most once
/// by the fallback.
pub fn assign(folders: &[Candidate<'_>]) -> Vec<Option<FolderRole>> {
    let mut roles: Vec<Option<FolderRole>> = folders
        .iter()
        .map(|f| {
            if f.name.eq_ignore_ascii_case("INBOX") {
                Some(FolderRole::Inbox)
            } else {
                f.attributes.iter().find_map(|a| role_from_attribute(a))
            }
        })
        .collect();

    for (role, names) in FALLBACK_NAMES {
        if roles.contains(&Some(role)) {
            continue;
        }
        let found = names.iter().find_map(|wanted| {
            folders.iter().enumerate().find_map(|(i, f)| {
                (roles.get(i).is_some_and(Option::is_none) && matches_name(f, wanted)).then_some(i)
            })
        });
        if let Some(slot) = found.and_then(|i| roles.get_mut(i)) {
            *slot = Some(role);
        }
    }
    roles
}

fn matches_name(folder: &Candidate<'_>, wanted: &str) -> bool {
    if folder.name.eq_ignore_ascii_case(wanted) {
        return true;
    }
    // Some servers keep everything under INBOX, e.g. "INBOX.Sent".
    let Some(delimiter) = folder.delimiter.filter(|d| !d.is_empty()) else {
        return false;
    };
    let mut parts = folder.name.splitn(2, delimiter);
    matches!(
        (parts.next(), parts.next()),
        (Some(top), Some(leaf)) if top.eq_ignore_ascii_case("INBOX") && leaf.eq_ignore_ascii_case(wanted)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand<'a>(name: &'a str, attributes: &'a [String]) -> Candidate<'a> {
        Candidate {
            name,
            delimiter: Some("/"),
            attributes,
        }
    }

    #[test]
    fn attributes_map_to_roles() {
        assert_eq!(role_from_attribute("\\Sent"), Some(FolderRole::Sent));
        assert_eq!(role_from_attribute("\\junk"), Some(FolderRole::Spam));
        assert_eq!(role_from_attribute("\\Flagged"), None);
        assert_eq!(role_from_attribute("\\HasNoChildren"), None);
    }

    #[test]
    fn gmails_starred_and_important_are_virtual_folders_with_no_role() {
        // #390: "[Gmail]/Starred" carries \Flagged and "[Gmail]/Important"
        // Gmail's own \Important, in any case. They get a virtual role and
        // no role, so nothing can move into them.
        let starred = vec!["\\HasNoChildren".to_string(), "\\Flagged".to_string()];
        let important = vec!["\\important".to_string()];
        assert_eq!(virtual_role(&starred, true), Some(VirtualRole::Starred));
        assert_eq!(virtual_role(&important, true), Some(VirtualRole::Important));
        assert_eq!(virtual_role(&["\\Sent".to_string()], true), None);
        assert_eq!(virtual_role(&[], true), None);
        // \Flagged is RFC 6154 on any server; \Important is Gmail's own, so a
        // server without the Gmail quirk doesn't get it.
        assert_eq!(virtual_role(&starred, false), Some(VirtualRole::Starred));
        assert_eq!(virtual_role(&important, false), None);
        let folders = [
            cand("INBOX", &[]),
            cand("[Gmail]/Starred", &starred),
            cand("[Gmail]/Important", &important),
        ];
        assert_eq!(assign(&folders), vec![Some(FolderRole::Inbox), None, None]);
    }

    #[test]
    fn attribute_beats_name() {
        let sent = vec!["\\Sent".to_string()];
        let folders = [
            cand("inbox", &[]),
            cand("Sent", &[]),
            cand("Gesendet", &sent),
        ];
        assert_eq!(
            assign(&folders),
            [Some(FolderRole::Inbox), None, Some(FolderRole::Sent)]
        );
    }

    #[test]
    fn names_fill_in_unclaimed_roles_once() {
        let folders = [
            cand("INBOX", &[]),
            cand("Sent Items", &[]),
            cand("Sent", &[]),
            cand("Deleted Messages", &[]),
            cand("Spam", &[]),
            cand("Projects/Archive", &[]),
            cand("Notes", &[]),
        ];
        assert_eq!(
            assign(&folders),
            [
                Some(FolderRole::Inbox),
                None,
                Some(FolderRole::Sent),
                Some(FolderRole::Trash),
                Some(FolderRole::Spam),
                None,
                None,
            ]
        );
    }

    #[test]
    fn names_under_inbox_count() {
        let folders = [
            Candidate {
                name: "INBOX.Drafts",
                delimiter: Some("."),
                attributes: &[],
            },
            Candidate {
                name: "INBOX.Junk",
                delimiter: Some("."),
                attributes: &[],
            },
        ];
        assert_eq!(
            assign(&folders),
            [Some(FolderRole::Drafts), Some(FolderRole::Spam)]
        );
    }
}
