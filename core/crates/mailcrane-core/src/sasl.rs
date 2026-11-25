//! SASL strings for logging in with an OAuth access token: Google's
//! XOAUTH2 and the standard OAUTHBEARER (RFC 7628). IMAP sends them base64
//! encoded in `AUTHENTICATE` (RFC 9051 §6.2.2), SMTP in `AUTH` (RFC 4954).
//!
//! The results contain the token: never log them.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

/// XOAUTH2 initial response, before base64:
/// `user=<user>^Aauth=Bearer <token>^A^A`.
pub fn xoauth2(user: &str, token: &str) -> String {
    format!("user={user}\x01auth=Bearer {token}\x01\x01")
}

/// [`xoauth2`], base64 encoded as sent on the wire.
pub fn xoauth2_base64(user: &str, token: &str) -> String {
    STANDARD.encode(xoauth2(user, token))
}

/// OAUTHBEARER initial client response (RFC 7628 §3.1), before base64:
/// a GS2 header with the authorization identity, then `host`, `port` and
/// `auth` key-value pairs separated by `^A`. Commas and `=` in the user
/// are escaped as `=2C` and `=3D` (RFC 5801 §4).
pub fn oauthbearer(user: &str, host: &str, port: u16, token: &str) -> String {
    let user = user.replace('=', "=3D").replace(',', "=2C");
    format!("n,a={user},\x01host={host}\x01port={port}\x01auth=Bearer {token}\x01\x01")
}

/// [`oauthbearer`], base64 encoded as sent on the wire.
pub fn oauthbearer_base64(user: &str, host: &str, port: u16, token: &str) -> String {
    STANDARD.encode(oauthbearer(user, host, port, token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xoauth2_matches_googles_example() {
        // https://developers.google.com/gmail/imap/xoauth2-protocol
        let token = "ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg";
        assert_eq!(
            xoauth2("someuser@example.com", token),
            "user=someuser@example.com\x01auth=Bearer ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg\x01\x01"
        );
        assert_eq!(
            xoauth2_base64("someuser@example.com", token),
            "dXNlcj1zb21ldXNlckBleGFtcGxlLmNvbQFhdXRoPUJlYXJlciB5YTI5LnZGOWRmdDRxbVRjMk52YjNSbGNrQmhkSFJoZG1semRHRXVZMjl0Q2cBAQ=="
        );
    }

    #[test]
    fn oauthbearer_matches_rfc_7628() {
        // RFC 7628 §4.1, the IMAP example.
        assert_eq!(
            oauthbearer(
                "user@example.com",
                "server.example.com",
                143,
                "vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg=="
            ),
            "n,a=user@example.com,\x01host=server.example.com\x01port=143\x01auth=Bearer vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg==\x01\x01"
        );
    }

    #[test]
    fn gs2_special_characters_are_escaped() {
        assert!(
            oauthbearer("a,b=c@example.org", "h", 1, "t").starts_with("n,a=a=2Cb=3Dc@example.org,")
        );
    }
}
