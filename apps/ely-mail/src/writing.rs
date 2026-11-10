//! Opening a message to write, and handing it over (C12.12).

use std::path::PathBuf;

use ely_gpui_component::documents::email::{EmailBlockKind, EmailDocument, EmailError, EmailImage, EmailParagraph, EmailText};
use ely_mail_engine::{
    AccountId, Draft, Error, Identity, Letter, Mail as Message, Mailbox, MessageId, Part, Result,
    Store, build,
    compose::{self, Earlier, Start, Writing},
    read,
};
use ely_platform::AppId;
use gpui::{App, AppContext as _, Context, WeakEntity, Window, WindowHandle, px, size};

use crate::{
    compose::{Compose, Reopened},
    mail::{Mail, text},
};

const EDIT_AS_NEW_INCOMPLETE: &str = "download the complete message and all attachments before editing as new";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    Reply,
    ReplyAll,
    Forward,
    EditAsNew,
}

/// An account one may send from.
#[derive(Clone)]
pub struct Sender {
    pub id: AccountId,
    pub address: String,
    pub identity: Identity,
    /// The outgoing server's last stated size limit.
    pub limit: Option<u64>,
}

/// What a new message starts from.
pub struct Opening {
    pub senders: Vec<Sender>,
    pub from: AccountId,
    pub answered: Option<Answered>,
    pub draft: Option<Reopened>,
    /// A mail link's addresses, subject and body.
    pub asked: Option<ely_mail_engine::mailto::Asked>,
}

/// The message answered or forwarded.
pub struct Answered {
    pub how: How,
    pub mail: Message,
    pub start: Start,
    /// Its words, empty when only its headers are kept.
    pub text: String,
    pub whole: bool,
    /// Forwarded files that are here.
    pub files: Vec<Part>,
    /// Forwarded files not downloaded, left out.
    pub missing: usize,
    /// Edit-as-new content prepared and validated by the background read.
    pub document: Option<EmailDocument>,
}

/// Everything a message is built from.
pub struct Outgoing {
    pub from: Mailbox,
    pub to: Vec<Mailbox>,
    pub cc: Vec<Mailbox>,
    pub bcc: Vec<Mailbox>,
    pub subject: String,
    /// Plain alternative serialized by the native HTML editor.
    pub text: String,
    /// HTML serialized by the native HTML editor.
    pub html: String,
    pub signature: String,
    pub before: Option<Earlier>,
    pub attachments: Vec<Part>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub date: i64,
}

/// Bcc stays in it until it is sent.
pub fn prepare(out: &Outgoing) -> Result<Letter> {
    let bodies = compose::bodies(&Writing {
        text: &out.text,
        html: &out.html,
        signature: &out.signature,
        earlier: out.before.as_ref(),
    })?;
    let built = build(&Draft {
        from: out.from.clone(),
        to: out.to.clone(),
        cc: out.cc.clone(),
        bcc: out.bcc.clone(),
        subject: out.subject.clone(),
        text: bodies.text,
        html: bodies.html,
        attachments: out.attachments.clone(),
        in_reply_to: out.in_reply_to.clone(),
        references: out.references.clone(),
        date: out.date,
    })?;
    let mut envelope: Vec<String> = Vec::new();
    for person in out.to.iter().chain(&out.cc).chain(&out.bcc) {
        if !envelope
            .iter()
            .any(|kept| kept.eq_ignore_ascii_case(&person.email))
        {
            envelope.push(person.email.clone());
        }
    }
    Ok(Letter {
        envelope,
        message_id: built.message_id,
        bytes: built.bytes,
    })
}

pub fn senders(store: &Store) -> Result<Vec<Sender>> {
    store
        .accounts()?
        .into_iter()
        .map(|(id, account)| {
            Ok(Sender {
                id,
                address: account.address,
                identity: store.identity(id)?,
                limit: store.smtp_limit(id)?,
            })
        })
        .collect()
}

pub(super) fn answered(store: &Store, id: MessageId, how: How) -> Result<Opening> {
    let senders = senders(store)?;
    let stored = store.get(id)?;
    let from = store.account_of(stored.mailbox)?;
    let me = senders
        .iter()
        .find(|sender| sender.id == from)
        .map(|sender| sender.address.clone())
        .ok_or_else(|| Error::Store(format!("account {} is not kept", from.0)))?;
    let (bytes, whole) = match store.original(id)? {
        Some(raw) => (raw, true),
        None => (store.head(id)?, false),
    };
    if how == How::EditAsNew && (!whole || !stored.missing.is_empty()) {
        return Err(Error::Invalid(EDIT_AS_NEW_INCOMPLETE.into()));
    }
    let mail = read(&bytes)?;
    let start = match how {
        How::Reply => compose::answer(&mail, &me, false),
        How::ReplyAll => compose::answer(&mail, &me, true),
        How::Forward => compose::forward(&mail),
        How::EditAsNew => Start {
            to: mail.to.clone(),
            cc: mail.cc.clone(),
            subject: mail.subject.clone().unwrap_or_default(),
            in_reply_to: None,
            references: Vec::new(),
        },
    };
    let text = if whole && how != How::EditAsNew {
        compose::original(&mail)
    } else {
        String::new()
    };
    let (files, missing) = match how {
        How::Forward => {
            let waiting = |part: &Part| {
                part.section
                    .as_ref()
                    .is_some_and(|section| stored.missing.contains(section))
            };
            let (gone, here): (Vec<Part>, Vec<Part>) =
                mail.attachments.iter().cloned().partition(waiting);
            (here, gone.len())
        }
        How::EditAsNew => {
            let inline = mail.inline.iter().filter(|part| {
                // Body alternatives with Content-ID are not attachments. Other
                // resources remain downloadable even when they cannot be drawn.
                let kind = part.content_type.split(';').next().unwrap_or_default().trim();
                !kind.eq_ignore_ascii_case("text/html") && !kind.eq_ignore_ascii_case("text/plain")
                    && !kind.to_ascii_lowercase().starts_with("multipart/")
            });
            (mail.attachments.iter().chain(inline).cloned().collect(), 0)
        }
        _ => (Vec::new(), 0),
    };
    let document = if how == How::EditAsNew {
        Some(editable_copy(&mail, &files).map_err(|error| Error::Invalid(error.to_string()))?)
    } else { None };
    Ok(Opening {
        senders,
        from,
        answered: Some(Answered {
            how,
            mail,
            start,
            text,
            whole,
            files,
            missing,
            document,
        }),
        draft: None,
        asked: None,
    })
}

/// Import a copy without dangling CID references or mutating any original part.
fn editable_copy(mail: &Message, files: &[Part]) -> std::result::Result<EmailDocument, EmailError> {
    let mut document = match &mail.html {
        Some(html) => EmailDocument::from_html(html)?,
        None => return EmailDocument::from_plain(mail.text.as_deref().unwrap_or_default()),
    };
    let valid: std::collections::HashSet<&str> = files.iter().filter_map(|part| {
        crate::letter::raster(&part.content_type, &part.bytes).ok()?;
        part.content_id.as_deref().map(|id| id.trim_matches(['<', '>']))
    }).collect();
    let canonical = |image: &EmailImage| -> Option<String> {
        let Some(cid) = image.source.strip_prefix("cid:") else { return Some(image.source.clone()); };
        // Prefer the literal ID when '%' is genuinely part of an address.
        if valid.contains(cid) { return Some(format!("cid:{cid}")); }
        let decoded = percent_encoding::percent_decode_str(cid).decode_utf8().ok()?;
        valid.contains(decoded.as_ref()).then(|| format!("cid:{decoded}"))
    };
    // Replace only missing/unsafe pictures with their original alternative text,
    // including inline objects in table cells, preserving unaffected formatting.
    let repair_text = |text: &mut EmailText| -> std::result::Result<(), EmailError> {
        let missing: Vec<_> = text.images.iter().filter(|(_, image)| canonical(image).is_none())
            .map(|(at, image)| (*at, image.alt.clone())).collect();
        for (at, alt) in missing.into_iter().rev() {
            let style = text.style_at(at);
            text.replace(at..at + '\u{fffc}'.len_utf8(), &alt, &style)?;
        }
        Ok(())
    };
    let mut blocks = document.blocks().to_vec();
    for block in &mut blocks {
        match &mut block.kind {
            EmailBlockKind::Image(image) if canonical(image).is_none() => {
                block.kind = EmailBlockKind::Paragraph(EmailParagraph {
                    align: image.align, content: EmailText::plain(&image.alt), ..Default::default()
                });
            }
            EmailBlockKind::Paragraph(paragraph) => repair_text(&mut paragraph.content)?,
            EmailBlockKind::Table(table) => for cell in &mut table.cells { repair_text(cell)?; },
            _ => {}
        }
    }
    document = EmailDocument::from_blocks(blocks)?;
    document.try_map_images(|image| {
        if let Some(source) = canonical(image) { image.source = source; }
        Ok(())
    })?;
    Ok(document)
}

/// Quit and closing the main window ask each message first.
pub fn guard_leaving(mail: WeakEntity<Mail>, window: &mut Window, cx: &mut App) {
    let quitting = mail.clone();
    ely_ui::commands::route_quit(cx, move |cx| {
        if may_leave(&quitting, cx) {
            cx.quit();
        }
    });
    // A kept window closes to the tray; only quitting asks.
    ely_gpui_component::shell::guard_close(window, cx, move |_, cx| ely_ui::tray::kept(cx) || may_leave(&mail, cx));
}

/// True when every message, then the Outbox, lets it go.
fn may_leave(mail: &WeakEntity<Mail>, cx: &mut App) -> bool {
    for each in cx.windows() {
        let Some(compose) = each.downcast::<Compose>() else {
            continue;
        };
        let leaves = compose.update(cx, |compose, window, cx| compose.may_leave(window, cx));
        match leaves {
            Ok(true) => {}
            Ok(false) => {
                log::info!("mail: a message holds the quit");
                return false;
            }
            Err(error) => log::warn!("mail: a closing message did not answer: {error}"),
        }
    }
    mail.update(cx, |mail, cx| mail.may_quit(cx))
        .unwrap_or(true)
}

impl Mail {
    /// A blank message from the open account, else the first.
    pub fn write_new(&mut self, cx: &mut Context<Self>) {
        self.write_with(Vec::new(), cx);
    }

    /// A new message carrying `files` (§16); it never sends itself.
    pub fn write_with(&mut self, files: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.write(files, None, cx);
    }

    /// A new message filled from a mail link.
    pub fn write_asked(&mut self, asked: ely_mail_engine::mailto::Asked, cx: &mut Context<Self>) {
        self.write(Vec::new(), Some(asked), cx);
    }

    fn write(&mut self, files: Vec<PathBuf>, asked: Option<ely_mail_engine::mailto::Asked>, cx: &mut Context<Self>) {
        let preferred = self.open.account();
        self.with_store(
            cx,
            |store| senders(store),
            move |mail, read, cx| match read {
                Ok(senders) if senders.is_empty() => {
                    mail.toast(
                        text(cx, "mail.write.no_account", &[]),
                        ely_gpui_component::primitives::Severity::Info,
                        cx,
                    );
                }
                Ok(senders) => {
                    let from = preferred
                        .filter(|id| senders.iter().any(|sender| sender.id == *id))
                        .unwrap_or(senders[0].id);
                    let opened = mail.open_compose(
                        Opening {
                            senders,
                            from,
                            answered: None,
                            draft: None,
                            asked,
                        },
                        cx,
                    );
                    if let Some(window) = opened.filter(|_| !files.is_empty()) {
                        window.update(cx, |compose, _, cx| compose.attach(files, cx)).ok();
                    }
                }
                Err(error) => mail.failed("mail.write.unread", &error, cx),
            },
        );
    }

    pub fn answer(&mut self, id: MessageId, how: How, cx: &mut Context<Self>) {
        log::info!("mail: {how:?} to message {}", id.0);
        self.with_store(
            cx,
            move |store| answered(store, id, how),
            move |mail, read, cx| match read {
                Ok(opening) => {
                    mail.open_compose(opening, cx);
                }
                Err(error) if how == How::EditAsNew && error == Error::Invalid(EDIT_AS_NEW_INCOMPLETE.into()).to_string() => {
                    mail.toast(text(cx, "mail.write.edit_as_new_incomplete", &[]), ely_gpui_component::primitives::Severity::Warning, cx);
                }
                Err(error) => mail.failed("mail.write.unread", &error, cx),
            },
        );
    }

    pub fn open_compose(&mut self, opening: Opening, cx: &mut Context<Self>) -> Option<WindowHandle<Compose>> {
        let mut options = ely_ui::window_options(AppId::Mail, size(px(760.0), px(680.0)), cx);
        let subject = match (&opening.draft, &opening.answered) {
            (Some(draft), _) => Some(&draft.unsent.subject),
            (None, Some(answered)) => Some(&answered.start.subject),
            (None, None) => None,
        };
        let title = match subject {
            Some(subject) if !subject.is_empty() => subject.clone().into(),
            _ => text(cx, "mail.write.title", &[]),
        };
        if let Some(bar) = options.titlebar.as_mut() {
            bar.title = Some(title);
        }
        let this = cx.weak_entity();
        let opened = cx.open_window(options, |window, cx| {
            cx.new(|cx| Compose::new(this, opening, window, cx))
        });
        match opened {
            Ok(window) => {
                log::info!("mail: a compose window opened");
                self.offer_contacts(window, cx);
                Some(window)
            }
            Err(error) => {
                self.failed("mail.write.unopened", &format!("{error:#}"), cx);
                None
            }
        }
    }
}
