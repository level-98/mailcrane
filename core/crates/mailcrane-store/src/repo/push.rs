//! JMAP push subscriptions (M4-27): what this install registered with each
//! JMAP account's server. See `migrations/0021_jmap_push.sql`.

use crate::{AccountId, Store, StoreError};
use rusqlite::{params, OptionalExtension};

/// One account's push subscription, as registered with its server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushSubscriptionRow {
    pub account: AccountId,
    /// The server's id for it (RFC 8620 §7.2).
    pub subscription_id: String,
    /// This install's `deviceClientId` for the account.
    pub device_client_id: String,
    /// The Web Push URL it delivers to.
    pub url: String,
    /// The public key sent with it, if any (the auth secret is not stored).
    pub p256dh: Option<String>,
    /// When the server drops it, unix seconds, if known.
    pub expires: Option<i64>,
    /// Whether the verification push was answered.
    pub verified: bool,
    /// When it was created, unix seconds.
    pub created_at: i64,
}

impl Store {
    /// Records `row`, replacing any earlier subscription of its account.
    pub fn save_push_subscription(&self, row: &PushSubscriptionRow) -> Result<(), StoreError> {
        self.conn.execute(
            "INSERT INTO jmap_push_subscriptions \
             (account_id, subscription_id, device_client_id, url, p256dh, expires, verified, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT (account_id) DO UPDATE SET subscription_id = excluded.subscription_id, \
             device_client_id = excluded.device_client_id, url = excluded.url, \
             p256dh = excluded.p256dh, expires = excluded.expires, \
             verified = excluded.verified, created_at = excluded.created_at",
            params![
                row.account.0,
                row.subscription_id,
                row.device_client_id,
                row.url,
                row.p256dh,
                row.expires,
                row.verified,
                row.created_at
            ],
        )?;
        Ok(())
    }

    /// The account's subscription, if it has one.
    pub fn push_subscription(
        &self,
        account: AccountId,
    ) -> Result<Option<PushSubscriptionRow>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT account_id, subscription_id, device_client_id, url, p256dh, expires, \
                 verified, created_at FROM jmap_push_subscriptions WHERE account_id = ?1",
                params![account.0],
                row,
            )
            .optional()?)
    }

    /// Every account's subscription.
    pub fn push_subscriptions(&self) -> Result<Vec<PushSubscriptionRow>, StoreError> {
        let mut stmt = self.conn.prepare(
            "SELECT account_id, subscription_id, device_client_id, url, p256dh, expires, \
             verified, created_at FROM jmap_push_subscriptions ORDER BY account_id",
        )?;
        let rows = stmt.query_map([], row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Marks the subscription with server id `subscription_id` verified.
    /// Returns its account, or `None` if no account has that subscription.
    pub fn mark_push_verified(
        &self,
        subscription_id: &str,
    ) -> Result<Option<AccountId>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "UPDATE jmap_push_subscriptions SET verified = 1 WHERE subscription_id = ?1 \
                 RETURNING account_id",
                params![subscription_id],
                |r| Ok(AccountId(r.get(0)?)),
            )
            .optional()?)
    }

    /// The account whose subscription has server id `subscription_id`.
    pub fn push_subscription_account(
        &self,
        subscription_id: &str,
    ) -> Result<Option<AccountId>, StoreError> {
        Ok(self
            .conn
            .query_row(
                "SELECT account_id FROM jmap_push_subscriptions WHERE subscription_id = ?1",
                params![subscription_id],
                |r| Ok(AccountId(r.get(0)?)),
            )
            .optional()?)
    }

    /// Sets when the account's subscription expires.
    pub fn set_push_expires(&self, account: AccountId, expires: i64) -> Result<(), StoreError> {
        self.conn.execute(
            "UPDATE jmap_push_subscriptions SET expires = ?2 WHERE account_id = ?1",
            params![account.0, expires],
        )?;
        Ok(())
    }

    /// Forgets the account's subscription.
    pub fn delete_push_subscription(&self, account: AccountId) -> Result<(), StoreError> {
        self.conn.execute(
            "DELETE FROM jmap_push_subscriptions WHERE account_id = ?1",
            params![account.0],
        )?;
        Ok(())
    }
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<PushSubscriptionRow> {
    Ok(PushSubscriptionRow {
        account: AccountId(r.get(0)?),
        subscription_id: r.get(1)?,
        device_client_id: r.get(2)?,
        url: r.get(3)?,
        p256dh: r.get(4)?,
        expires: r.get(5)?,
        verified: r.get(6)?,
        created_at: r.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewAccount, Security};

    fn account(store: &Store) -> AccountId {
        store
            .insert_jmap_account(
                &NewAccount {
                    display_name: "A".into(),
                    email: "a@example.org".into(),
                    imap_host: String::new(),
                    imap_port: 0,
                    imap_security: Security::Tls,
                    smtp_host: String::new(),
                    smtp_port: 0,
                    smtp_security: Security::Tls,
                    username: "a".into(),
                },
                "https://jmap.example.org/session",
            )
            .expect("account")
    }

    #[test]
    fn a_subscription_is_saved_verified_refreshed_and_removed_with_its_account() {
        let store = Store::open_in_memory().expect("store");
        let a = account(&store);
        assert_eq!(store.push_subscription(a).expect("read"), None);
        let mut sub = PushSubscriptionRow {
            account: a,
            subscription_id: "P1".into(),
            device_client_id: "d".into(),
            url: "https://push.example.org/up/1".into(),
            p256dh: Some("key".into()),
            expires: Some(100),
            verified: false,
            created_at: 10,
        };
        store.save_push_subscription(&sub).expect("save");
        assert_eq!(store.push_subscription(a).expect("read"), Some(sub.clone()));
        assert_eq!(store.mark_push_verified("P1").expect("verify"), Some(a));
        assert_eq!(store.mark_push_verified("P9").expect("verify"), None);
        store.set_push_expires(a, 200).expect("expires");
        sub.verified = true;
        sub.expires = Some(200);
        assert_eq!(store.push_subscriptions().expect("list"), vec![sub.clone()]);

        // A new endpoint replaces the row.
        sub.subscription_id = "P2".into();
        sub.verified = false;
        store.save_push_subscription(&sub).expect("replace");
        assert_eq!(store.push_subscription_account("P1").expect("read"), None);
        assert_eq!(
            store.push_subscription_account("P2").expect("read"),
            Some(a)
        );

        store.delete_account(a).expect("delete account");
        assert!(store.push_subscriptions().expect("list").is_empty());
    }
}
