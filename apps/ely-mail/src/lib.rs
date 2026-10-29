//! ElyMail as a library: the window app (`main.rs`) and host apps that embed
//! the mail view in their own tab both build on these modules.
//!
//! Host apps call [`mail::embed`] for a chromeless [`mail::Mail`] view and
//! [`mail::Parts`] when they lay out the account store themselves. The binary
//! calls [`mail::open`] for the standalone window.

mod add_view;
mod adding;
mod boxes;
mod capacity;
mod compose;
mod contacts;
mod controls;
mod draft_list;
mod drafts;
mod eml;
mod files;
mod flow;
mod identity;
mod kinds;
mod letter;
pub mod mail;
mod menu;
mod migrate;
mod notices;
mod organize;
mod outbox;
mod outbox_list;
mod panes;
mod pictures;
mod reader;
mod reading;
mod removing;
mod rules;
mod seeking;
mod side;
mod storage;
mod syncing;
mod view;
mod writing;

#[cfg(test)]
mod tests;

pub use mail::{Mail, Parts, embed, open};
