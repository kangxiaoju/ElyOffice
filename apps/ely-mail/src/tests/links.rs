use std::path::PathBuf;

use ely_gpui_component::mail::Draft;
use gpui::TestAppContext;

use super::{
    Keys, window,
    writing::{NOTE, composing, opened, seeded},
};
use crate::writing::prepare;

#[test]
fn a_mail_link_opens_a_message_filled_from_it() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let link = PathBuf::from("mailto:a@b.c?subject=Hi%20there&cc=d@e.f&body=one%0D%0Atwo");
    cx.update(|_, app| mail.update(app, |mail, cx| mail.take_files(vec![link], cx)));
    let compose = opened(cx);
    cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        assert_eq!(compose.to, ["a@b.c"]);
        assert_eq!(compose.cc, ["d@e.f"]);
        assert_eq!(compose.subject.as_ref(), "Hi there");
        let out = compose
            .outgoing(&super::writing::draft(&["a@b.c"], "Hi there"), app)
            .unwrap();
        assert!(!out.html.is_empty(), "links open in the HTML editor");
        assert_eq!(out.text, "one\ntwo");
    });
}

#[test]
fn mailto_markdown_punctuation_stays_literal_when_sent() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let link = PathBuf::from(
        "mailto:a@b.c?body=%23%20Title%0A**bold**%0A%5Blabel%5D(https://example.com)",
    );
    cx.update(|_, app| mail.update(app, |mail, cx| mail.take_files(vec![link], cx)));
    let compose = opened(cx);
    let out = cx.update(|_, app| {
        compose
            .read(app)
            .unwrap()
            .outgoing(&super::writing::draft(&["a@b.c"], ""), app)
            .unwrap()
    });
    assert!(!out.html.is_empty());
    let sent = ely_mail_engine::read(&prepare(&out).unwrap().bytes).unwrap();
    let html = sent.html.unwrap();
    assert!(html.contains("# Title"), "{html}");
    assert!(html.contains("**bold**"), "{html}");
    assert!(html.contains("[label](https://example.com)"), "{html}");
    assert!(!html.contains("<strong>"), "{html}");
    assert!(!html.contains("<h1>"), "{html}");
    assert!(!html.contains("<a "), "{html}");
}

#[test]
fn a_broken_mail_link_opens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    for link in [
        "mailto:a@b.c?subject=%zz",
        "mailto:a@b.c?subject=%3D%3FUTF-8%3FQ%3Fa%3D0D%3D0Ab%3F%3D",
    ] {
        cx.update(|_, app| {
            mail.update(app, |mail, cx| {
                mail.take_files(vec![PathBuf::from(link)], cx)
            })
        });
        cx.run_until_parked();
        assert!(composing(cx).is_empty(), "{link}");
    }
}

#[test]
fn quoted_international_and_encoded_parts_reach_the_envelope() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let link = "mailto:%22last%2Cfirst%22@example.org?cc=user@%E7%B4%8D%E8%B1%86.example.org\
        &subject=%3D%3Futf-8%3FQ%3Fcaf%3DC3%3DA9%3F%3D";
    cx.update(|_, app| {
        mail.update(app, |mail, cx| {
            mail.take_files(vec![PathBuf::from(link)], cx)
        })
    });
    let compose = opened(cx);
    let built = cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        assert_eq!(compose.to, ["\"last,first\"@example.org"]);
        assert_eq!(compose.subject.as_ref(), "café");
        let written = Draft {
            to: compose.to.clone(),
            cc: compose.cc.clone(),
            bcc: compose.bcc.clone(),
            subject: compose.subject.clone(),
        };
        prepare(&compose.outgoing(&written, app).unwrap()).unwrap()
    });
    assert_eq!(
        built.envelope,
        ["\"last,first\"@example.org", "user@xn--99zt52a.example.org"]
    );
}
