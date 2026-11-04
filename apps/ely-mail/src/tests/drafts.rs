//! Drafts kept while writing, listed, reopened, removed (C12.13).

use ely_gpui_component::primitives::Severity;
use ely_mail_engine::{Role, Store};
use ely_platform::AppId;
use ely_ui::Root;
use gpui::{Entity, TestAppContext, VisualTestContext, WindowHandle};

use super::{
    Keys, account,
    leaving::kept,
    settle, window,
    writing::{NOTE, composing, draft, opened, seeded},
};
mod deleting;

use crate::{
    boxes::Open,
    compose::{Compose, PAUSE},
    draft_list,
    mail::Mail,
};

pub(super) fn with<R>(
    compose: WindowHandle<Compose>,
    cx: &mut VisualTestContext,
    act: impl FnOnce(&mut Compose, &mut gpui::Window, &mut gpui::Context<Compose>) -> R,
) -> R {
    cx.update(|_, app| compose.update(app, act).unwrap())
}

pub(super) fn write(compose: WindowHandle<Compose>, words: &str, cx: &mut VisualTestContext) {
    with(compose, cx, |compose, _, cx| {
        compose
            .document
            .update(cx, |document, cx| document.set_document(ely_gpui_component::documents::email::EmailDocument::from_plain(words).unwrap(), cx).unwrap())
    });
}

/// Lets typing stop, then waits on the store.
pub(super) fn pause(mail: &Entity<Mail>, cx: &mut VisualTestContext, drafts: usize) {
    cx.executor().advance_clock(PAUSE);
    settle(cx, |app| {
        let store = mail.read(app).store.clone().unwrap();
        let count = store.lock().unwrap().drafts().unwrap().len();
        count == drafts
    });
}

pub(super) fn new_message(
    mail: &Entity<Mail>,
    cx: &mut VisualTestContext,
) -> WindowHandle<Compose> {
    cx.update(|_, app| mail.update(app, |mail, cx| mail.write_new(cx)));
    opened(cx)
}

/// A shell around the window, to read its toasts.
pub(super) fn rooted(mail: &Entity<Mail>, cx: &mut VisualTestContext) -> Entity<Root> {
    cx.update(|window, app| {
        let root = ely_ui::tool_root(AppId::Mail, mail.clone().into(), Vec::new(), window, app);
        mail.update(app, |mail, _| mail.root = root.downgrade());
        root
    })
}

pub(super) fn told(root: &Entity<Root>, severity: Severity, cx: &mut VisualTestContext) -> bool {
    cx.update(|_, app| root.read(app).toasts(app))
        .iter()
        .any(|(_, shown)| *shown == Some(severity))
}

pub(super) fn problem(
    compose: WindowHandle<Compose>,
    cx: &mut VisualTestContext,
) -> Option<Severity> {
    cx.update(|_, app| compose.read(app).unwrap().problem.clone())
        .map(|(severity, _)| severity)
}

#[test]
fn words_are_kept_once_typing_stops() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "Lunch?", cx);
    cx.run_until_parked();
    assert!(kept(&mail, cx).is_empty(), "not while typing");
    pause(&mail, cx, 1);
    let [first] = kept(&mail, cx).try_into().unwrap();
    assert_eq!(first.unsent.body_text(), "Lunch?");
    assert!(first.unsent.html.is_some(), "new messages save native HTML");
    write(compose, "Lunch at one?", cx);
    with(compose, cx, |compose, _, cx| {
        compose.edited(draft(&["ann@example.com"], "Plan"), cx)
    });
    cx.executor().advance_clock(PAUSE);
    settle(cx, |app| {
        let store = mail.read(app).store.clone().unwrap();
        let drafts = store.lock().unwrap().drafts().unwrap();
        drafts.len() == 1 && drafts[0].unsent.subject == "Plan"
    });
    let [second] = kept(&mail, cx).try_into().unwrap();
    assert_eq!(second.id, first.id, "the same draft, written again");
    assert_eq!(second.unsent.body_text(), "Lunch at one?");
    assert_eq!(second.unsent.to, ["ann@example.com"]);
    write(compose, "", cx);
    with(compose, cx, |compose, _, cx| {
        compose.edited(draft(&[], ""), cx)
    });
    pause(&mail, cx, 0);
    assert!(
        cx.update(|_, app| compose.read(app).unwrap().shelf.id.is_none()),
        "back to blank, the draft goes"
    );
}

#[test]
fn rich_formatting_is_autosaved_and_sent_without_a_mode_switch() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "Lunch", cx);
    with(compose, cx, |compose, _, cx| {
        compose
            .document
            .update(cx, |input, cx| {
                let selection = ely_gpui_component::documents::email::EmailSelection { anchor: input.document().first_point(), head: input.document().end_point() };
                input.set_selection(selection, cx);
            });
    });
    {
        let mut there = VisualTestContext::from_window(compose.into(), cx);
        there.run_until_parked();
        assert!(there.debug_bounds("segment plain").is_none());
        assert!(there.debug_bounds("segment markdown").is_none());
        let bold = there
            .debug_bounds("toggle bold")
            .expect("the rich formatting toolbar");
        there.simulate_click(bold.center(), gpui::Modifiers::none());
        there.run_until_parked();
    }
    pause(&mail, cx, 1);
    let [saved] = kept(&mail, cx).try_into().unwrap();
    assert!(saved.unsent.html.as_deref().unwrap().contains("<strong>Lunch</strong>"));
    assert_eq!(saved.unsent.body_text(), "Lunch");
    let out = cx.update(|_, app| {
        compose
            .read(app)
            .unwrap()
            .outgoing(&draft(&["ann@example.com"], "Hi"), app)
            .unwrap()
    });
    assert_eq!(out.text, "Lunch");
    let sent = ely_mail_engine::read(&crate::writing::prepare(&out).unwrap().bytes).unwrap();
    assert!(sent.html.unwrap().contains("<strong>Lunch</strong>"));
}

#[test]
fn a_draft_reopens_as_it_was_and_once() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "**Lunch?**", cx);
    with(compose, cx, |compose, _, cx| {
        let mut addressed = draft(&["ann@example.com"], "Plan");
        addressed.bcc = vec!["cy@example.com".into()];
        compose.edited(addressed, cx);
    });
    let file = dir.path().join("menu.txt");
    std::fs::write(&file, "soup").unwrap();
    with(compose, cx, |compose, _, cx| compose.attach(vec![file], cx));
    settle(cx, |app| compose.read(app).unwrap().reading == 0);
    pause(&mail, cx, 1);
    with(compose, cx, |_, window, _| window.remove_window());
    settle(cx, |app| compose.read(app).is_err());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.open_box(Open::Drafts, cx)));
    settle(
        cx,
        |app| matches!(&mail.read(app).listing.mails, Some(Ok(rows)) if rows.len() == 1),
    );
    let [shelved] = kept(&mail, cx).try_into().unwrap();
    let key = draft_list::key(shelved.id);
    cx.update(|_, app| mail.update(app, |mail, cx| mail.select(&[key], cx)));
    settle(cx, |_| true);
    assert!(
        cx.debug_bounds("mail-draft-words").is_some(),
        "the draft shows"
    );
    for _ in 0..2 {
        cx.update(|_, app| mail.update(app, |mail, cx| mail.reopen(shelved.id, cx)));
        settle(cx, |_| true);
    }
    let [again] = composing(cx).try_into().unwrap();
    cx.update(|_, app| {
        let again = again.read(app).unwrap();
        assert_eq!(again.shelf.id, Some(shelved.id));
        assert_eq!(again.to, ["ann@example.com"]);
        assert_eq!(again.bcc, ["cy@example.com"]);
        assert_eq!(again.subject.as_ref(), "Plan");
        assert_eq!(again.files.len(), 1);
        assert_eq!(again.document.read(app).document().to_plain(), "**Lunch?**");
    });
    let shut = with(again, cx, |compose, window, cx| {
        compose.may_close(window, cx)
    });
    assert!(shut, "untouched, it closes as it is");
    assert_eq!(
        kept(&mail, cx),
        std::slice::from_ref(&shelved),
        "the draft stays unchanged"
    );
    with(again, cx, |_, window, _| window.remove_window());
    settle(cx, |app| again.read(app).is_err());
    cx.update(|_, app| {
        mail.update(app, |mail, cx| {
            mail.reopen(shelved.id, cx);
            mail.reopen(shelved.id, cx);
        })
    });
    settle(cx, |_| true);
    assert_eq!(composing(cx).len(), 1, "asked twice at once, one window");
}

#[test]
fn a_plain_draft_reopens_literal_and_is_saved_and_sent_as_rich() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let literal = "# Title\n**bold**\n[label](https://example.com)";
    let mut store = Store::open(dir.path()).unwrap();
    let from = store.accounts().unwrap()[0].0;
    let old = super::outbox::unsent(literal);
    let id = store.shelve(None, from, &old, None).unwrap().id;
    drop(store);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.reopen(id, cx)));
    let compose = opened(cx);
    let out = cx.update(|_, app| {
        let compose = compose.read(app).unwrap();
        assert!(compose.document.read(app).document().validate().is_ok());
        compose
            .outgoing(&draft(&["ann@example.com"], "Hi"), app)
            .unwrap()
    });
    assert!(!out.html.is_empty());
    assert_eq!(out.text, literal);
    let sent = ely_mail_engine::read(&crate::writing::prepare(&out).unwrap().bytes).unwrap();
    let html = sent.html.unwrap();
    assert!(html.contains("**bold**"), "{html}");
    assert!(!html.contains("<strong>"), "{html}");
    with(compose, cx, |compose, _, cx| {
        compose.edited(draft(&["ann@example.com"], "Updated"), cx)
    });
    pause(&mail, cx, 1);
    settle(cx, |app| {
        let store = mail.read(app).store.clone().unwrap();
        store.lock().unwrap().drafts().unwrap()[0].unsent.subject == "Updated"
    });
    let [saved] = kept(&mail, cx).try_into().unwrap();
    assert!(saved.unsent.html.is_some());
    assert_eq!(saved.unsent.body_text(), out.text);
    with(compose, cx, |_, window, _| window.remove_window());
    settle(cx, |app| compose.read(app).is_err());
    cx.update(|_, app| mail.update(app, |mail, cx| mail.reopen(id, cx)));
    let again = opened(cx);
    cx.update(|_, app| {
        let out = again
            .read(app)
            .unwrap()
            .outgoing(&draft(&["ann@example.com"], "Updated"), app)
            .unwrap();
        assert!(!out.html.is_empty());
        assert_eq!(
            out.html, saved.unsent.html.unwrap(),
            "HTML drafts are not escaped twice"
        );
    });
}

/// Holds whatever connects, until told to let go.
pub(super) fn silent_server() -> (u16, std::sync::mpsc::Sender<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    let (stop, stopped) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut held = Vec::new();
        while stopped.try_recv().is_err() {
            if let Ok((stream, _)) = listener.accept() {
                held.push(stream);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    });
    (port, stop)
}

#[test]
fn a_server_copy_failing_after_its_window_closed_says_so_here() {
    let dir = tempfile::tempdir().unwrap();
    let (port, stop) = silent_server();
    let mut quiet = account("bo@example.com");
    quiet.incoming.port = port;
    quiet.outgoing.port = port;
    let mut store = Store::open(dir.path()).unwrap();
    let bo = store.add_account(&quiet).unwrap();
    store
        .set_folders(
            bo,
            &[("INBOX", Some(Role::Inbox)), ("Drafts", Some(Role::Drafts))],
        )
        .unwrap();
    store.set_drafts_up(bo, true).unwrap();
    drop(store);
    let keys = Keys::default();
    keys.kept
        .lock()
        .unwrap()
        .insert("account:bo@example.com".into(), "pw".into());
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), keys);
    let root = rooted(&mail, cx);
    let compose = new_message(&mail, cx);
    write(compose, "words", cx);
    let shut = with(compose, cx, |compose, window, cx| {
        compose.may_close(window, cx)
    });
    assert!(!shut, "kept first");
    settle(cx, |app| compose.read(app).is_err());
    assert!(
        !cx.update(|_, app| mail.read(app).pushing.is_empty()),
        "still going up"
    );
    stop.send(()).unwrap();
    settle(cx, |app| {
        let toasts = root.read(app).toasts(app);
        toasts
            .iter()
            .any(|(_, shown)| *shown == Some(Severity::Warning))
    });
}

#[test]
fn spacing_alone_is_an_edit_and_spaces_alone_are_blank() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "   ", cx);
    let shut = with(compose, cx, |compose, window, cx| {
        compose.may_close(window, cx)
    });
    assert!(shut, "only spaces, nothing to keep");
    write(compose, "hello", cx);
    pause(&mail, cx, 1);
    write(compose, "    hello", cx);
    let shut = with(compose, cx, |compose, window, cx| {
        compose.may_close(window, cx)
    });
    assert!(!shut, "the indent is unkept yet");
    settle(cx, |app| compose.read(app).is_err());
    assert_eq!(ely_gpui_component::documents::email::EmailDocument::from_html(kept(&mail, cx)[0].unsent.html.as_deref().unwrap()).unwrap().to_plain(), "    hello");
}

#[test]
fn a_file_unread_while_closing_keeps_the_window() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "see attached", cx);
    let gone = dir.path().join("gone.txt");
    let shut = with(compose, cx, |compose, window, cx| {
        compose.attach(vec![gone], cx);
        compose.may_close(window, cx)
    });
    assert!(!shut, "waits for the file");
    settle(cx, |app| compose.read(app).unwrap().reading == 0);
    assert_eq!(problem(compose, cx), Some(Severity::Danger));
    write(compose, "see below", cx);
    pause(&mail, cx, 1);
    assert!(
        cx.update(|_, app| compose.read(app).is_ok()),
        "the failed file ended that close"
    );
    let shut = with(compose, cx, |compose, window, cx| {
        compose.may_close(window, cx)
    });
    assert!(shut, "kept already, it closes");
    assert_eq!(kept(&mail, cx)[0].unsent.body_text(), "see below");
}

#[test]
fn a_draft_the_store_refuses_keeps_the_window() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "words", cx);
    let from = cx.update(|_, app| compose.read(app).unwrap().from);
    let mut other = Store::open(dir.path()).unwrap();
    other.remove_account(from).unwrap();
    let shut = with(compose, cx, |compose, window, cx| {
        compose.may_close(window, cx)
    });
    assert!(!shut);
    settle(cx, |_| true);
    assert!(
        cx.update(|_, app| compose.read(app).is_ok()),
        "the words stay"
    );
    assert_eq!(problem(compose, cx), Some(Severity::Danger));
}

#[test]
fn a_server_copy_needs_a_drafts_folder_and_says_why_not() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    let from = cx.update(|_, app| compose.read(app).unwrap().from);
    cx.update(|_, app| mail.update(app, |mail, cx| mail.open_box(Open::Account(from), cx)));
    settle(cx, |app| mail.read(app).drafts_up == Some((from, false)));
    assert!(
        cx.debug_bounds("mail-drafts-up").is_some(),
        "the switch shows"
    );
    let switch = cx.debug_bounds("mail-drafts-up").unwrap();
    cx.simulate_click(switch.center(), gpui::Modifiers::none());
    settle(cx, |app| mail.read(app).drafts_up == Some((from, true)));
    write(compose, "words", cx);
    pause(&mail, cx, 1);
    settle(cx, |_| true);
    assert_eq!(
        problem(compose, cx),
        Some(Severity::Info),
        "no Drafts folder"
    );
    let store = cx.update(|_, app| mail.read(app).store.clone().unwrap());
    store
        .lock()
        .unwrap()
        .set_folders(
            from,
            &[("INBOX", Some(Role::Inbox)), ("Drafts", Some(Role::Drafts))],
        )
        .unwrap();
    write(compose, "more words", cx);
    cx.executor().advance_clock(PAUSE);
    settle(cx, |app| {
        let problem = compose.read(app).unwrap().problem.clone();
        problem.is_some_and(|(severity, _)| severity == Severity::Warning)
    });
    assert!(cx.update(|_, app| mail.read(app).pushing.is_empty()));
}

#[test]
fn drafts_have_no_mail_filters() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    settle(cx, |_| true);
    assert!(cx.debug_bounds("mail-filters").is_some(), "inboxes filter");
    cx.update(|_, app| mail.update(app, |mail, cx| mail.open_box(Open::Drafts, cx)));
    settle(cx, |_| true);
    assert!(cx.debug_bounds("mail-filters").is_none());
}

#[test]
fn oversized_escaped_html_never_overwrites_a_reopenable_draft() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "safe", cx);
    pause(&mail, cx, 1);
    write(compose, &"&".repeat(500_000), cx);
    with(compose, cx, |compose, _, cx| compose.keep(cx));
    settle(cx, |app| compose.read(app).unwrap().problem.as_ref().is_some_and(|(severity, _)| *severity == Severity::Danger));
    assert_eq!(kept(&mail, cx)[0].unsent.body_text(), "safe");
    assert!(cx.update(|_, app| compose.read(app).unwrap().outgoing(&draft(&["ann@example.com"], "Large"), app).is_none()));
}

#[test]
fn failed_clipboard_completion_releases_pending_close_and_keeps_text() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "kept", cx);
    with(compose, cx, |compose, window, cx| {
        compose.reading = 1;
        assert!(!compose.may_close(window, cx));
        compose.reading = 0;
        compose.finish_clipboard(Err("clipboard unavailable".into()), cx);
        assert_eq!(compose.document.read(cx).snapshot().to_plain(), "kept");
        compose.keep(cx);
    });
    pause(&mail, cx, 1);
    assert!(cx.update(|_, app| compose.read(app).is_ok()), "failed operation cancels close");
    assert_eq!(kept(&mail, cx)[0].unsent.body_text(), "kept");
}

#[test]
fn ime_preedit_is_never_autosaved_or_sent() {
    use gpui::EntityInputHandler;
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "committed", cx);
    with(compose, cx, |compose, window, cx| {
        compose.document.update(cx, |editor, cx| editor.replace_and_mark_text_in_range(None, "pinyin", None, window, cx));
        let out = compose.outgoing(&draft(&["ann@example.com"], "IME"), cx).unwrap();
        assert_eq!(out.text, "committed");
        assert!(!out.html.contains("pinyin"));
    });
    pause(&mail, cx, 1);
    assert_eq!(kept(&mail, cx)[0].unsent.body_text(), "committed");
}

#[test]
fn body_edits_with_same_text_length_remain_dirty_during_background_save() {
    let dir = tempfile::tempdir().unwrap();
    seeded(dir.path(), format!("{NOTE}\r\nx").as_bytes(), &[]);
    let mut cx = TestAppContext::single();
    let (mail, cx) = window(&mut cx, dir.path(), Keys::default());
    let compose = new_message(&mail, cx);
    write(compose, "first", cx);
    with(compose, cx, |compose, _, cx| compose.keep(cx));
    write(compose, "other", cx);
    with(compose, cx, |compose, _, cx| compose.keep(cx));
    settle(cx, |_| true);
    pause(&mail, cx, 1);
    settle(cx, |app| {
        let store = mail.read(app).store.clone().unwrap();
        store.lock().unwrap().drafts().unwrap()[0].unsent.body_text() == "other"
    });
    assert_eq!(kept(&mail, cx)[0].unsent.body_text(), "other");
}

#[test]
fn legacy_task_completion_survives_native_import_save_and_reopen() {
    use ely_gpui_component::documents::email::{EmailBlockKind, EmailDocument, EmailParagraphKind};
    let html = ely_mail_engine::compose::legacy_draft_html("- [x] done\n- [ ] pending", true).unwrap();
    let imported = EmailDocument::from_html(&html).unwrap();
    let tasks = |document: &EmailDocument| document.blocks().iter().filter_map(|block| {
        match &block.kind {
            EmailBlockKind::Paragraph(paragraph) if !paragraph.content.text.is_empty() => Some((paragraph.kind.clone(), paragraph.content.text.clone())),
            _ => None,
        }
    }).collect::<Vec<_>>();
    let expected = vec![(EmailParagraphKind::Task(true), "done".into()), (EmailParagraphKind::Task(false), "pending".into())];
    assert_eq!(tasks(&imported), expected);
    let mut saved = super::outbox::unsent("");
    saved.html = Some(imported.to_html());
    let bytes = serde_json::to_vec(&saved).unwrap();
    let reopened: ely_mail_engine::Unsent = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(tasks(&EmailDocument::from_html(reopened.html.as_deref().unwrap()).unwrap()), expected);
}
