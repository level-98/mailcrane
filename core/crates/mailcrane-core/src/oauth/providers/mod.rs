//! The OAuth providers Mailcrane signs in with, by the id stored with an
//! account (`accounts.oauth_provider`).

pub mod google;

use super::OAuthProvider;

/// The provider with this stored id, if Mailcrane knows it.
pub fn by_id(id: &str) -> Option<OAuthProvider> {
    match id {
        google::ID => google::provider(),
        _ => None,
    }
}
