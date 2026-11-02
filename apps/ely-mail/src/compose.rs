//! Native HTML email composition with decode-only legacy draft migration.

use std::path::PathBuf;

use ely_gpui_component::{
    documents::email::{EmailDocument, EmailEditorEvent, HtmlEmailEditor},
    mail::Draft,
    primitives::Severity,
};
use ely_mail_engine::{
    AccountId, Mailbox, Part,
    compose::{self, Earlier},
};
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Entity, FocusHandle, PathPromptOptions,
    SharedString, WeakEntity, Window,
};

mod keeping;
mod lead;
mod page;

mod clipboard;
#[cfg(test)]
pub use keeping::PAUSE;
pub use keeping::Shelf;

use crate::{
    mail::{Mail, text},
    writing::{Opening, Outgoing, Sender},
};

/// An attached file and its key in the list.
pub struct File {
    pub key: u64,
    pub part: Part,
}

pub struct Compose {
    mail: WeakEntity<Mail>,
    pub senders: Vec<Sender>,
    pub from: AccountId,
    pub to: Vec<SharedString>,
    pub cc: Vec<SharedString>,
    pub bcc: Vec<SharedString>,
    pub subject: SharedString,
    in_reply_to: Option<String>,
    references: Vec<String>,
    pub before: Option<Earlier>,
    /// Only the answered message's headers are kept.
    pub headers_only: bool,
    pub document: Entity<HtmlEmailEditor>,
    import_failed: bool,
    pub files: Vec<File>,
    next_file: u64,
    /// Files picked and not yet read.
    pub reading: usize,
    /// Forwarded files not downloaded.
    pub missing: usize,
    /// Contacts offered as addresses.
    pub offered: Vec<ely_gpui_component::mail::Contact>,
    /// What it opened with, or last sent.
    base: Carried,
    /// The addresses and subject as now written.
    written: Draft,
    /// What the send under way carries.
    in_flight: Option<Carried>,
    /// Sent, waiting for a draft write to land.
    to_queue: Option<Frozen>,
    pub sending: bool,
    pub problem: Option<(Severity, SharedString)>,
    pub discarding: bool,
    pub shelf: Shelf,
    window: AnyWindowHandle,
    focus: FocusHandle,
}

/// A file's shown name; some have none.
pub fn file_name(part: &Part, cx: &App) -> SharedString {
    match &part.name {
        Some(name) => name.clone().into(),
        None => text(cx, "mail.write.unnamed", &[]),
    }
}

fn addresses(list: &[SharedString]) -> Vec<Mailbox> {
    list.iter()
        .map(|email| Mailbox {
            name: None,
            email: email.to_string(),
        })
        .collect()
}

impl Compose {
    pub fn new(
        mail: WeakEntity<Mail>,
        opening: Opening,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let imported = match (&opening.draft, &opening.asked) {
            (Some(kept), _) => match &kept.unsent.html {
                Some(html) => EmailDocument::from_html(html).map_err(|error| error.to_string()),
                None => compose::legacy_draft_html(&kept.unsent.words, kept.unsent.markdown)
                    .map_err(|error| error.to_string())
                    .and_then(|html| EmailDocument::from_html(&html).map_err(|error| error.to_string())),
            },
            (None, Some(asked)) => EmailDocument::from_plain(&asked.body).map_err(|error| error.to_string()),
            _ => Ok(EmailDocument::default()),
        };
        let import_problem = imported.as_ref().err().cloned();
        let document = cx.new(|cx| {
            let mut editor = HtmlEmailEditor::new(imported.unwrap_or_default(), window, cx);
            editor.set_host_clipboard(true);
            if let Some(error) = &import_problem {
                editor.set_problem(Some(error.clone()), cx);
                editor.set_read_only(true, cx);
            }
            editor
        });
        cx.subscribe(&document, |compose, _, event, cx| match event {
            EmailEditorEvent::Changed => compose.touched(cx),
            EmailEditorEvent::PasteRequested { request } => compose.paste(*request, cx),
            EmailEditorEvent::CopyRequested { html, text, cut } => compose.copy(html.clone(), text.clone(), *cut, cx),
            EmailEditorEvent::InsertImageRequested { request } => compose.pick_image(*request, cx),
            EmailEditorEvent::Problem(error) => compose.editor_problem(error.clone(), cx),
        }).detach();
        let this = cx.weak_entity();
        ely_gpui_component::shell::guard_close(window, cx, move |window, cx| {
            this.update(cx, |compose, cx| compose.may_close(window, cx))
                .unwrap_or(true)
        });
        let mut compose = Compose {
            mail,
            senders: opening.senders,
            from: opening.from,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: SharedString::default(),
            in_reply_to: None,
            references: Vec::new(),
            before: None,
            headers_only: false,
            document,
            import_failed: import_problem.is_some(),
            files: Vec::new(),
            next_file: 0,
            reading: 0,
            missing: 0,
            offered: Vec::new(),
            base: Carried {
                draft: blank(),
                revision: 0,
                empty: true,
                files: Vec::new(),
                from: opening.from,
            },
            written: blank(),
            in_flight: None,
            to_queue: None,
            sending: false,
            problem: import_problem.map(|error| (Severity::Danger, error.into())),
            discarding: false,
            shelf: Shelf::default(),
            window: window.window_handle(),
            focus: cx.focus_handle(),
        };
        if let Some(draft) = opening.draft {
            compose.restore(draft);
        }
        if let Some(answered) = opening.answered {
            let start = answered.start;
            compose.to = start.to.iter().map(|to| to.email.clone().into()).collect();
            compose.cc = start.cc.iter().map(|cc| cc.email.clone().into()).collect();
            compose.subject = start.subject.into();
            compose.in_reply_to = start.in_reply_to;
            compose.references = start.references;
            compose.headers_only = !answered.whole;
            compose.missing = answered.missing;
            for part in answered.files {
                compose.add_file(part);
            }
            if answered.whole {
                let earlier = lead::lead(answered.how, &answered.mail, answered.text, cx);
                compose.before = Some(earlier);
            }
        }
        if let Some(asked) = opening.asked {
            let addresses = |list: Vec<String>| list.into_iter().map(Into::into).collect();
            compose.to = addresses(asked.to);
            compose.cc = addresses(asked.cc);
            compose.bcc = addresses(asked.bcc);
            compose.subject = asked.subject.into();
        }
        compose.written = Draft {
            to: compose.to.clone(),
            cc: compose.cc.clone(),
            bcc: compose.bcc.clone(),
            subject: compose.subject.trim().to_owned().into(),
        };
        compose.sync_images(cx);
        compose.base = compose.carried(cx);
        if let Some(id) = compose.shelf.id {
            compose.shelf = Shelf::reopened(id, compose.base.clone());
        }
        compose
    }

    /// Takes up a kept draft where it was left.
    fn restore(&mut self, draft: Reopened) {
        let addresses = |list: Vec<String>| list.into_iter().map(Into::into).collect();
        let unsent = draft.unsent;
        self.to = addresses(unsent.to);
        self.cc = addresses(unsent.cc);
        self.bcc = addresses(unsent.bcc);
        self.subject = unsent.subject.into();
        self.in_reply_to = unsent.in_reply_to;
        self.references = unsent.references;
        self.before = unsent.earlier;
        self.headers_only = unsent.headers_only;
        self.missing = unsent.missing;
        for part in draft.files {
            self.add_file(part);
        }
        self.shelf.id = Some(draft.id);
    }

    /// The composer's addresses and subject after an edit.
    pub fn edited(&mut self, draft: Draft, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        self.written = draft;
        self.touched(cx);
        cx.notify();
    }

    pub fn choose_sender(&mut self, from: AccountId, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        self.from = from;
        self.touched(cx);
        cx.notify();
    }

    fn sender(&self) -> Option<&Sender> {
        self.senders.iter().find(|sender| sender.id == self.from)
    }

    fn add_file(&mut self, part: Part) {
        self.next_file += 1;
        self.files.push(File {
            key: self.next_file,
            part,
        });
    }

    fn carried(&self, cx: &App) -> Carried {
        let editor = self.document.read(cx);
        Carried {
            draft: self.written.clone(),
            revision: editor.committed_revision(),
            empty: editor.committed_is_blank(),
            files: self.files.iter().map(|file| file.key).collect(),
            from: self.from,
        }
    }

    fn pick_files(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn(async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) => return log::info!("mail: the attach panel was cancelled"),
                Ok(Err(error)) => return log::error!("mail: the attach panel failed: {error}"),
                Err(error) => return log::error!("mail: the attach panel went away: {error}"),
            };
            this.update(cx, |compose, cx| compose.attach(paths, cx))
                .ok();
        })
        .detach();
    }

    /// Reads the files off the UI thread, then lists them.
    pub fn offer(&mut self, contacts: Vec<ely_mail_engine::Contact>, cx: &mut Context<Self>) {
        let named = |contact: ely_mail_engine::Contact| {
            ely_gpui_component::mail::Contact::new(
                contact.name.unwrap_or_default(),
                contact.address,
            )
        };
        self.offered = contacts.into_iter().map(named).collect();
        cx.notify();
    }

    pub fn attach(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.import_failed { return; }
        let count = paths.len();
        self.reading += count;
        cx.notify();
        let read = cx.background_spawn(async move {
            paths
                .into_iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .ok_or_else(|| "a path names no file".to_owned())?
                        .to_string_lossy()
                        .into_owned();
                    let bytes = std::fs::read(&path).map_err(|err| err.to_string())?;
                    Ok(compose::attachment(&name, bytes))
                })
                .collect::<Result<Vec<_>, String>>()
        });
        cx.spawn(async move |this, cx| {
            let read = read.await;
            this.update(cx, |compose, cx| {
                compose.reading -= count;
                match read {
                    Ok(parts) => {
                        log::info!("mail: {} files attached", parts.len());
                        parts.into_iter().for_each(|part| compose.add_file(part));
                        compose.touched(cx);
                    }
                    Err(error) => {
                        log::warn!("mail: a file was not attached");
                        let words = text(cx, "mail.write.attach_failed", &[("error", &error)]);
                        compose.problem = Some((Severity::Danger, words));
                        compose.unread(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    #[cfg(test)]
    pub fn outgoing(&self, draft: &Draft, cx: &App) -> Option<Outgoing> {
        self.body_job(draft, cx)?.encode().ok().map(|(_, out)| out)
    }

    fn body_job(&self, draft: &Draft, cx: &App) -> Option<BodyJob> {
        let sender = self.sender()?;
        if self.import_failed { return None; }
        let mut unsent = self.unsent_headers();
        unsent.to = draft.to.iter().map(ToString::to_string).collect();
        unsent.cc = draft.cc.iter().map(ToString::to_string).collect();
        unsent.bcc = draft.bcc.iter().map(ToString::to_string).collect();
        unsent.subject = draft.subject.to_string();
        let out = Outgoing {
            from: Mailbox {
                name: sender.identity.name.clone(),
                email: sender.address.clone(),
            },
            to: addresses(&draft.to),
            cc: addresses(&draft.cc),
            bcc: addresses(&draft.bcc),
            subject: draft.subject.to_string(),
            text: String::new(),
            html: String::new(),
            signature: sender.identity.signature.clone(),
            before: self.before.clone(),
            attachments: self.files.iter().map(|file| file.part.clone()).collect(),
            in_reply_to: self.in_reply_to.clone(),
            references: self.references.clone(),
            date: jiff::Timestamp::now().as_second(),
        };
        Some(BodyJob { document: self.document.read(cx).snapshot(), unsent, out })
    }

    /// Hands it to the Outbox; the queue sends it.
    pub fn send(&mut self, draft: &Draft, _: &mut Window, cx: &mut Context<Self>) {
        if self.sending {
            return;
        }
        if self.reading > 0 {
            return log::info!("mail: send waits for {} files", self.reading);
        }
        self.begin_sending(cx);
        let Some(job) = self.body_job(draft, cx) else {
            log::error!("mail: the sender {} is not listed", self.from.0);
            return self.queued(Err("the sender is not listed".into()), cx);
        };
        log::info!(
            "mail: queueing for {} addresses",
            draft.to.len() + draft.cc.len()
        );
        let from = self.from;
        let encoded = cx.background_spawn(async move { job.encode() });
        cx.spawn(async move |this, cx| {
            let encoded = encoded.await;
            this.update(cx, |compose, cx| match encoded {
                Ok((unsent, out)) => {
                    compose.to_queue = Some(Frozen { from, unsent, out });
                    compose.queue_now(cx);
                }
                Err(error) => compose.queued(Err(error), cx),
            }).ok();
        }).detach();
    }

    /// Queues once no draft write is under way.
    fn queue_now(&mut self, cx: &mut Context<Self>) {
        if self.shelf_busy() {
            return log::info!("mail: send waits for its draft write");
        }
        let Some(Frozen { from, unsent, out }) = self.to_queue.take() else {
            return log::error!("mail: queueing with nothing sent");
        };
        let (held, this) = (self.shelf.id, cx.weak_entity());
        let asked = self.mail.update(cx, |mail, cx| {
            mail.queue_message(from, held, unsent, out, this, cx)
        });
        if asked.is_err() {
            self.queued(Err("the mail window is closed".into()), cx);
        }
    }

    pub fn begin_sending(&mut self, cx: &mut Context<Self>) {
        self.sending = true;
        self.in_flight = Some(self.carried(cx));
        self.problem = None;
        cx.notify();
    }

    /// In the Outbox, it closes; else it stays to retry.
    pub fn queued(&mut self, done: Result<(), String>, cx: &mut Context<Self>) {
        self.sending = false;
        let carried = self.in_flight.take();
        match done {
            Ok(()) => {
                self.handed_over();
                self.after_sending(carried, cx);
            }
            Err(error) => {
                log::warn!("mail: a message was not queued");
                let words = text(cx, "mail.outbox.unqueued", &[("error", &error)]);
                self.problem = Some((Severity::Danger, words));
                self.next(cx);
            }
        }
    }
}

impl Compose {
    /// Closes, unless edits came after what was sent.
    fn after_sending(&mut self, carried: Option<Carried>, cx: &mut Context<Self>) {
        let Some(carried) = carried else {
            log::error!("mail: a send ended that was never started");
            return;
        };
        if self.reading == 0 && carried == self.carried(cx) {
            return self.sent_off(true, cx);
        }
        log::info!("mail: edits made while sending stay, unsent");
        self.base = carried;
        self.problem = Some((Severity::Info, text(cx, "mail.write.sent_later", &[])));
        self.sent_off(false, cx);
        cx.notify();
    }
}

/// A send, fixed as it was pressed.
struct Frozen {
    from: AccountId,
    unsent: ely_mail_engine::Unsent,
    out: Outgoing,
}

struct BodyJob {
    document: EmailDocument,
    unsent: ely_mail_engine::Unsent,
    out: Outgoing,
}

impl BodyJob {
    fn encode(mut self) -> Result<(ely_mail_engine::Unsent, Outgoing), String> {
        let html = self.document.to_html();
        EmailDocument::from_html(&html).map_err(|error| error.to_string())?;
        for image in self.document.images() {
            if let Some(cid) = image.source.strip_prefix("cid:") {
                if !self.out.attachments.iter().any(|part| part.content_id.as_deref().is_some_and(|id| id.trim_matches(['<', '>']) == cid)) {
                    return Err("An inline picture has no attachment".into());
                }
            }
        }
        self.out.text = self.document.to_plain();
        self.unsent.html = Some(html.clone());
        self.out.html = html;
        Ok((self.unsent, self.out))
    }
}

/// What one send carried.
#[derive(Clone, PartialEq)]
pub struct Carried {
    draft: Draft,
    revision: u64,
    empty: bool,
    files: Vec<u64>,
    from: AccountId,
}

impl Carried {
    /// The same message, but for outer spaces.
    pub(super) fn blank_as(&self, base: &Carried) -> bool {
        (self.revision == base.revision || (self.empty && base.empty)) && self.draft == base.draft && self.files == base.files && self.from == base.from
    }
}

/// A draft from the store, to write on.
pub struct Reopened {
    pub id: ely_mail_engine::DraftId,
    pub unsent: ely_mail_engine::Unsent,
    pub files: Vec<Part>,
}

fn blank() -> Draft {
    Draft {
        to: Vec::new(),
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: SharedString::default(),
    }
}
