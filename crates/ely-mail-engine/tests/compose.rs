use ely_mail_engine::{
    Mail, Mailbox,
    compose::{Earlier, Writing, answer, bodies, forward, is_address, original},
    read,
};

fn mail(headers: &str, body: &str) -> Mail {
    let raw = format!("{headers}\nContent-Type: text/plain\n\n{body}").replace('\n', "\r\n");
    read(raw.as_bytes()).unwrap()
}

fn emails(people: &[Mailbox]) -> Vec<&str> {
    people.iter().map(|person| person.email.as_str()).collect()
}

const ME: &str = "me@example.com";

fn group() -> Mail {
    mail(
        "From: Ana <ana@example.com>\nTo: me@example.com, Ben <ben@example.com>\nCc: ME@Example.com, cy@example.com, ben@example.com\nSubject: Plans\nMessage-ID: <c@example.com>\nIn-Reply-To: <b@example.com>\nReferences: <a@example.com> <b@example.com>",
        "Hi",
    )
}

#[test]
fn a_reply_goes_to_the_sender_and_follows_the_thread() {
    let start = answer(&group(), ME, false);
    assert_eq!(emails(&start.to), ["ana@example.com"]);
    assert!(start.cc.is_empty());
    assert_eq!(start.subject, "Re: Plans");
    assert_eq!(start.in_reply_to.as_deref(), Some("c@example.com"));
    assert_eq!(
        start.references,
        ["a@example.com", "b@example.com", "c@example.com"]
    );
}

#[test]
fn reply_all_reaches_everyone_but_me_once() {
    let start = answer(&group(), ME, true);
    assert_eq!(emails(&start.to), ["ana@example.com", "ben@example.com"]);
    assert_eq!(emails(&start.cc), ["cy@example.com"]);
}

#[test]
fn reply_to_wins_over_from() {
    let start = answer(
        &mail(
            "From: list-bot@example.com\nReply-To: list@example.com\nTo: me@example.com\nSubject: RE: Notes",
            "x",
        ),
        ME,
        false,
    );
    assert_eq!(emails(&start.to), ["list@example.com"]);
    assert_eq!(start.subject, "RE: Notes", "one Re is enough");
}

#[test]
fn answering_my_own_message_goes_to_its_recipients() {
    let sent = mail(
        "From: me@example.com\nTo: ana@example.com\nCc: ben@example.com\nSubject: Sent",
        "x",
    );
    assert_eq!(emails(&answer(&sent, ME, false).to), ["ana@example.com"]);
    let all = answer(&sent, ME, true);
    assert_eq!(emails(&all.to), ["ana@example.com"]);
    assert_eq!(emails(&all.cc), ["ben@example.com"]);
    let note = mail(
        "From: me@example.com\nTo: me@example.com\nSubject: Note",
        "x",
    );
    assert_eq!(
        emails(&answer(&note, ME, true).to),
        [ME],
        "a note to oneself"
    );
}

#[test]
fn a_parent_with_only_in_reply_to_starts_the_references() {
    let start = answer(
        &mail(
            "From: ana@example.com\nSubject: x\nMessage-ID: <c@example.com>\nIn-Reply-To: <b@example.com>",
            "x",
        ),
        ME,
        false,
    );
    assert_eq!(start.references, ["b@example.com", "c@example.com"]);
}

#[test]
fn a_forward_goes_to_nobody_under_fwd() {
    let start = forward(&group());
    assert!(start.to.is_empty() && start.cc.is_empty());
    assert_eq!(start.subject, "Fwd: Plans");
    assert_eq!(start.in_reply_to, None);
    let again = forward(&mail("From: a@example.com\nSubject: FW: Plans", "x"));
    assert_eq!(again.subject, "FW: Plans");
    let none = forward(&mail("From: a@example.com", "x"));
    assert_eq!(none.subject, "Fwd:");
}

#[test]
fn the_original_leaves_hidden_words_out_and_keeps_its_shape() {
    let raw = "From: a@example.com\r\nContent-Type: text/html\r\n\r\n<p>Lunch <b>at</b> one</p><span style=\"display:none\">tracker</span><ol start=\"3\"><li>bread</li><li>tea</li></ol><blockquote>old</blockquote>";
    let words = original(&read(raw.as_bytes()).unwrap());
    assert_eq!(words, "Lunch at one\n\n3. bread\n4. tea\n\n> old");
}

#[test]
fn html_and_plain_alternatives_are_not_reinterpreted_as_markdown() {
    let html = "<p><strong>Yes</strong> **literal**</p>";
    let made = bodies(&Writing {
        text: "Yes **literal**", html, signature: "Ana\nAtrium",
        earlier: Some(&Earlier::Quoted { lead: "Ben <b@x> wrote:".into(), text: "a < b".into() }),
    }).unwrap();
    assert_eq!(made.text, "Yes **literal**\n\n-- \nAna\nAtrium\n\nBen <b@x> wrote:\n> a < b");
    let rendered = made.html.unwrap();
    assert!(rendered.starts_with(html));
    assert!(rendered.contains("Ben &lt;b@x&gt; wrote:"));
    assert!(rendered.contains("a &lt; b</blockquote>"));
}

#[test]
fn legacy_markdown_import_is_narrow_and_plain_import_remains_literal() {
    use ely_mail_engine::compose::legacy_draft_html;
    assert!(legacy_draft_html("**Yes**", true).unwrap().contains("<strong>Yes</strong>"));
    let literal = legacy_draft_html("**Yes**\n<&", false).unwrap();
    assert!(literal.contains("**Yes**<br>&lt;&amp;"));
    assert!(!literal.contains("<strong>"));
}

#[test]
fn a_forward_carries_its_header_lines_unquoted() {
    let made = bodies(&Writing {
        text: "", html: "<p></p>", signature: "Ben",
        earlier: Some(&Earlier::Forwarded { lead: vec!["From: Ana".into()], text: "Hi\nthere".into() }),
    }).unwrap();
    assert_eq!(made.text, "-- \nBen\n\nFrom: Ana\n\nHi\nthere");
    assert!(made.html.unwrap().contains("<p>Hi<br>\nthere</p>"));
}

#[test]
fn explicit_plain_alternative_preserves_trailing_spaces_and_empty_lines() {
    for text in ["body  ", "body\n\n", "  \n\t"] {
        let made = bodies(&Writing { text, html: "<p>body</p>", signature: "", earlier: None }).unwrap();
        assert_eq!(made.text, text);
        assert_eq!(made.html.as_deref(), Some("<p>body</p>"));
        let signed = bodies(&Writing { text, html: "<p>body</p>", signature: "Ana", earlier: None }).unwrap();
        assert_eq!(signed.text, format!("{text}\n\n-- \nAna"));
    }
}

#[test]
fn a_file_is_typed_by_its_name() {
    use ely_mail_engine::compose::attachment;
    let pdf = attachment("plan.PDF", b"%PDF".to_vec());
    assert_eq!(pdf.content_type, "application/pdf");
    assert_eq!(pdf.name.as_deref(), Some("plan.PDF"));
    assert_eq!(pdf.bytes, b"%PDF");
    let odd = attachment("notes.zzzz", vec![1]);
    assert_eq!(odd.content_type, "application/octet-stream");
}

#[test]
fn an_address_is_what_the_server_is_sent() {
    for good in [
        "\"first last\"@example.com",
        "ann@example.com",
        "ann@localhost",
        "ann@[10.0.0.1]",
    ] {
        assert!(is_address(good), "{good}");
    }
    for bad in ["ann@", "@example.com", "ann", "a b@example.com", "ann@.com"] {
        assert!(!is_address(bad), "{bad}");
    }
}

#[test]
fn a_bare_quoted_local_part_is_an_address() {
    let start = answer(
        &mail(
            "From: \"first last\"@example.com\nReply-To: Ann <\"a\\\"b\"@example.com>, \"c d\" @example.com\nTo: me@example.com\nSubject: x",
            "x",
        ),
        ME,
        false,
    );
    assert_eq!(
        emails(&start.to),
        ["\"a\\\"b\"@example.com", "\"c d\"@example.com"]
    );
    assert!(start.to.iter().all(|to| is_address(&to.email)));
    let from = mail("From: \"first last\"@example.com\nSubject: x", "x").from;
    assert_eq!(emails(&from), ["\"first last\"@example.com"]);
    assert_eq!(from[0].name, None);
    let named = mail("From: \"Ann Lee\" <ann@example.com>\nSubject: x", "x").from;
    assert_eq!(
        (named[0].name.as_deref(), emails(&named)),
        (Some("Ann Lee"), vec!["ann@example.com"])
    );
}
