//! The Message, Account and Sync menus.

use gpui::{App, Menu, MenuItem, actions};

use crate::mail::text;

actions!(mail, [AddAccount, RemoveAccount, SyncAll, NewMessage, OpenMessageFile]);

pub const CONTEXT: &str = "mail";

pub fn menus(cx: &App) -> Vec<Menu> {
    vec![
        Menu::new(text(cx, "mail.menu.message", &[])).items([
            MenuItem::action(text(cx, "mail.write.new", &[]), NewMessage),
            MenuItem::action(text(cx, "mail.eml.open", &[]), OpenMessageFile),
        ]),
        Menu::new(text(cx, "mail.menu.account", &[])).items([
            MenuItem::action(text(cx, "mail.add.open", &[]), AddAccount),
            MenuItem::action(text(cx, "mail.remove.open", &[]), RemoveAccount),
        ]),
        Menu::new(text(cx, "mail.menu.sync", &[]))
            .items([MenuItem::action(text(cx, "mail.sync.now", &[]), SyncAll)]),
    ]
}
