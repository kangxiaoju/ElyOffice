use ely_mail_engine::{
    Mail, Mailbox,
    compose::{Bodies, Earlier, Writing, answer, bodies, forward, is_address, original},
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
fn plain_text_sends_no_html() {
    let made = bodies(&Writing {
        words: "Yes.\n**not bold**\n\n",
        markdown: false,
        signature: "Ana\nAtrium",
        earlier: Some(&Earlier::Quoted {
            lead: "Ben wrote:".into(),
            text: "Lunch?\n\nAt one.".into(),
        }),
    })
    .unwrap();
    assert_eq!(
        made,
        Bodies {
            text: "Yes.\n**not bold**\n\n-- \nAna\nAtrium\n\nBen wrote:\n> Lunch?\n>\n> At one."
                .into(),
            html: None,
        }
    );
}

#[test]
fn rich_text_sends_html_and_readable_plain_text() {
    let made = bodies(&Writing {
        words: "**Yes** <script>x()</script> [go](javascript:x)",
        markdown: true,
        signature: "",
        earlier: Some(&Earlier::Quoted {
            lead: "Ben <b@x> wrote:".into(),
            text: "a < b".into(),
        }),
    })
    .unwrap();
    assert!(
        made.text.starts_with("Yes <script>x()</script> go"),
        "{}",
        made.text
    );
    let html = made.html.unwrap();
    assert!(html.contains("<strong>Yes</strong>"), "{html}");
    assert!(html.contains("&lt;script&gt;"), "raw HTML escaped: {html}");
    assert!(!html.contains("javascript:"), "{html}");
    assert!(html.contains("<p>Ben &lt;b@x&gt; wrote:</p>"), "{html}");
    assert!(html.contains("a &lt; b</blockquote>"), "{html}");
}

#[test]
fn a_forward_carries_its_header_lines_unquoted() {
    let lead = [
        "---------- Forwarded ----------".to_owned(),
        "From: Ana".to_owned(),
    ];
    let made = bodies(&Writing {
        words: "",
        markdown: true,
        signature: "Ben",
        earlier: Some(&Earlier::Forwarded {
            lead: lead.to_vec(),
            text: "Hi\nthere".into(),
        }),
    })
    .unwrap();
    assert_eq!(
        made.text,
        "-- \nBen\n\n---------- Forwarded ----------\nFrom: Ana\n\nHi\nthere"
    );
    let html = made.html.unwrap();
    assert!(html.contains("<p>-- <br>\nBen</p>"), "{html}");
    assert!(html.contains("<p>Hi<br>\nthere</p>"), "{html}");
}

#[test]
fn literal_text_import_preserves_markup_and_line_breaks() {
    let plain = "*literal*\n# title\n1. item\nC:\\notes\\file\n\nline < & >";
    let words = ely_mail_engine::compose::rich_text(plain);
    let made = bodies(&Writing {
        words: &words,
        markdown: true,
        signature: "",
        earlier: None,
    })
    .unwrap();
    assert_eq!(made.text, plain);
    let html = made.html.unwrap();
    assert!(!html.contains("<em>"), "{html}");
    assert!(!html.contains("<h1>"), "{html}");
    assert!(!html.contains("<ol>"), "{html}");
}

#[test]
fn formatted_body_has_no_markup_in_plain_alternative() {
    let made = bodies(&Writing {
        words: "**Hello** [team](https://example.com)\n\n- *one*\n- two",
        markdown: true,
        signature: "Ana",
        earlier: None,
    })
    .unwrap();
    assert_eq!(made.text, "Hello team\n\n- one\n- two\n\n-- \nAna");
    let html = made.html.unwrap();
    assert!(html.contains("<strong>Hello</strong>"), "{html}");
    assert!(html.contains("href=\"https://example.com\""), "{html}");
}

#[test]
fn literal_import_keeps_indentation_spacing_and_empty_lines() {
    let plain = "    *literal*\n\t# tab\none  two\n\n\nthree";
    let words = ely_mail_engine::compose::rich_text(plain);
    let made = bodies(&Writing {
        words: &words,
        markdown: true,
        signature: "",
        earlier: None,
    })
    .unwrap();
    assert_eq!(made.text, plain);
    let html = made.html.unwrap();
    assert!(!html.contains("<pre>"), "{html}");
    assert!(!html.contains("\\*"), "{html}");
}

#[test]
fn literal_import_keeps_line_end_whitespace_and_boundary_lines() {
    for plain in [
        "a \nb",
        "a  \nb",
        "a\t\nb",
        "a \t \nb",
        "\n\n  a  \n\t b\t\n \t\n",
        "  \n\t\n",
        "a  ",
        "    a \n    b  \n\t c\t\n",
        "a\\\nb",
    ] {
        let words = ely_mail_engine::compose::rich_text(plain);
        let made = bodies(&Writing {
            words: &words,
            markdown: true,
            signature: "",
            earlier: None,
        })
        .unwrap();
        assert_eq!(made.text, plain, "source: {plain:?}");
        let html = made.html.unwrap();
        let visible = html
            .strip_prefix("<div style=\"white-space:pre-wrap\">")
            .unwrap()
            .strip_suffix("</div>")
            .unwrap()
            .replace("<br>", "\n");
        assert_eq!(visible, plain, "HTML source: {plain:?}");
    }
}

#[test]
fn markdown_hardbreak_syntax_is_not_literal_whitespace() {
    let made = bodies(&Writing {
        words: "a\\\n  b",
        markdown: true,
        signature: "",
        earlier: None,
    })
    .unwrap();
    assert_eq!(made.text, "a\n  b");
    assert!(!made.html.unwrap().contains('\\'));
}

#[test]
fn rich_draft_references_resolve_normalized_definitions_once() {
    let made = bodies(&Writing {
        words: "[**go**][Some ID] [shortcut] [collapsed][] ![photo][pic]\n\n[some   id]: https://example.com/first?a=1&b=2 \"Link title\"\n[SOME ID]: https://example.com/ignored\n[shortcut]: mailto:team@example.com\n[collapsed]: http://example.com/c\n[pic]: https://example.com/photo.png \"Photo title\"",
        markdown: true, signature: "", earlier: None,
    }).unwrap();
    assert!(
        made.text.starts_with("go shortcut collapsed photo"),
        "{}",
        made.text
    );
    let html = made.html.unwrap();
    assert!(
        html.contains("href=\"https://example.com/first?a=1&amp;b=2\""),
        "{html}"
    );
    assert!(html.contains("<strong>go</strong></a>"), "{html}");
    assert!(html.contains("title=\"Link title\""), "{html}");
    assert!(html.contains("href=\"mailto:team@example.com\""), "{html}");
    assert!(html.contains("href=\"http://example.com/c\""), "{html}");
    assert!(
        html.contains(
            "<img src=\"https://example.com/photo.png\" alt=\"photo\" title=\"Photo title\">"
        ),
        "{html}"
    );
    assert!(!html.contains("ignored"), "{html}");
}

#[test]
fn rich_draft_images_keep_safe_sources_and_escape_attributes() {
    let made = bodies(&Writing {
        words: "![a & b](http://example.com/p.png \"a & b\") ![secure](https://example.com/s.png)",
        markdown: true,
        signature: "",
        earlier: None,
    })
    .unwrap();
    assert_eq!(made.text, "a & b secure");
    let html = made.html.unwrap();
    assert!(
        html.contains(
            "<img src=\"http://example.com/p.png\" alt=\"a &amp; b\" title=\"a &amp; b\">"
        ),
        "{html}"
    );
    assert!(
        html.contains("<img src=\"https://example.com/s.png\" alt=\"secure\">"),
        "{html}"
    );
}

#[test]
fn rich_draft_references_and_images_drop_unsafe_schemes() {
    let made = bodies(&Writing {
        words: "[go][bad] ![photo][pic] ![inline](data:image/png;base64,eA==) ![mail](mailto:a@example.com)\n\n[bad]: javascript:alert%281%29\n[pic]: javascript:alert%282%29",
        markdown: true, signature: "", earlier: None,
    }).unwrap();
    assert!(
        made.text.starts_with("go photo inline mail"),
        "{}",
        made.text
    );
    let html = made.html.unwrap();
    assert!(!html.contains("href="), "{html}");
    assert!(!html.contains("<img"), "{html}");
    assert!(
        !html.contains("javascript:") && !html.contains("data:") && !html.contains("mailto:"),
        "{html}"
    );
}

#[test]
fn task_list_states_match_both_mime_alternatives() {
    let made = bodies(&Writing {
        words: "- [x] approved\n- [ ] pending",
        markdown: true,
        signature: "",
        earlier: None,
    })
    .unwrap();
    assert_eq!(made.text, "- [x] approved\n- [ ] pending");
    assert!(made.html.unwrap().contains("checked"));
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
