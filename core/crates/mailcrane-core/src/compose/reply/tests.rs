use super::*;
use mailcrane_store::AccountId;

const OWN: &str = "me@example.org";
const HEADER: &str = "On 24 Nov 2026, at 10:42, Ana wrote:";

fn addr(name: Option<&str>, email: &str) -> Address {
    Address {
        name: name.map(str::to_owned),
        email: email.to_owned(),
    }
}

fn original() -> ParsedMessage {
    ParsedMessage {
        subject: Some("Lunch".into()),
        from: vec![addr(Some("Ana"), "ana@example.net")],
        to: vec![
            addr(Some("Me"), "ME@Example.org"),
            addr(None, "bob@example.com"),
        ],
        cc: vec![
            addr(Some("Carol, C."), "carol@example.com"),
            addr(None, "Bob@example.com"),
        ],
        date: Some(1_795_516_920),
        message_id: Some("m3@example.net".into()),
        text_plain: Some("Hi all,\n\nnoon?\n".into()),
        ..ParsedMessage::default()
    }
}

fn draft(o: &ParsedMessage, kind: ReplyKind) -> ComposeDraft {
    draft_for(o, kind, HEADER, OWN, AccountId(1), 0)
}

#[test]
fn recipients_follow_reply_to_from_and_reply_all_rules() {
    let o = original();
    let reply = draft(&o, ReplyKind::Reply);
    assert_eq!(reply.to, ["Ana <ana@example.net>"]);
    assert!(reply.cc.is_empty());

    let mut with_reply_to = o.clone();
    with_reply_to.reply_to = vec![addr(None, "list@example.net")];
    assert_eq!(
        draft(&with_reply_to, ReplyKind::Reply).to,
        ["list@example.net"]
    );

    // Reply All: the sender, the other To, then Cc; the account itself (in
    // any case) and duplicates (in any case) are left out.
    let all = draft(&o, ReplyKind::ReplyAll);
    assert_eq!(all.to, ["Ana <ana@example.net>", "bob@example.com"]);
    assert_eq!(all.cc, ["\"Carol, C.\" <carol@example.com>"]);

    let forward = draft(&o, ReplyKind::Forward);
    assert!(forward.to.is_empty() && forward.cc.is_empty() && forward.bcc.is_empty());
}

#[test]
fn formatted_entries_parse_back_to_the_same_addresses() {
    for a in [
        addr(Some("Carol, C."), "carol@example.com"),
        addr(Some("Say \"hi\""), "q@example.com"),
        addr(Some("José"), "jose@example.com"),
        addr(Some("  "), "blank@example.com"),
        addr(None, "bare@example.com"),
    ] {
        let entry = format_entry(&a);
        let parsed = mailcrane_mime::parse_address_list(&entry).expect(&entry);
        assert_eq!(parsed.len(), 1, "{entry}");
        assert_eq!(parsed[0].email, a.email, "{entry}");
    }
}

#[test]
fn subject_prefixes_are_never_stacked() {
    for (subject, kind, want) in [
        ("Lunch", ReplyKind::Reply, "Re: Lunch"),
        ("Re: Lunch", ReplyKind::Reply, "Re: Lunch"),
        ("RE: Lunch", ReplyKind::ReplyAll, "RE: Lunch"),
        ("re:Lunch", ReplyKind::Reply, "re:Lunch"),
        ("Lunch", ReplyKind::Forward, "Fwd: Lunch"),
        ("Fwd: Lunch", ReplyKind::Forward, "Fwd: Lunch"),
        ("Fw: Lunch", ReplyKind::Forward, "Fw: Lunch"),
        ("FWD: Lunch", ReplyKind::Forward, "FWD: Lunch"),
        ("Re: Lunch", ReplyKind::Forward, "Fwd: Re: Lunch"),
        ("", ReplyKind::Reply, "Re: "),
    ] {
        let mut o = original();
        o.subject = Some(subject.into());
        assert_eq!(draft(&o, kind).subject, want, "{subject:?} {kind:?}");
    }
}

#[test]
fn thread_headers_for_each_kind_of_original() {
    // With References.
    let mut o = original();
    o.references = vec!["m1@example.net".into(), "m2@example.net".into()];
    o.in_reply_to = vec!["m2@example.net".into()];
    let d = draft(&o, ReplyKind::Reply);
    assert_eq!(d.in_reply_to.as_deref(), Some("m3@example.net"));
    assert_eq!(
        d.references,
        ["m1@example.net", "m2@example.net", "m3@example.net"]
    );

    // Only In-Reply-To.
    let mut o = original();
    o.in_reply_to = vec!["m2@example.net".into()];
    assert_eq!(
        draft(&o, ReplyKind::ReplyAll).references,
        ["m2@example.net", "m3@example.net"]
    );

    // Neither.
    assert_eq!(
        draft(&original(), ReplyKind::Reply).references,
        ["m3@example.net"]
    );

    // No Message-ID at all.
    let mut o = original();
    o.message_id = None;
    let d = draft(&o, ReplyKind::Reply);
    assert_eq!(d.in_reply_to, None);
    assert!(d.references.is_empty());

    // 30 references plus the Message-ID: the first and the newest 19.
    let mut o = original();
    o.references = (1..=30).map(|n| format!("r{n}@example.net")).collect();
    let refs = draft(&o, ReplyKind::Reply).references;
    assert_eq!(refs.len(), 20);
    assert_eq!(refs[0], "r1@example.net");
    assert_eq!(refs[1], "r13@example.net");
    assert_eq!(refs[19], "m3@example.net");

    // A forward starts a new thread.
    let d = draft(&o, ReplyKind::Forward);
    assert_eq!(d.in_reply_to, None);
    assert!(d.references.is_empty());
}

#[test]
fn reply_quotes_line_by_line_under_the_given_header() {
    let d = draft(&original(), ReplyKind::Reply);
    assert_eq!(
        d.body_text,
        format!("\n\n{HEADER}\n> Hi all,\n>\n> noon?\n")
    );
}

#[test]
fn forward_includes_the_original_headers_and_text() {
    let d = draft_for(
        &original(),
        ReplyKind::Forward,
        "---------- Forwarded message ----------",
        OWN,
        AccountId(1),
        0,
    );
    assert!(d.body_text.starts_with("\n\n---------- Forwarded message ----------\n"));
    assert!(d.body_text.contains("From: Ana <ana@example.net>\n"));
    assert!(d.body_text.contains("Date: Tue, 24 Nov 2026 10:42:00 +0000\n"));
    assert!(d.body_text.contains("Subject: Lunch\n"));
    assert!(d
        .body_text
        .contains("To: Me <ME@Example.org>, bob@example.com\n"));
    assert!(d.body_text.contains("Cc: \"Carol, C.\" <carol@example.com>"));
    assert!(d.body_text.ends_with("\nHi all,\n\nnoon?\n"));
}
