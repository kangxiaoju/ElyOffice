use super::super::input::{Input, accepts};

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
