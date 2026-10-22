use std::sync::{Arc, Mutex};

use super::super::input::{Input, accepts};
use crate::Doc;

#[test]
fn read_only_rejects_edits_before_the_queue() {
    assert!(Input::Text { commit: true, text: "x".into() }.edits());
    assert!(Input::Paste.edits());
    assert!(Input::Copy { cut: true }.edits());
    assert!(Input::Drag { from: (0, 0), to: (1, 1) }.edits());
    assert!(Input::Key { char_code: 'x' as u32, key_code: 512 }.edits());
    assert!(!Input::Key { char_code: 0, key_code: 1027 }.edits());
    assert!(!Input::Copy { cut: false }.edits());
    assert!(!Input::Part(1).edits());
    assert!(!accepts(true, &Input::Text { commit: true, text: "x".into() }));
    assert!(!accepts(true, &Input::Paste));
    assert!(!accepts(true, &Input::Copy { cut: true }));
    assert!(accepts(true, &Input::Copy { cut: false }));
    assert!(accepts(true, &Input::Key { char_code: 0, key_code: 1027 }));
    assert!(accepts(true, &Input::Part(1)));
    assert!(accepts(false, &Input::Paste));
}


#[test]
fn read_only_state_is_explicit() {
    let _setter: fn(&mut crate::LokView, bool) = crate::LokView::set_read_only;
    let _getter: fn(&crate::LokView) -> bool = crate::LokView::is_read_only;
}

/// Steps the scheduler until `done` or ten seconds pass.
fn settle(cx: &gpui::TestAppContext, done: impl Fn(&gpui::App) -> bool) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if !cx.executor().tick() {
            cx.executor().advance_clock(std::time::Duration::from_millis(20));
        }
        if cx.update(|app| done(app)) {
            return true;
        }
    }
    false
}

/// Two views of the same document: one editable, one to be made read-only.
fn two_views(
    app: &mut gpui::App,
    install: &std::path::Path,
) -> (gpui::Entity<crate::LokView>, gpui::Entity<crate::LokView>, Arc<Mutex<crate::Worker>>, Doc) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let mut worker = crate::Worker::spawn(install, &dir.path().join("profile")).expect("the worker");
    let source = dir.path().join("note.fodt");
    std::fs::write(&source, crate::client::tests::FODT).expect("the fixture writes");
    let doc = worker.load(&crate::file_url(&source)).expect("the document loads");
    std::mem::forget(dir);
    let shared = Arc::new(Mutex::new(worker));
    let laid =
        crate::layout(&mut shared.lock().expect("the worker lock"), &doc).expect("the layout reads");
    let editable = crate::LokView::open(Arc::clone(&shared), doc.clone(), laid.clone(), app);
    let read_only = crate::LokView::open(Arc::clone(&shared), doc.clone(), laid, app);
    (editable, read_only, shared, doc)
}

#[test]
fn read_only_view_rejects_every_edit_before_it_reaches_the_worker() {
    let Some(install) = crate::client::tests::install() else {
        eprintln!("SKIP: no pinned LOK build");
        return;
    };
    let cx = gpui::TestAppContext::single();
    cx.update(|app| {
        let (editable, read_only, shared, doc) = two_views(app, &install);
        read_only.update(app, |view, _| view.set_read_only(true));
        let revision = |shared: &Arc<Mutex<crate::Worker>>| {
            shared
                .lock()
                .expect("the worker lock")
                .now(&doc)
                .expect("the revision")
                .rev
        };
        let before = revision(&shared);

        // Every editing input is refused at the gate, before the worker.
        read_only.update(app, |view, _| {
            view.send(Input::Text { commit: true, text: "blocked".into() });
            view.send(Input::Paste);
            view.send(Input::Copy { cut: true });
            view.send(Input::Drag { from: (0, 0), to: (1, 1) });
            view.send(Input::Run { commands: vec![("edit.delete", Vec::new())], at: None });
        });
        assert_eq!(revision(&shared), before, "read-only edits never reach the worker");
        assert_eq!(read_only.read(app).edited(), 0, "no edit lands on the read-only view");

        // The same kind of edit reaches the worker through the editable view.
        editable.update(app, |view, _| {
            view.send(Input::Text { commit: true, text: "hello".into() });
        });
        assert!(
            settle(&cx, |_| revision(&shared) > before),
            "an editable view's edit bumps the revision"
        );
        assert!(editable.read(app).edited() > 0, "the editable view counts its edit");

        // Non-editing input still flows through the read-only view.
        let sent = read_only.read(app).sent();
        read_only.update(app, |view, _| {
            view.send(Input::Key { char_code: 0, key_code: 1027 });
            view.send(Input::Run { commands: vec![("edit.select_all", Vec::new())], at: None });
            view.send(Input::Copy { cut: false });
            view.send(Input::Visible((0, 0, 3840, 3840), (256, crate::TILE_TWIPS)));
            view.show_part(0);
        });
        let read = read_only.read(app);
        assert!(read.sent() > sent, "non-editing input still queues on the read-only view");
        assert!(
            settle(&cx, |app| read_only.read(app).settled()),
            "the read-only view processes its passthrough input"
        );
    });
}
