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
}

#[derive(Debug)]
struct Folder {
    messages: Vec<Message>,
    next_uid: u32,
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
            let messages: Vec<Message> = raws
                .iter()
                .enumerate()
                .map(|(index, raw)| Message {
                    uid: index as u32 + 1,
                    raw: raw.clone(),
                    flags: Vec::new(),
                    corrupt_envelope: false,
                })
                .collect();
            let len = messages.len();
            map.insert(
                (*name).to_string(),
                Folder {
                    messages,
                    next_uid: u32::try_from(len).unwrap_or(u32::MAX).saturating_add(1),
                },
            );
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
            .or_insert_with(|| Folder {
                messages: Vec::new(),
                next_uid: 1,
            });
        let uid = target.next_uid;
        target.next_uid = target.next_uid.saturating_add(1);
        target.messages.push(Message {
            uid,
            raw,
            flags: Vec::new(),
            corrupt_envelope: false,
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

    /// Replaces the folder's messages and numbers them from 1 again.
    pub async fn replace_folder(&self, folder: &str, raws: Vec<Vec<u8>>) {
        let mut inner = self.inner.lock().await;
        let messages: Vec<Message> = raws
            .into_iter()
            .enumerate()
            .map(|(index, raw)| Message {
                uid: index as u32 + 1,
                raw,
                flags: Vec::new(),
                corrupt_envelope: false,
            })
            .collect();
        let next_uid = u32::try_from(messages.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        inner
            .folders
            .insert(folder.to_string(), Folder { messages, next_uid });
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

    /// Adds a flag to a message the way a `\Seen`-setting fetch would.
    pub async fn add_flag(&self, folder: &str, uid: u32, flag: &str) {
        let mut inner = self.inner.lock().await;
        if let Some(message) = inner
            .folders
            .get_mut(folder)
            .and_then(|f| f.messages.iter_mut().find(|m| m.uid == uid))
        {
            if !message.flags.iter().any(|f| f == flag) {
                message.flags.push(flag.to_string());
            }
        }
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
