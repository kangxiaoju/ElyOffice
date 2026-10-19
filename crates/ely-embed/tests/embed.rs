use std::{path::{Path, PathBuf}, time::{Duration, Instant}};

use ely_embed::{Access, DocumentState, OfficeKind, OfficeRuntime, RuntimePaths};
use gpui::Entity;

/// The LOK worker binary, resolved the same way `ely-lok` does.
fn worker_bin() -> Option<PathBuf> {
    if let Ok(bin) = std::env::var("ELY_LOK_WORKER") {
        let path = PathBuf::from(bin);
        return path.try_exists().ok().unwrap_or(false).then_some(path);
    }
    let name = if cfg!(windows) { "ely-lok-worker.exe" } else { "ely-lok-worker" };
    for profile in ["debug", "release"] {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(profile)
            .join(name);
        if path.try_exists().ok().unwrap_or(false) {
            return Some(path);
        }
    }
    None
}

/// The pinned LOK install and its worker, or None when the build is absent.
fn lok() -> Option<(PathBuf, PathBuf)> {
    let install = ely_lok::install_dir().ok()?;
    let program = worker_bin()?;
    Some((install, program))
}

/// Copies a fixture beside the profile and returns its path.
fn fixture(dir: &Path, name: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let target = dir.join(name);
    std::fs::copy(&source, &target).expect("the fixture copies");
    target
}

/// Steps the scheduler until the view's queued inputs reach the worker.
fn settle(cx: &gpui::TestAppContext, surface: &Entity<ely_lok::LokView>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if !cx.executor().tick() {
            cx.executor().advance_clock(Duration::from_millis(20));
        }
        if cx.update(|app| surface.read(app).settled()) {
            return;
        }
    }
}

#[test]
fn public_protocol_describes_an_explicit_runtime() {
    let paths = RuntimePaths {
        lok_program: "lok-worker".into(),
        lok_install: "libreoffice".into(),
        lok_profile: "profile".into(),
        pdf_worker: "pdf-worker".into(),
    };
    assert_eq!(paths.lok_program.to_string_lossy(), "lok-worker");
    assert_eq!(Access::ReadOnly, Access::ReadOnly);
    assert_eq!(OfficeKind::Writer, OfficeKind::Writer);
    assert_eq!(DocumentState::Closed, DocumentState::Closed);
}

#[test]
fn session_lifecycle_tracks_edits_saves_and_worker_end() {
    let Some((install, program)) = lok() else {
        eprintln!("SKIP: no pinned LOK build");
        return;
    };
    let dir = tempfile::tempdir().expect("a temp dir");
    let note = fixture(dir.path(), "note.fodt");
    let deck = fixture(dir.path(), "deck.fodp");
    let runtime = OfficeRuntime::spawn(RuntimePaths {
        lok_program: program,
        lok_install: install,
        lok_profile: dir.path().join("profile"),
        pdf_worker: PathBuf::new(),
    })
    .expect("the runtime spawns");

    let cx = gpui::TestAppContext::single();
    cx.update(|app| {
        // A Writer session: open clean, edit dirty, save ready, close closed.
        let mut writer = runtime
            .open(OfficeKind::Writer, &note, Access::Editable, app)
            .expect("the writer opens");
        assert_eq!(writer.state(app), DocumentState::Ready, "a fresh session is ready");
        let surface = writer.surface();
        let edited = surface.read(app).edited();
        surface.update(app, |view, _| view.insert("hello"));
        settle(&cx, &surface);
        assert!(surface.read(app).edited() > edited, "an edit lands on the view");
        assert_eq!(writer.state(app), DocumentState::Dirty, "an edit dirties the session");
        writer.save(app).expect("the writer saves");
        assert_eq!(writer.state(app), DocumentState::Ready, "a save returns to ready");
        // A Writer session must refuse to save a deck extension.
        assert!(
            writer.save_as(dir.path().join("as-deck.fodp"), app).is_err(),
            "a writer refuses to save_as a presentation format"
        );
        writer.close().expect("the writer closes");
        assert_eq!(writer.state(app), DocumentState::Closed, "a closed session is closed");
        // Saving a closed session fails without marking it failed.
        assert!(writer.save(app).is_err(), "a closed session cannot save");
        assert_eq!(writer.state(app), DocumentState::Closed, "and stays closed, not failed");

        // An Impress session: open clean, then the worker ends underneath it.
        let impress = runtime
            .open(OfficeKind::Presentation, &deck, Access::Editable, app)
            .expect("the deck opens");
        assert_eq!(impress.state(app), DocumentState::Ready, "the deck opens ready");
        assert!(impress.surface().read(app).pages().len() >= 1, "the deck has a surface with slides");
        runtime.close().expect("the worker shuts down");
        assert_eq!(
            impress.state(app),
            DocumentState::WorkerEnded,
            "a session over an ended worker reports it"
        );
    });
}
