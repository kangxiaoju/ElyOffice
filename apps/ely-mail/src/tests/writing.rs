//! Writing: replies, forwards, sending, the sender's identity (C12.12).

use std::path::Path;

use ely_gpui_component::mail::Draft;
use ely_mail_engine::{
    Fetched, Identity, Key, Mailbox, MessageId, Role, Store,
    compose::{Earlier, attachment},
    read, without_bcc,
};
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

use super::{Keys, account, settle, window};
use crate::{
    boxes::Open,
    compose::Compose,
    writing::{How, Outgoing, prepare},
};

pub(super) const NOTE: &str = "From: Ann <ann@example.com>\r\nTo: bo@example.com, Cy <cy@example.com>\r\n\
                    Cc: dee@example.com\r\nSubject: plan\r\nDate: 1 Jan 2024 09:00:00 +0000\r\n\
                    Message-ID: <a@x>\r\n";

/// Bo's inbox with one message; returns its id.
pub(super) fn seeded(dir: &Path, bytes: &[u8], missing: &[String]) -> MessageId {
    let mut store = Store::open(dir).unwrap();
    let bo = store.add_account(&account("bo@example.com")).unwrap();
    let [inbox] = store
        .set_folders(bo, &[("INBOX", Some(Role::Inbox))])
        .unwrap()
        .try_into()
        .unwrap();
    let key = Key::Imap {
        uidvalidity: 1,
        uid: 1,
    };
    let fetched = match missing.is_empty() {
        true => Fetched::Whole(bytes),
        false => Fetched::Text { bytes, missing },
    };
    store.put(inbox, &key, &[], fetched).unwrap()
}

pub(super) fn composing(cx: &mut VisualTestContext) -> Vec<WindowHandle<Compose>> {
    cx.update(|_, app| {
        app.windows()
            .into_iter()
            .filter_map(|each| each.downcast::<Compose>())
            .collect()
    })
}

/// Waits for the one compose window.
pub(super) fn opened(cx: &mut VisualTestContext) -> WindowHandle<Compose> {
    settle(cx, |app| {
        app.windows()
            .iter()
            .any(|each| each.downcast::<Compose>().is_some())
    });
    let mut all = composing(cx);
    assert_eq!(all.len(), 1);
    all.remove(0)
}

pub(super) fn draft(to: &[&str], subject: &str) -> Draft {
    Draft {
        to: to.iter().map(|to| to.to_string().into()).collect(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: subject.to_owned().into(),
    }
}

#[test]
fn reply_all_starts_addressed_and_quoting_the_message() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = format!("{NOTE}Content-Type: text/html\r\n\r\n<p>Lunch <b>Friday</b>?</p>\r\n");
    let id = seeded(dir.path(), bytes.as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.answer(id, How::ReplyAll, cx)));
    let compose = opened(cx);
    cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        let shown =
            |list: &[gpui::SharedString]| list.iter().map(|it| it.to_string()).collect::<Vec<_>>();
        assert_eq!(shown(&compose.to), ["ann@example.com", "cy@example.com"]);
        assert_eq!(shown(&compose.cc), ["dee@example.com"]);
        assert_eq!(compose.subject.as_ref(), "Re: plan");
        let Some(Earlier::Quoted { lead, text }) = &compose.before else {
            panic!("the message is quoted");
        };
        assert!(lead.starts_with("On 2024-01-0"), "{lead}");
        assert!(lead.ends_with("Ann <ann@example.com> wrote:"), "{lead}");
        assert_eq!(text, "Lunch Friday?");
    });
    // The quoted reply threads under the original.
    let bytes = cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        let out = compose
            .outgoing(&draft(&["ann@example.com"], "Re: plan"), app)
            .unwrap();
        without_bcc(&prepare(&out).unwrap().bytes).unwrap()
    });
    let sent = read(&bytes).unwrap();
    assert_eq!(sent.in_reply_to, ["a@x"]);
    assert_eq!(sent.references, ["a@x"]);
    assert!(sent.text.unwrap().contains("> Lunch Friday?"));
}

#[test]
fn a_forward_carries_the_files_that_are_here() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = format!(
        "{NOTE}Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee both\r\n\
         --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=a.pdf\r\n\r\n%PDF\r\n\
         --b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=b.pdf\r\n\r\n\r\n--b--\r\n"
    );
    let id = seeded(dir.path(), bytes.as_bytes(), &["3".to_owned()]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.answer(id, How::Forward, cx)));
    let compose = opened(cx);
    cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        assert!(compose.to.is_empty());
        assert_eq!(compose.subject.as_ref(), "Fwd: plan");
        let names: Vec<_> = compose
            .files
            .iter()
            .map(|file| file.part.name.clone())
            .collect();
        assert_eq!(names, [Some("a.pdf".to_owned())]);
        assert_eq!(compose.missing, 1, "b.pdf is not downloaded");
        let Some(Earlier::Forwarded { lead, text }) = &compose.before else {
            panic!("the message is forwarded");
        };
        assert!(
            lead.contains(&"From: Ann <ann@example.com>".to_owned()),
            "{lead:?}"
        );
        assert!(lead.contains(&"Cc: dee@example.com".to_owned()), "{lead:?}");
        assert_eq!(text, "See both");
    });
    let mut there = VisualTestContext::from_window(compose.into(), cx);
    settle(&mut there, |_| true);
    assert!(there.debug_bounds("write-missing").is_some());
}

#[test]
fn a_message_is_built_with_its_hidden_copies_off_the_page() {
    let mailbox = |email: &str| Mailbox {
        name: None,
        email: email.into(),
    };
    let out = Outgoing {
        from: Mailbox {
            name: Some("Bo Lee".into()),
            email: "bo@example.com".into(),
        },
        to: vec![mailbox("ann@example.com")],
        cc: vec![mailbox("ANN@example.com"), mailbox("cy@example.com")],
        bcc: vec![mailbox("hidden@example.com")],
        subject: "Plans bcc marker".into(),
        text: "Friday then".into(),
        html: "<p><strong>Friday</strong> then</p>".into(),
        signature: "Bo".into(),
        before: None,
        attachments: vec![attachment("plan.pdf", b"%PDF".to_vec())],
        in_reply_to: None,
        references: Vec::new(),
        date: 1_704_099_600,
    };
    let built = prepare(&out).unwrap();
    let bytes = without_bcc(&built.bytes).unwrap();
    assert_eq!(
        built.envelope,
        ["ann@example.com", "cy@example.com", "hidden@example.com"]
    );
    let raw = String::from_utf8_lossy(&bytes);
    // Not "bcc" anywhere: a hex Message-ID may hold it.
    let head = &raw[..raw.find("\r\n\r\n").expect("a header end")];
    assert!(
        !head
            .lines()
            .any(|line| line.to_ascii_lowercase().starts_with("bcc:")),
        "{raw}"
    );
    assert!(!raw.contains("hidden@example.com"), "{raw}");
    let sent = read(&bytes).unwrap();
    assert_eq!(sent.subject.as_deref(), Some("Plans bcc marker"));
    assert_eq!(sent.from[0].name.as_deref(), Some("Bo Lee"));
    let text = sent.text.unwrap().replace("\r\n", "\n");
    assert_eq!(text.trim_end(), "Friday then\n\n-- \nBo");
    assert!(sent.html.unwrap().contains("<strong>Friday</strong>"));
    assert_eq!(sent.attachments[0].name.as_deref(), Some("plan.pdf"));
    let copy = prepare(&out).unwrap().bytes;
    let copy = String::from_utf8_lossy(&copy);
    let head = &copy[..copy.find("\r\n\r\n").unwrap()];
    assert!(
        head.contains("hidden@example.com"),
        "a draft keeps Bcc: {head}"
    );
    assert!(
        head.lines()
            .any(|line| line.to_ascii_lowercase().starts_with("bcc:")),
        "{head}"
    );
}

#[test]
fn legacy_drafts_reopen_as_html_without_exposing_format_syntax() {
    use ely_gpui_component::documents::email::EmailDocument;
    for (markdown, source, visible) in [
        (false, "**literal**", "**literal**"),
        (true, "**formatted**", "formatted"),
    ] {
        let html = ely_mail_engine::compose::legacy_draft_html(source, markdown).unwrap();
        let document = EmailDocument::from_html(&html).unwrap();
        assert_eq!(document.to_plain(), visible);
        let mut draft = super::outbox::unsent("");
        draft.html = Some(document.to_html());
        let json = serde_json::to_string(&draft).unwrap();
        assert!(!json.contains("\"words\""));
        assert!(!json.contains("\"markdown\""));
        let restored: ely_mail_engine::Unsent = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.html, draft.html);
        let bodies = ely_mail_engine::compose::bodies(&ely_mail_engine::compose::Writing {
            text: &document.to_plain(), html: restored.html.as_deref().unwrap(),
            signature: "", earlier: None,
        }).unwrap();
        assert_eq!(bodies.text, visible);
        assert!(bodies.html.is_some());
    }
}

#[test]
fn old_plain_and_markdown_json_reopen_in_native_editor_and_roundtrip_mime() {
    for markdown in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
        let store = Store::open(dir.path()).unwrap();
        let from = store.accounts().unwrap()[0].0;
        let senders = crate::writing::senders(&store).unwrap();
        let unsent: ely_mail_engine::Unsent = serde_json::from_value(serde_json::json!({
            "to": ["ann@example.com"], "cc": [], "bcc": [], "subject": "Old",
            "words": "**Lunch**", "markdown": markdown, "earlier": null,
            "in_reply_to": null, "references": [], "headers_only": false, "missing": 0
        })).unwrap();
        let mut cx = TestAppContext::single();
        let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
        cx.update(|_, app| mail.update(app, |mail, cx| mail.open_compose(crate::writing::Opening {
            senders, from, answered: None, asked: None,
            draft: Some(crate::compose::Reopened { id: ely_mail_engine::DraftId(900), unsent, files: Vec::new() }),
        }, cx)));
        let compose = opened(cx);
        let out = cx.update(|_, app| compose.read(app).unwrap().outgoing(&draft(&["ann@example.com"], "Old"), app).unwrap());
        assert_eq!(out.text, if markdown { "Lunch" } else { "**Lunch**" });
        assert_eq!(out.html.contains("<strong>Lunch</strong>"), markdown);
        let sent = read(&prepare(&out).unwrap().bytes).unwrap();
        assert_eq!(sent.text.unwrap().trim_end(), out.text);
        assert_eq!(sent.html.unwrap().contains("<strong>Lunch</strong>"), markdown);
    }
}

#[test]
fn malformed_draft_can_close_without_overwriting_after_header_edits() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut store = Store::open(dir.path()).unwrap();
    let from = store.accounts().unwrap()[0].0;
    let mut broken = super::outbox::unsent("");
    broken.html = Some("x".repeat(2 * 1024 * 1024 + 1));
    let id = store.shelve(None, from, &broken, None).unwrap().id;
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.reopen(id, cx)));
    let compose = opened(cx);
    let may_close = cx.update(|_, app| compose.update(app, |compose, window, cx| {
        compose.edited(draft(&["ann@example.com"], "cannot save header"), cx);
        compose.choose_sender(ely_mail_engine::AccountId(from.0 + 100), cx);
        compose.attach(vec![dir.path().join("must-not-read.png")], cx);
        assert_eq!(compose.from, from, "sender changes are rejected");
        assert_eq!(compose.reading, 0, "attachment requests are rejected before IO");
        assert!(compose.outgoing(&draft(&["ann@example.com"], "broken"), cx).is_none());
        compose.keep(cx);
        assert!(compose.outgoing(&draft(&["ann@example.com"], "broken"), cx).is_none());
        compose.may_close(window, cx)
    }).unwrap());
    assert!(may_close, "malformed bodies keep their stored source and may safely close");
    assert_eq!(store.draft(id).unwrap().unwrap().unsent.html, broken.html);
    assert_eq!(store.draft(id).unwrap().unwrap().unsent.subject, broken.subject);
}

#[test]
fn asynchronous_clipboard_and_picture_results_refuse_later_edits() {
    use ely_gpui_component::documents::email::{EmailAlign, EmailDocument, EmailImage};
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.write_new(cx)));
    let compose = opened(cx);
    cx.update(|_, app| compose.update(app, |compose, _, cx| {
        compose.document.update(cx, |editor, cx| {
            let paste = editor.request_paste(cx);
            editor.insert_text("later", cx).unwrap();
            assert!(editor.complete_paste(paste, EmailDocument::from_html("<p><strong>stale</strong></p>").unwrap(), cx).is_err());
            let picture = editor.request_image(cx);
            editor.insert_text(" edit", cx).unwrap();
            assert!(editor.complete_image(picture, EmailImage {
                source: "cid:stale@image".into(), alt: "stale".into(), width: Some(1), height: Some(1), align: EmailAlign::Left,
            }, cx).is_err());
            assert_eq!(editor.document().to_plain(), "later edit");
            assert!(!editor.document().to_html().contains("stale"));
        });
        assert!(compose.files.is_empty(), "a stale result cannot attach orphaned bytes");
    }).unwrap());
}

#[test]
fn words_written_are_kept_on_closing() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.write_new(cx)));
    let compose = opened(cx);
    let closes = |cx: &mut VisualTestContext| {
        cx.update(|_, app| {
            compose
                .update(app, |compose, window, cx| compose.may_close(window, cx))
                .unwrap()
        })
    };
    assert!(closes(cx), "a blank message closes");
    cx.update(|_, app| {
        compose
            .update(app, |compose, _, cx| {
                compose
                    .document
                    .update(cx, |document, cx| document.set_document(ely_gpui_component::documents::email::EmailDocument::from_plain("Hello").unwrap(), cx).unwrap())
            })
            .unwrap()
    });
    assert!(!closes(cx), "words are kept first");
    settle(cx, |app| compose.read(app).is_err());
}

#[test]
fn a_saved_name_and_signature_sign_the_next_message() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let bo = cx.update(|_, app| mail.read(app).accounts()[0].0);
    cx.update(|_, app| mail.update(app, |mail, cx| mail.open_box(Open::Account(bo), cx)));
    settle(cx, |app| {
        mail.read(app)
            .identity
            .as_ref()
            .is_some_and(|editing| editing.kept.is_some())
    });
    assert!(cx.debug_bounds("mail-identity").is_some());
    let signed = Identity {
        name: Some("Bo Lee".into()),
        signature: "Bo\nAtrium".into(),
    };
    cx.update(|_, app| mail.update(app, |mail, cx| mail.save_identity(bo, signed.clone(), cx)));
    settle(cx, |_| {
        Store::open(dir.path()).unwrap().identity(bo).unwrap() == signed
    });
    cx.update(|_, app| mail.update(app, |mail, cx| mail.write_new(cx)));
    let compose = opened(cx);
    cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        assert_eq!(compose.from, bo);
        assert_eq!(compose.senders[0].identity, signed);
        let out = compose
            .outgoing(&draft(&["ann@example.com"], "Hi"), app)
            .unwrap();
        let sent = read(&prepare(&out).unwrap().bytes).unwrap();
        assert_eq!(sent.from[0].name.as_deref(), Some("Bo Lee"));
        let text = sent.text.unwrap().replace("\r\n", "\n");
        assert!(text.ends_with("-- \nBo\nAtrium"), "{text:?}");
    });
}

#[test]
fn a_message_kept_as_headers_is_answered_without_quoting() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let bo = store.add_account(&account("bo@example.com")).unwrap();
    let [inbox] = store
        .set_folders(bo, &[("INBOX", Some(Role::Inbox))])
        .unwrap()
        .try_into()
        .unwrap();
    let key = Key::Imap {
        uidvalidity: 1,
        uid: 1,
    };
    let head = format!("{NOTE}\r\n");
    let id = store
        .put(inbox, &key, &[], Fetched::Headers(head.as_bytes()))
        .unwrap();
    drop(store);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.answer(id, How::Reply, cx)));
    let compose = opened(cx);
    cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        assert_eq!(compose.subject.as_ref(), "Re: plan");
        assert!(compose.headers_only);
        assert!(compose.before.is_none(), "no words to quote");
    });
}

#[test]
fn the_shared_new_command_opens_a_message() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    store.add_account(&account("me@example.com")).unwrap();
    drop(store);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    // As Ctrl+N and File > New send it.
    cx.update(|window, app| {
        let focus = mail.read(app).focus.clone();
        window.focus(&focus, app);
        window.dispatch_action(Box::new(ely_ui::commands::New), app)
    });
    opened(cx);
}
