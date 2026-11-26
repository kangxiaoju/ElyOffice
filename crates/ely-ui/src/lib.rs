pub mod commands;
#[cfg(any(test, feature = "test-support"))]
mod ending;
pub mod i18n;
#[cfg(any(test, feature = "test-support"))]
pub mod laid;
#[cfg(any(test, feature = "test-support"))]
pub mod logs;
pub mod menu;
mod outward;
mod dropping;
mod frames;
mod opening;
mod pick;
mod run;
pub mod windows;
pub mod settings;
mod shell;
mod stalls;
pub mod tasks;
mod theme;
pub mod tray;
pub mod updates;
pub mod watching;
pub mod wheel;

#[cfg(any(test, feature = "test-support"))]
pub use ending::at_thread_end;
pub use outward::outward;
pub use dropping::open_dropped;
pub use opening::{Ask, Opened, ask_again, launch_files, mail_link, on_ask, on_open};
#[cfg(any(test, feature = "test-support"))]
pub use opening::install_no_launch_files;
#[cfg(any(test, feature = "test-support"))]
pub use theme::install as install_theme;
pub use run::{run, window_options};
pub use shell::{Root, Shell, blank, document_root, root, set_document_menus, tool_root};
pub use tasks::{LongTask, TaskState};

struct Embedded;
impl gpui::Global for Embedded {}

/// Installs Office catalogs and settings without taking over the host lifecycle.
pub fn embed_init(app: ely_platform::AppId, cx: &mut gpui::App) {
    if cx.has_global::<Embedded>() { return; }
    settings::install(app, cx);
    i18n::install(cx);
    commands::bind_keys(cx);
    cx.set_global(Embedded);
}
