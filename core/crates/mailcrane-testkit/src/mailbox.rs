use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

#[derive(Debug, Clone)]
pub struct Message {
    pub uid: u32,
    pub raw: Vec<u8>,
    /// IMAP flags, such as `\\Seen`.
    pub flags: Vec<String>,
    /// When set, the server sends an `ENVELOPE` that is not valid IMAP.
    pub corrupt_envelope: bool,
    /// The message's mod-sequence (RFC 7162 §3.1): raised whenever it is
    /// delivered or its flags change.
    pub modseq: u64,
}

#[derive(Debug)]
struct Folder {
    messages: Vec<Message>,
    next_uid: u32,
    /// `HIGHESTMODSEQ`: the largest mod-sequence handed out in the folder.
    highest_modseq: u64,
    /// Expunged UIDs with the mod-sequence of the expunge, for `VANISHED`.
    vanished: Vec<(u32, u64)>,
    /// When set, `SELECT` answers `NOMODSEQ` (RFC 7162 §3.1.2.2).
    nomodseq: bool,
}

impl Folder {
    fn new(raws: Vec<Vec<u8>>) -> Self {
        let messages: Vec<Message> = raws
            .into_iter()
            .enumerate()
            .map(|(index, raw)| {
                let n = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
                Message {
                    uid: n,
                    raw,
                    flags: Vec::new(),
                    corrupt_envelope: false,
                    modseq: u64::from(n),
                }
            })
            .collect();
        let len = u32::try_from(messages.len()).unwrap_or(u32::MAX);
        Folder {
            messages,
            next_uid: len.saturating_add(1),
            highest_modseq: u64::from(len).max(1),
            vanished: Vec::new(),
            nomodseq: false,
        }
    }

    fn bump(&mut self) -> u64 {
        self.highest_modseq = self.highest_modseq.saturating_add(1);
        self.highest_modseq
    }
}

/// What changed in a folder since a mod-sequence (RFC 7162 §3.2.5).
#[derive(Debug, Clone, Default)]
pub struct Changes {
    /// Messages whose mod-sequence is higher, with their sequence numbers.
    pub changed: Vec<(usize, Message)>,
    /// UIDs expunged since then.
    pub vanished: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Mailbox {
    inner: Arc<Mutex<MailboxInner>>,
    uidvalidity: Arc<AtomicU32>,
    pub notify: Arc<Notify>,
}

#[derive(Debug)]
struct MailboxInner {
    folders: HashMap<String, Folder>,
}

impl Mailbox {
    pub fn new(folders: &[(&str, Vec<Vec<u8>>)]) -> Self {
        let mut map = HashMap::new();
        for (name, raws) in folders {
            map.insert((*name).to_string(), Folder::new(raws.clone()));
        }
        Self {
            inner: Arc::new(Mutex::new(MailboxInner { folders: map })),
            uidvalidity: Arc::new(AtomicU32::new(1)),
            notify: Arc::new(Notify::new()),
        }
    }

    pub async fn deliver(&self, folder: &str, raw: Vec<u8>) {
        let mut inner = self.inner.lock().await;
        let target = inner
            .folders
            .entry(folder.to_string())
            .or_insert_with(|| Folder::new(Vec::new()));
        let uid = target.next_uid;
        target.next_uid = target.next_uid.saturating_add(1);
        let modseq = target.bump();
        target.messages.push(Message {
            uid,
            raw,
            flags: Vec::new(),
            corrupt_envelope: false,
            modseq,
        });
        drop(inner);
        self.notify.notify_waiters();
    }

    /// The folder's `UIDVALIDITY` (the same for every folder).
    pub fn uidvalidity(&self) -> u32 {
        self.uidvalidity.load(Ordering::SeqCst)
    }

    /// Changes `UIDVALIDITY`, as a server does when it renumbers UIDs.
    pub fn set_uidvalidity(&self, value: u32) {
        self.uidvalidity.store(value, Ordering::SeqCst);
    }

    /// The UID the next delivered message gets, or 1 for an unknown folder.
    pub async fn uidnext(&self, folder: &str) -> u32 {
        let inner = self.inner.lock().await;
        inner.folders.get(folder).map_or(1, |f| f.next_uid)
    }

    /// The folder's `HIGHESTMODSEQ`, or `None` if it answers `NOMODSEQ` or
    /// does not exist.
    pub async fn highest_modseq(&self, folder: &str) -> Option<u64> {
        let inner = self.inner.lock().await;
        inner
            .folders
            .get(folder)
            .filter(|f| !f.nomodseq)
            .map(|f| f.highest_modseq)
    }

    /// Replaces the folder's messages and numbers them from 1 again.
    pub async fn replace_folder(&self, folder: &str, raws: Vec<Vec<u8>>) {
        let mut inner = self.inner.lock().await;
        inner.folders.insert(folder.to_string(), Folder::new(raws));
    }

    /// Makes the server send an invalid `ENVELOPE` for this message.
    pub async fn corrupt_envelope(&self, folder: &str, uid: u32) {
        let mut inner = self.inner.lock().await;
        if let Some(message) = inner
            .folders
            .get_mut(folder)
            .and_then(|f| f.messages.iter_mut().find(|m| m.uid == uid))
        {
            message.corrupt_envelope = true;
        }
    }

    /// The flags of a message, or `None` if it does not exist.
    pub async fn flags(&self, folder: &str, uid: u32) -> Option<Vec<String>> {
        let inner = self.inner.lock().await;
        inner
            .folders
            .get(folder)?
            .messages
            .iter()
            .find(|m| m.uid == uid)
            .map(|m| m.flags.clone())
    }

    /// Adds a flag to a message, as another client or a `\Seen`-setting
    /// fetch would. Raises its mod-sequence if anything changed.
    pub async fn add_flag(&self, folder: &str, uid: u32, flag: &str) {
        self.change_flags(folder, uid, |flags| {
            if flags.iter().any(|f| f == flag) {
                false
            } else {
                flags.push(flag.to_string());
                true
            }
        })
        .await;
    }

    /// Removes a flag from a message, as another client would. Raises its
    /// mod-sequence if anything changed.
    pub async fn remove_flag(&self, folder: &str, uid: u32, flag: &str) {
        self.change_flags(folder, uid, |flags| {
            let before = flags.len();
            flags.retain(|f| f != flag);
            flags.len() != before
        })
        .await;
    }

    async fn change_flags(
        &self,
        folder: &str,
        uid: u32,
        change: impl FnOnce(&mut Vec<String>) -> bool,
    ) {
        let mut inner = self.inner.lock().await;
        let Some(target) = inner.folders.get_mut(folder) else {
            return;
        };
        let next = target.highest_modseq.saturating_add(1);
        let Some(message) = target.messages.iter_mut().find(|m| m.uid == uid) else {
            return;
        };
        if change(&mut message.flags) {
            message.modseq = next;
            target.highest_modseq = next;
            drop(inner);
            self.notify.notify_waiters();
        }
    }

    /// Expunges a message, as another client would, remembering it for
    /// `VANISHED`.
    pub async fn expunge(&self, folder: &str, uid: u32) {
        let mut inner = self.inner.lock().await;
        let Some(target) = inner.folders.get_mut(folder) else {
            return;
        };
        if target.messages.iter().any(|m| m.uid == uid) {
            target.messages.retain(|m| m.uid != uid);
            let modseq = target.bump();
            target.vanished.push((uid, modseq));
            drop(inner);
            self.notify.notify_waiters();
        }
    }

    /// Makes `SELECT` answer `NOMODSEQ` for the folder (or stop doing so).
    pub async fn set_nomodseq(&self, folder: &str, nomodseq: bool) {
        let mut inner = self.inner.lock().await;
        if let Some(target) = inner.folders.get_mut(folder) {
            target.nomodseq = nomodseq;
        }
    }

    /// Forgets every mod-sequence, as a server that lost its database
    /// would: `HIGHESTMODSEQ` goes back to 1 and the expunge history is
    /// gone. UIDs and flags are kept.
    pub async fn reset_modseq(&self, folder: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(target) = inner.folders.get_mut(folder) {
            for message in &mut target.messages {
                message.modseq = 1;
            }
            target.highest_modseq = 1;
            target.vanished.clear();
        }
    }

    /// What changed since `modseq`, limited to UIDs `known` accepts.
    pub async fn changes_since(
        &self,
        folder: &str,
        modseq: u64,
        known: impl Fn(u32) -> bool,
    ) -> Changes {
        let inner = self.inner.lock().await;
        let Some(target) = inner.folders.get(folder) else {
            return Changes::default();
        };
        Changes {
            changed: target
                .messages
                .iter()
                .enumerate()
                .filter(|(_, m)| m.modseq > modseq && known(m.uid))
                .map(|(i, m)| (i + 1, m.clone()))
                .collect(),
            vanished: target
                .vanished
                .iter()
                .filter(|(uid, m)| *m > modseq && known(*uid))
                .map(|(uid, _)| *uid)
                .collect(),
        }
    }

    /// Adds (`add`) or removes `flag` on every message of `folder` whose UID
    /// `selected` accepts, raising mod-sequences of messages that changed.
    /// Returns false if the folder doesn't exist.
    pub async fn store_flag(
        &self,
        folder: &str,
        selected: impl Fn(u32) -> bool,
        flag: &str,
        add: bool,
    ) -> bool {
        let mut inner = self.inner.lock().await;
        let Some(target) = inner.folders.get_mut(folder) else {
            return false;
        };
        let mut highest = target.highest_modseq;
        for message in target.messages.iter_mut().filter(|m| selected(m.uid)) {
            let has = message.flags.iter().any(|f| f.eq_ignore_ascii_case(flag));
            if add && !has {
                message.flags.push(flag.to_string());
            } else if !add && has {
                message.flags.retain(|f| !f.eq_ignore_ascii_case(flag));
            } else {
                continue;
            }
            highest = highest.saturating_add(1);
            message.modseq = highest;
        }
        target.highest_modseq = highest;
        true
    }

    /// Copies the messages of `from` whose UID `selected` accepts to `to`,
    /// with new UIDs and the same flags, as `COPY` does. Returns false (and
    /// copies nothing) if either folder doesn't exist.
    pub async fn copy(&self, from: &str, selected: impl Fn(u32) -> bool, to: &str) -> bool {
        let mut inner = self.inner.lock().await;
        if !inner.folders.contains_key(to) {
            return false;
        }
        let Some(source) = inner.folders.get(from) else {
            return false;
        };
        let picked: Vec<Message> = source
            .messages
            .iter()
            .filter(|m| selected(m.uid))
            .cloned()
            .collect();
        let Some(target) = inner.folders.get_mut(to) else {
            return false;
        };
        for mut message in picked {
            message.uid = target.next_uid;
            target.next_uid = target.next_uid.saturating_add(1);
            message.modseq = target.bump();
            target.messages.push(message);
        }
        drop(inner);
        self.notify.notify_waiters();
        true
    }

    /// Removes the messages of `folder` that have `\Deleted` and whose UID
    /// `selected` accepts, remembering them for `VANISHED`. Returns the
    /// removed UIDs.
    pub async fn expunge_deleted(&self, folder: &str, selected: impl Fn(u32) -> bool) -> Vec<u32> {
        let mut inner = self.inner.lock().await;
        let Some(target) = inner.folders.get_mut(folder) else {
            return Vec::new();
        };
        let gone: Vec<u32> = target
            .messages
            .iter()
            .filter(|m| {
                selected(m.uid) && m.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Deleted"))
            })
            .map(|m| m.uid)
            .collect();
        target.messages.retain(|m| !gone.contains(&m.uid));
        for uid in &gone {
            let modseq = target.bump();
            target.vanished.push((*uid, modseq));
        }
        gone
    }

    /// Moves messages from `from` to `to`, as `MOVE` does: a copy, then an
    /// expunge of the originals. Returns false if either folder doesn't exist.
    pub async fn move_messages(
        &self,
        from: &str,
        selected: impl Fn(u32) -> bool + Copy,
        to: &str,
    ) -> bool {
        if !self.copy(from, selected, to).await {
            return false;
        }
        let mut inner = self.inner.lock().await;
        if let Some(source) = inner.folders.get_mut(from) {
            let gone: Vec<u32> = source
                .messages
                .iter()
                .filter(|m| selected(m.uid))
                .map(|m| m.uid)
                .collect();
            source.messages.retain(|m| !gone.contains(&m.uid));
            for uid in gone {
                let modseq = source.bump();
                source.vanished.push((uid, modseq));
            }
        }
        true
    }

    /// Appends a message with `flags` to an existing folder, as `APPEND`
    /// does. Returns its UID, or `None` if the folder doesn't exist.
    pub async fn append(&self, folder: &str, raw: Vec<u8>, flags: Vec<String>) -> Option<u32> {
        let mut inner = self.inner.lock().await;
        let target = inner.folders.get_mut(folder)?;
        let uid = target.next_uid;
        target.next_uid = target.next_uid.saturating_add(1);
        let modseq = target.bump();
        target.messages.push(Message {
            uid,
            raw,
            flags,
            corrupt_envelope: false,
            modseq,
        });
        drop(inner);
        self.notify.notify_waiters();
        Some(uid)
    }

    /// Creates an empty folder if it doesn't exist.
    pub async fn create_folder(&self, folder: &str) {
        let mut inner = self.inner.lock().await;
        inner
            .folders
            .entry(folder.to_string())
            .or_insert_with(|| Folder::new(Vec::new()));
    }

    /// Renames a folder and every folder under it (`old/…`), keeping their
    /// messages (RFC 9051 §6.3.6). Returns whether `old` existed.
    pub async fn rename_folder(&self, old: &str, new: &str) -> bool {
        let mut inner = self.inner.lock().await;
        if !inner.folders.contains_key(old) {
            return false;
        }
        let prefix = format!("{old}/");
        let names: Vec<String> = inner
            .folders
            .keys()
            .filter(|n| *n == old || n.starts_with(&prefix))
            .cloned()
            .collect();
        for name in names {
            if let Some(folder) = inner.folders.remove(&name) {
                let renamed = format!("{new}{}", &name[old.len()..]);
                inner.folders.insert(renamed, folder);
            }
        }
        true
    }

    /// Deletes a folder and its messages, as another client would.
    pub async fn delete_folder(&self, folder: &str) {
        let mut inner = self.inner.lock().await;
        inner.folders.remove(folder);
    }

    pub async fn messages(&self, folder: &str) -> Vec<Message> {
        let inner = self.inner.lock().await;
        inner
            .folders
            .get(folder)
            .map(|folder| folder.messages.clone())
            .unwrap_or_default()
    }

    /// Names of all folders, sorted.
    pub async fn folder_names(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        let mut names: Vec<String> = inner.folders.keys().cloned().collect();
        names.sort();
        names
    }

    pub async fn exists(&self, folder: &str) -> usize {
        self.messages(folder).await.len()
    }
}
