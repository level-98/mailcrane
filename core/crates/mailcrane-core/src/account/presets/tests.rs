use super::*;

fn provider(email: &str) -> Option<AppPasswordProvider> {
    preset_for(email).map(|p| p.provider)
}

#[test]
fn every_listed_domain_matches_its_provider() {
    for (list, want) in [
        (ICLOUD_DOMAINS, AppPasswordProvider::Icloud),
        (YAHOO_DOMAINS, AppPasswordProvider::Yahoo),
        (AOL_DOMAINS, AppPasswordProvider::Aol),
    ] {
        for domain in list {
            assert_eq!(provider(&format!("ann@{domain}")), Some(want), "{domain}");
            // Case and a trailing dot don't matter.
            assert_eq!(
                provider(&format!("Ann@{}.", domain.to_uppercase())),
                Some(want),
                "{domain}"
            );
        }
    }
}

#[test]
fn look_alikes_and_other_domains_do_not_match() {
    for email in [
        "ann@notyahoo.com",
        "ann@yahoo.com.example.net",
        "ann@mail.yahoo.com",
        "ann@icloud.com.example.org",
        "ann@myicloud.com",
        "ann@aol.com.evil.example",
        "ann@yahoo.co.jp",
        "ann@example.com",
        "ann@gmail.com",
        "@yahoo.com",
        "yahoo.com",
        "",
    ] {
        assert_eq!(provider(email), None, "{email}");
    }
}

#[test]
fn presets_have_the_documented_settings() {
    let icloud = preset_for("ann@icloud.com").expect("icloud");
    assert_eq!(icloud.imap, tls("imap.mail.me.com", 993));
    assert_eq!(icloud.smtp, starttls("smtp.mail.me.com", 587));
    let yahoo = preset_for("ann@yahoo.co.uk").expect("yahoo");
    assert_eq!(yahoo.imap, tls("imap.mail.yahoo.com", 993));
    assert_eq!(yahoo.smtp, tls("smtp.mail.yahoo.com", 465));
    let aol = preset_for("ann@aim.com").expect("aol");
    assert_eq!(aol.imap, tls("imap.aol.com", 993));
    assert_eq!(aol.smtp, tls("smtp.aol.com", 465));
    // The login is the full address.
    assert_eq!(icloud.username(" ann@me.com "), "ann@me.com");
}

#[test]
fn links_are_the_providers_own_https_pages() {
    for (email, host) in [
        ("ann@icloud.com", "account.apple.com"),
        ("ann@yahoo.com", "login.yahoo.com"),
        ("ann@aol.com", "login.aol.com"),
    ] {
        let url = preset_for(email).expect("preset").app_password_url;
        let rest = url.strip_prefix("https://").expect("https");
        assert_eq!(rest.split('/').next(), Some(host), "{url}");
    }
}
