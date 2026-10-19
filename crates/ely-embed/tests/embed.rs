use ely_embed::{Access, DocumentState, OfficeKind, RuntimePaths};

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
