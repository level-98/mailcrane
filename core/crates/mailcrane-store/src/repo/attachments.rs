use super::narrow;
use crate::{AttachmentId, AttachmentMeta, MessageId, NewAttachment, Store, StoreError};
use rusqlite::params;

impl Store {
    /// Inserts all attachments for a message, or none if any insert fails.
    pub fn insert_attachments(
        &self,
        message: MessageId,
        items: &[NewAttachment],
    ) -> Result<Vec<AttachmentId>, StoreError> {
        let tx = self.conn.unchecked_transaction()?;
        let mut ids = Vec::with_capacity(items.len());
        {
            let mut stmt = tx.prepare(
                "INSERT INTO attachments (message_id, part_id, filename, mime_type, size, \
                 content_id, is_inline) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            )?;
            for a in items {
                stmt.execute(params![
                    message.0,
                    a.part_id,
                    a.filename,
                    a.mime_type,
                    a.size,
                    a.content_id,
                    a.is_inline
                ])?;
                ids.push(AttachmentId(tx.last_insert_rowid()));
            }
        }
        tx.commit()?;
        Ok(ids)
    }

    /// Lists a message's attachments in insertion order.
    pub fn list_attachments(&self, message: MessageId) -> Result<Vec<AttachmentMeta>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, part_id, filename, mime_type, size, content_id, is_inline \
             FROM attachments WHERE message_id = ?1 ORDER BY id",
        )?;
        let mut rows = stmt.query(params![message.0])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            out.push(AttachmentMeta {
                id: AttachmentId(r.get(0)?),
                message_id: message,
                details: NewAttachment {
                    part_id: r.get(1)?,
                    filename: r.get(2)?,
                    mime_type: r.get(3)?,
                    size: r.get(4)?,
                    content_id: r.get(5)?,
                    is_inline: narrow::<u8>(r.get(6)?, "is_inline")? != 0,
                },
            });
        }
        Ok(out)
    }
}
