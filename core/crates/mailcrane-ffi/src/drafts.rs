//! Drafts for the apps (M2-05): saved here at once, uploaded to the Drafts
//! folder by `SyncEngine::replay_actions` and before folder syncs.

use crate::actions::now;
use crate::compose::ComposeDraft;
use crate::messages::one_line;
use mailcrane_core::drafts::{self, DraftError};
use mailcrane_store::{AccountId, DraftFields, DraftId, MessageId, Store};

/// Why a draft call failed. Nothing was changed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum DraftFailure {
    /// The draft, message or account doesn't exist.
    #[error("not found")]
    NotFound,
    /// The message body isn't downloaded yet; call `fetch_body` first.
    #[error("body not downloaded")]
    BodyNotDownloaded,
    /// The local database failed, or something else inside the app.
    #[error("storage failure")]
    Storage,
}

impl From<DraftError> for DraftFailure {
    fn from(error: DraftError) -> Self {
        match error {
            DraftError::NotFound => Self::NotFound,
            DraftError::BodyNotDownloaded => Self::BodyNotDownloaded,
            DraftError::Internal | DraftError::Store(_) => Self::Storage,
        }
    }
}

/// A draft in a list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftSummary {
    pub id: i64,
    /// Recipients as typed, joined with ", ".
    pub to: String,
    pub subject: String,
    /// The start of the body, on one line.
    pub snippet: String,
    /// Unix seconds.
    pub updated_at: i64,
    /// Whether the newest version is in the server's Drafts folder.
    pub on_server: bool,
    /// A version changed on another device, kept beside this device's.
    pub is_conflict: bool,
}

fn open(db_path: &str) -> Result<Store, DraftFailure> {
    Store::open(db_path).map_err(|_| DraftFailure::Storage)
}

fn fields(draft: &ComposeDraft) -> DraftFields {
    DraftFields {
        to: draft.to.clone(),
        cc: draft.cc.clone(),
        bcc: draft.bcc.clone(),
        subject: draft.subject.clone(),
        body: draft.body_text.clone(),
        in_reply_to: draft.in_reply_to.clone(),
        references: draft.references.clone(),
    }
}

/// Saves `draft`: a new draft when `draft_id` is `None`, otherwise a new
/// version of that one. Instant; nothing goes over the network. Returns the
/// draft's id, to pass on the next save.
#[uniffi::export]
pub fn save_draft(
    db_path: String,
    draft_id: Option<i64>,
    draft: ComposeDraft,
) -> Result<i64, DraftFailure> {
    let store = open(&db_path)?;
    let id = drafts::save_draft(
        &store,
        AccountId(draft.account_id),
        draft_id.map(DraftId),
        &fields(&draft),
        now(),
    )?;
    Ok(id.0)
}

/// The saved draft, ready for the compose screen.
#[uniffi::export]
pub fn get_draft(db_path: String, draft_id: i64) -> Result<ComposeDraft, DraftFailure> {
    let draft = open(&db_path)?
        .get_draft(DraftId(draft_id))
        .map_err(|_| DraftFailure::Storage)?
        .ok_or(DraftFailure::NotFound)?;
    let f = draft.fields;
    Ok(ComposeDraft {
        account_id: draft.account_id.0,
        to: f.to,
        cc: f.cc,
        bcc: f.bcc,
        subject: f.subject,
        body_text: f.body,
        in_reply_to: f.in_reply_to,
        references: f.references,
        attachments: Vec::new(),
        forward_attachments_of: None,
        rich: None,
    })
}

/// Opens a message from the Drafts folder for editing. Returns the draft's
/// id: the one already linked to the message, or a new one made from it.
#[uniffi::export]
pub fn open_draft(db_path: String, message_id: i64) -> Result<i64, DraftFailure> {
    let store = open(&db_path)?;
    Ok(drafts::open_draft(&store, MessageId(message_id), now())?.0)
}

/// Deletes a draft (after it was sent, or when discarded), here at once and
/// on the server through the action queue.
#[uniffi::export]
pub fn delete_draft(db_path: String, draft_id: i64) -> Result<(), DraftFailure> {
    let store = open(&db_path)?;
    Ok(drafts::delete_draft(&store, DraftId(draft_id), now())?)
}

/// The account's drafts, most recently changed first.
#[uniffi::export]
pub fn list_drafts(db_path: String, account_id: i64) -> Result<Vec<DraftSummary>, DraftFailure> {
    let list = open(&db_path)?
        .list_drafts(AccountId(account_id))
        .map_err(|_| DraftFailure::Storage)?;
    Ok(list
        .into_iter()
        .map(|d| DraftSummary {
            id: d.id.0,
            to: one_line(&d.fields.to.join(", ")),
            subject: one_line(&d.fields.subject),
            snippet: one_line(&d.fields.body.chars().take(200).collect::<String>()),
            updated_at: d.updated_at,
            on_server: !d.dirty && d.server_uid.is_some(),
            is_conflict: d.is_conflict,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailcrane_store::{NewAccount, Security};

    fn setup() -> (tempfile::TempDir, String, i64) {
        let dir = tempfile::tempdir().expect("dir");
        let path = dir.path().join("d.db").to_string_lossy().into_owned();
        let account = Store::open(&path)
            .expect("store")
            .insert_account(&NewAccount {
                display_name: "Ann".into(),
                email: "ann@example.org".into(),
                imap_host: "imap.example.org".into(),
                imap_port: 993,
                imap_security: Security::Tls,
                smtp_host: "smtp.example.org".into(),
                smtp_port: 465,
                smtp_security: Security::Tls,
                username: "ann".into(),
            })
            .expect("account")
            .0;
        (dir, path, account)
    }

    fn compose(account_id: i64, body: &str) -> ComposeDraft {
        ComposeDraft {
            account_id,
            to: vec!["Bob <bob@example.net>".into()],
            cc: vec![],
            bcc: vec!["carol@example.com".into()],
            subject: "Plans\r\nX".into(),
            body_text: body.into(),
            in_reply_to: Some("p@example.net".into()),
            references: vec!["p@example.net".into()],
            attachments: Vec::new(),
            forward_attachments_of: None,
            rich: None,
        }
    }

    #[test]
    fn save_get_list_and_delete() {
        let (_dir, path, account) = setup();
        let id = save_draft(path.clone(), None, compose(account, "One")).expect("saved");
        assert_eq!(
            save_draft(path.clone(), Some(id), compose(account, "Two")),
            Ok(id)
        );
        let draft = get_draft(path.clone(), id).expect("read");
        assert_eq!(draft, compose(account, "Two"));

        let list = list_drafts(path.clone(), account).expect("list");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, id);
        assert_eq!(list[0].subject, "Plans  X", "one line");
        assert_eq!(list[0].to, "Bob <bob@example.net>");
        assert!(!list[0].on_server && !list[0].is_conflict);

        assert_eq!(delete_draft(path.clone(), id), Ok(()));
        assert_eq!(get_draft(path.clone(), id), Err(DraftFailure::NotFound));
        assert_eq!(delete_draft(path.clone(), id), Err(DraftFailure::NotFound));
        assert_eq!(
            save_draft(path.clone(), Some(id), compose(account, "x")),
            Err(DraftFailure::NotFound)
        );
        assert_eq!(open_draft(path, 999), Err(DraftFailure::NotFound));
    }
}
