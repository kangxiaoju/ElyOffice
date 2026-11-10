//! What the window has open, and its list.

use std::{collections::HashMap, rc::Rc};

use ely_gpui_component::mail::Mail as Row;
use ely_mail_engine::{AccountId, Filter, MailboxId, MessageCursor, MessageId, Role, Scope, Search, Summary};
use gpui::{Context, SharedString};

use crate::{
    draft_list,
    mail::{Mail, text},
    outbox_list,
};

pub type Unread = HashMap<MailboxId, u32>;

/// What the middle of the window shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Open {
    Inboxes,
    /// Drafts kept here, of every account.
    Drafts,
    /// Messages handed over to send, of every account.
    Outbox,
    /// The people mail is written to.
    Contacts,
    Mailbox(AccountId, MailboxId),
    /// The account's settings and sync.
    Account(AccountId),
}

impl Open {
    pub fn key(self) -> SharedString {
        match self {
            Open::Inboxes => "inboxes".into(),
            Open::Drafts => "drafts".into(),
            Open::Outbox => "outbox".into(),
            Open::Contacts => "contacts".into(),
            Open::Mailbox(_, mailbox) => format!("box-{}", mailbox.0).into(),
            Open::Account(id) => format!("account-{}", id.0).into(),
        }
    }

    pub fn account(self) -> Option<AccountId> {
        match self {
            Open::Inboxes | Open::Drafts | Open::Outbox | Open::Contacts => None,
            Open::Mailbox(id, _) | Open::Account(id) => Some(id),
        }
    }

    fn scope(self) -> Option<Scope> {
        match self {
            Open::Inboxes => Some(Scope::Inboxes),
            Open::Mailbox(_, mailbox) => Some(Scope::Mailbox(mailbox)),
            Open::Drafts | Open::Outbox | Open::Contacts | Open::Account(_) => None,
        }
    }
}

/// Where a listed message is, and its state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fact {
    pub id: MessageId,
    pub account: AccountId,
    pub mailbox: MailboxId,
    pub role: Option<Role>,
    pub pop: bool,
    pub seen: bool,
    pub flagged: bool,
}

/// Rows read, and where each key sits.
pub struct Indexed {
    rows: Vec<Row>,
    at: HashMap<SharedString, usize>,
    facts: HashMap<SharedString, Fact>,
}

/// Off the UI thread: a long list takes a while.
pub fn index(rows: Vec<Row>) -> Indexed {
    let at = rows
        .iter()
        .enumerate()
        .map(|(ix, row)| (row.key.clone(), ix))
        .collect();
    Indexed {
        rows,
        at,
        facts: HashMap::new(),
    }
}

/// The open list and the messages picked in it.
#[derive(Default)]
pub struct Listing {
    /// Numbers reads; an older one is dropped.
    pub read: u64,
    query: Option<(Open, Filter, Option<Search>)>,
    pub loading: bool,
    pub has_more: bool,
    pub page_error: Option<String>,
    next: Option<MessageCursor>,
    /// The loaded date/id boundary, even when the last page has no continuation.
    tail: Option<MessageCursor>,
    /// None until read, or when nothing lists mail.
    pub mails: Option<Result<Rc<Vec<Row>>, String>>,
    at: HashMap<SharedString, usize>,
    /// Mail rows only; drafts and the Outbox have none.
    pub facts: HashMap<SharedString, Fact>,
    pub selected: Vec<SharedString>,
    /// The one message picked, for the reading pane.
    pub chosen: Option<Row>,
}

impl Listing {
    pub fn show(&mut self, mails: Option<Result<Indexed, String>>) {
        self.at = HashMap::new();
        self.facts = HashMap::new();
        self.mails = mails.map(|read| {
            read.map(|indexed| {
                self.at = indexed.at;
                self.facts = indexed.facts;
                Rc::new(indexed.rows)
            })
        });
    }

    fn append(&mut self, indexed: Indexed) {
        if let Some(Ok(rows)) = &mut self.mails {
            let rows = Rc::make_mut(rows);
            for row in indexed.rows {
                if !self.at.contains_key(&row.key) {
                    self.at.insert(row.key.clone(), rows.len());
                    rows.push(row);
                }
            }
            self.facts.extend(indexed.facts);
        }
    }

    /// Invalidates an in-flight read immediately, including search's debounce gap.
    pub fn invalidate(&mut self) {
        self.read += 1;
        self.loading = false;
    }

    fn accepts(&self, read: u64) -> bool {
        self.read == read
    }

    pub(crate) fn matches_query(&self, open: Open, filter: Filter, search: &Option<Search>) -> bool {
        self.query.as_ref().is_some_and(|query| query.0 == open && query.1 == filter && &query.2 == search)
    }

    /// The picked messages' facts, in list order.
    pub fn picked(&self) -> Vec<Fact> {
        let facts = self.selected.iter().filter_map(|key| self.facts.get(key));
        facts.copied().collect()
    }

    /// Keeps the picks still listed.
    fn pick(&mut self, keys: Vec<SharedString>) {
        let Some(Ok(mails)) = &self.mails else {
            self.selected.clear();
            self.chosen = None;
            return;
        };
        self.selected = keys
            .into_iter()
            .filter(|key| self.at.contains_key(key))
            .collect();
        self.chosen = match &self.selected[..] {
            [key] => Some(mails[self.at[key]].clone()),
            _ => None,
        };
    }
}

impl Mail {
    pub fn open_box(&mut self, open: Open, cx: &mut Context<Self>) {
        log::info!("mail: open {}", open.key());
        if open != self.open {
            self.open = open;
            if let Open::Account(id) = open {
                self.read_identity(id, cx);
                self.read_rules(id, cx);
                self.read_drafts_up(id, cx);
                self.read_sent_copy(id, cx);
            }
            if open == Open::Contacts {
                self.read_contacts(cx);
            }
            self.listing.show(None);
            self.listing.pick(Vec::new());
            self.stop_seeking(cx);
            self.read_letter(cx);
        }
        self.read_mail(cx);
        self.notify(cx);
    }

    pub fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        log::info!("mail: filter {filter:?}");
        self.filter = filter;
        self.read_mail(cx);
        cx.notify();
    }

    pub fn select(&mut self, keys: &[SharedString], cx: &mut Context<Self>) {
        self.listing.pick(keys.to_vec());
        self.read_letter(cx);
        self.read_on_open(cx);
        cx.notify();
    }

    /// Unread counts, and the open list when it lists mail.
    pub fn read_mail(&mut self, cx: &mut Context<Self>) {
        self.load_mail(false, cx);
    }

    pub fn load_more_mail(&mut self, cx: &mut Context<Self>) {
        if !self.listing.loading && self.listing.has_more {
            self.load_mail(true, cx);
        }
    }

    fn load_mail(&mut self, mut append: bool, cx: &mut Context<Self>) {
        self.listing.read += 1;
        let (read, open, filter) = (self.listing.read, self.open, self.filter);
        let scope = open.scope();
        let search = match scope.map(|_| self.seeking.search(filter, cx)).transpose() {
            Ok(search) => search.flatten(),
            Err(error) => {
                log::warn!("mail: a search's dates failed: {error}");
                self.listing.loading = false;
                self.listing.has_more = false;
                self.listing.show(Some(Err(error)));
                self.listing.pick(Vec::new());
                return cx.notify();
            }
        };
        let query = (open, filter, search.clone());
        if self.listing.query.as_ref() != Some(&query) {
            self.listing.show(None);
            self.listing.pick(Vec::new());
            self.listing.next = None;
            self.listing.tail = None;
            self.listing.has_more = false;
            self.listing.query = Some(query);
            append = false;
        }
        let loaded = self.listing.mails.as_ref().and_then(|rows| rows.as_ref().ok()).map_or(0, |rows| rows.len());
        let limit = if scope.is_some() { 20 } else { loaded.max(20) + if append { 20 } else { 0 } };
        let after = if append { self.listing.next } else { None };
        let through = if append { None } else { self.listing.tail };
        let picked_drafts: Vec<_> = if open == Open::Drafts {
            self.shelved.iter()
                .filter(|draft| self.listing.selected.contains(&draft_list::key(draft.id)))
                .map(|draft| draft.id)
                .collect()
        } else { Vec::new() };
        self.listing.loading = true;
        self.listing.page_error = None;
        let none = (
            text(cx, "mail.no_sender", &[]),
            text(cx, "mail.no_subject", &[]),
        );
        let names = none.clone();
        let searched = search.is_some();
        self.with_store(
            cx,
            move |store| {
                let (mut kept, mut unsearched) = (Vec::new(), None);
                let (mut next, mut has_more) = (None, false);
                let mut last_message = None;
                let rows = match scope {
                    Some(scope) => {
                        let mut list = Vec::new();
                        let mut cursor = after;
                        loop {
                            let page = match &search {
                                Some(search) => {
                                    let (page, missing) = store.search_page(search, cursor, limit)?;
                                    unsearched = Some(missing);
                                    page
                                }
                                None => store.list_page(scope, filter, cursor, limit)?,
                            };
                            next = page.next;
                            has_more = next.is_some();
                            let crossed = through.is_some_and(|tail| page.messages.last().is_some_and(|last| {
                                (last.date, last.id.0) <= (tail.date, tail.id.0)
                            }));
                            for message in page.messages {
                                if through.is_some_and(|tail| (message.date, message.id.0) < (tail.date, tail.id.0)) {
                                    has_more = true;
                                    break;
                                }
                                list.push(message);
                            }
                            if through.is_none() || crossed || next.is_none() {
                                if through.is_some() && has_more {
                                    next = list.last().map(|last| MessageCursor { date: last.date, id: last.id }).or(through);
                                }
                                break;
                            }
                            cursor = next;
                        }
                        let tail = list.last().map(|last| MessageCursor { date: last.date, id: last.id });
                        let facts = list.iter().map(|each| (key(each), fact(each))).collect();
                        last_message = tail;
                        let rows = list.into_iter().map(|each| row(each, &none)).collect();
                        Some(Indexed {
                            facts,
                            ..index(rows)
                        })
                    }
                    None if open == Open::Drafts => {
                        kept = store.drafts_prefix(limit + 1)?;
                        has_more = kept.len() > limit;
                        kept.truncate(limit);
                        // A newly saved draft may displace a still-selected draft from the prefix.
                        for id in picked_drafts {
                            if !kept.iter().any(|draft| draft.id == id) {
                                if let Some(draft) = store.draft(id)? { kept.push(draft); }
                            }
                        }
                        kept.sort_by_key(|draft| std::cmp::Reverse((draft.saved, draft.id.0)));
                        Some(index(draft_list::rows(&kept, &none)))
                    }
                    None => None,
                };
                let queued = match open {
                    Open::Outbox => {
                        let mut queued = store.outbox_prefix(limit + 1)?;
                        has_more = queued.len() > limit;
                        queued.truncate(limit);
                        queued
                    }
                    _ => Vec::new(),
                };
                let unread = store.unread()?;
                Ok((rows, unread, kept, queued, store.pending()?, unsearched, next, has_more, last_message))
            },
            move |mail, read_now, cx| {
                if !mail.listing.accepts(read) {
                    return log::info!("mail: list read {read} ended after a newer one");
                }
                mail.listing.loading = false;
                match read_now {
                    Ok((rows, unread, kept, queued, pending, unsearched, next, has_more, last_message)) => {
                        if last_message.is_some() && (append || mail.listing.tail.is_none()) {
                            mail.listing.tail = last_message;
                        }
                        mail.listing.next = next;
                        mail.listing.has_more = has_more;
                        mail.seeking.unsearched = unsearched;
                        mail.unread = unread;
                        mail.shelved = kept;
                        mail.pending = pending;
                        let rows = match open {
                            Open::Outbox => Some(index(outbox_list::rows(&queued, &names, cx))),
                            _ => rows,
                        };
                        mail.queued = queued;
                        let shown = rows.as_ref().map_or(0, |rows| rows.rows.len());
                        log::info!("mail: list read {read} shows {shown} messages, searched {searched}");
                        if append && scope.is_some() {
                            if let Some(rows) = rows { mail.listing.append(rows); }
                        } else {
                            mail.listing.show(rows.map(Ok));
                        }
                    }
                    Err(error) => {
                        log::warn!("mail: list read {read} failed: {error}");
                        mail.listing.page_error = Some(error.clone());
                        if mail.listing.mails.is_none() {
                            mail.seeking.unsearched = None;
                            let listed = scope.is_some() || matches!(open, Open::Drafts | Open::Outbox);
                            mail.listing.show(listed.then_some(Err(error)));
                        }
                    }
                }
                let keys = std::mem::take(&mut mail.listing.selected);
                mail.listing.pick(keys);
                mail.read_letter(cx);
            },
        );
    }

    /// Unread mail in every account's inbox.
    pub fn inboxes_unread(&self) -> u32 {
        self.syncs
            .values()
            .flat_map(|syncing| &syncing.folders)
            .filter(|folder| folder.role == Some(Role::Inbox))
            .filter_map(|folder| self.unread.get(&folder.id))
            .sum()
    }
}

fn key(summary: &Summary) -> SharedString {
    summary.id.0.to_string().into()
}

fn fact(summary: &Summary) -> Fact {
    Fact {
        id: summary.id,
        account: summary.account,
        mailbox: summary.mailbox,
        role: summary.role,
        pop: summary.pop,
        seen: summary.seen,
        flagged: summary.flagged,
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `none` names a missing sender, then subject.
fn row(summary: Summary, none: &(SharedString, SharedString)) -> Row {
    let key = key(&summary);
    let from = summary.from.first().map(|from| match &from.name {
        Some(name) if !name.trim().is_empty() => one_line(name),
        _ => one_line(&from.email),
    });
    let from: SharedString = from.map_or_else(|| none.0.clone(), Into::into);
    let subject: SharedString = summary
        .subject
        .map(|subject| one_line(&subject))
        .map_or_else(|| none.1.clone(), Into::into);
    let row = match summary.date.map(jiff::Timestamp::from_second) {
        Some(Ok(at)) => Row::new(key, from, subject, at),
        Some(Err(error)) => {
            log::warn!("mail: message {} date unshown: {error}", summary.id.0);
            Row::undated(key, from, subject)
        }
        None => Row::undated(key, from, subject),
    };
    let row = row.unread(!summary.seen).starred(summary.flagged);
    match summary.attachments {
        Some(count) => row.attachments(count as usize),
        None => row,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_retains_selection_and_invalidated_scope_rejects_old_reads() {
        let make = |id: &'static str| Row::undated(id, "Sender", "Subject");
        let mut listing = Listing::default();
        listing.show(Some(Ok(index(vec![make("1"), make("2")]))));
        listing.pick(vec!["2".into()]);
        listing.append(index(vec![make("2"), make("3")]));
        assert_eq!(listing.mails.as_ref().unwrap().as_ref().unwrap().len(), 3);
        assert_eq!(listing.selected, vec![SharedString::from("2")]);
        assert_eq!(listing.chosen.as_ref().unwrap().key.as_ref(), "2");
        let old = listing.read;
        listing.loading = true;
        listing.invalidate();
        assert!(!listing.accepts(old));
        assert!(!listing.loading);
        assert!(listing.accepts(listing.read));
    }

    #[test]
    fn summary_rows_collapse_header_newlines_for_single_line_components() {
        let summary = Summary {
            id: MessageId(1),
            account: AccountId(1),
            mailbox: MailboxId(1),
            role: None,
            pop: false,
            from: vec![ely_mail_engine::Mailbox {
                name: Some("Sender\nName".into()),
                email: "sender@example.com".into(),
            }],
            subject: Some("First\r\nSecond".into()),
            date: None,
            seen: false,
            flagged: false,
            attachments: None,
        };
        let row = row(summary, &("Unknown".into(), "No subject".into()));
        assert_eq!(row.from.as_ref(), "Sender Name");
        assert_eq!(row.subject.as_ref(), "First Second");
    }
}
