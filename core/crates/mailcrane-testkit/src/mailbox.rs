use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, Notify};

#[derive(Debug, Clone)]
pub struct Message {
    pub uid: u32,
    pub raw: Vec<u8>,
}

#[derive(Debug)]
struct Folder {
    messages: Vec<Message>,
    next_uid: u32,
}

#[derive(Debug, Clone)]
pub struct Mailbox {
    inner: Arc<Mutex<MailboxInner>>,
    pub uidvalidity: u32,
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
            uidvalidity: 1,
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
        target.messages.push(Message { uid, raw });
        drop(inner);
        self.notify.notify_waiters();
    }

    pub async fn messages(&self, folder: &str) -> Vec<Message> {
        let inner = self.inner.lock().await;
        inner
            .folders
            .get(folder)
            .map(|folder| folder.messages.clone())
            .unwrap_or_default()
    }

    pub async fn exists(&self, folder: &str) -> usize {
        self.messages(folder).await.len()
    }
}
