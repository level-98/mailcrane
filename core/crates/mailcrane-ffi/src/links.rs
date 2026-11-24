//! Link cleaning for the apps.

/// Removes tracking parameters (`utm_*`, `fbclid` and others) from an `http`
/// or `https` link; anything else comes back unchanged. The apps call this
/// before opening a link when "Remove tracking from links" is on.
#[uniffi::export]
pub fn clean_url(input: String) -> String {
    mailcrane_core::links::clean_url(&input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_core() {
        assert_eq!(
            clean_url("https://example.com/?utm_source=x&a=1#f".into()),
            "https://example.com/?a=1#f"
        );
        assert_eq!(
            clean_url("mailto:a@example.com".into()),
            "mailto:a@example.com"
        );
    }
}
