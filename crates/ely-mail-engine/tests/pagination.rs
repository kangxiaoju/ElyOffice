use ely_mail_engine::{Fetched, Field, Filter, Key, Role, Scope, Search, Store};

#[test]
fn pages_cross_equal_and_null_dates_without_duplicates_and_search_is_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let account = store.add_account(&fake::account("pages@example.com")).unwrap();
    let mailbox = store.set_folders(account, &[("INBOX", Some(Role::Inbox))]).unwrap()[0];
    store.open_mailbox(mailbox, 1).unwrap();
    for uid in 1..=65 {
        let date = if uid <= 43 { "Date: Mon, 19 Oct 2026 10:00:00 +0000\r\n" } else { "" };
        let bytes = format!("From: a@example.com\r\nSubject: needle\r\n{date}\r\nbody");
        let flags = if uid % 2 == 0 { vec!["\\Seen"] } else { vec![] };
        store.put(mailbox, &Key::Imap { uidvalidity: 1, uid }, &flags, Fetched::Whole(bytes.as_bytes())).unwrap();
    }
    for filter in [Filter::default(), Filter { unread: true, ..Filter::default() }] {
        let expected = store.list(Scope::Mailbox(mailbox), filter).unwrap();
        let mut cursor = None;
        let mut actual = Vec::new();
        loop {
            let page = store.list_page(Scope::Mailbox(mailbox), filter, cursor, 20).unwrap();
            assert!(page.messages.len() <= 20);
            actual.extend(page.messages);
            cursor = page.next;
            if cursor.is_none() { break; }
        }
        assert_eq!(actual, expected);
        let search = Search { words: "needle".into(), field: Field::Subject, filter, ..Search::default() };
        let mut actual = Vec::new();
        let mut cursor = None;
        loop {
            let (page, _) = store.search_page(&search, cursor, 20).unwrap();
            assert!(page.messages.len() <= 20);
            actual.extend(page.messages);
            cursor = page.next;
            if cursor.is_none() { break; }
        }
        assert_eq!(actual, expected);
    }
    let first = store.list_page(Scope::Mailbox(mailbox), Filter::default(), None, 20).unwrap();
    store.put(mailbox, &Key::Imap { uidvalidity: 1, uid: 66 }, &[], Fetched::Whole(b"From: a@example.com\r\nDate: Mon, 19 Oct 2026 10:00:00 +0000\r\n\r\nnew")).unwrap();
    let second = store.list_page(Scope::Mailbox(mailbox), Filter::default(), first.next, 20).unwrap();
    assert!(!second.messages.iter().any(|row| first.messages.iter().any(|old| old.id == row.id)));
    assert_eq!(second.messages.len(), 20);
    assert_eq!(second.messages[0].id, store.list(Scope::Mailbox(mailbox), Filter::default()).unwrap()[21].id);
    let other = store.mailbox(account, "Other").unwrap();
    assert!(store.list_page(Scope::Mailbox(other), Filter::default(), None, 20).unwrap().messages.is_empty());
    let no_match = Search { words: "absent".into(), ..Search::default() };
    let (empty, _) = store.search_page(&no_match, None, 20).unwrap();
    assert!(empty.messages.is_empty());
    assert!(empty.next.is_none());
    assert!(store.list_page(Scope::Inboxes, Filter::default(), None, 0).is_err());
}

#[test]
fn drafts_and_outbox_prefixes_leave_all_later_rows_and_delivery_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = Store::open(dir.path()).unwrap();
    let account = store.add_account(&fake::account("queue@example.com")).unwrap();
    let unsent = ely_mail_engine::Unsent {
        to: vec!["to@example.com".into()], cc: vec![], bcc: vec![], subject: "Draft".into(),
        html: Some("<p>body</p>".into()), words: String::new(), markdown: false,
        earlier: None, in_reply_to: None, references: vec![], headers_only: false, missing: 0,
    };
    for n in 0..45 {
        store.shelve(None, account, &unsent, None).unwrap();
        store.enqueue(account, None, &unsent, &[], &ely_mail_engine::Letter {
            envelope: vec!["to@example.com".into()], message_id: format!("{n}@example.com"), bytes: vec![1],
        }).unwrap();
    }
    assert_eq!(store.drafts_prefix(21).unwrap().len(), 21);
    assert_eq!(store.outbox_prefix(21).unwrap().len(), 21);
    assert_eq!(store.drafts_prefix(41).unwrap().len(), 41);
    assert_eq!(store.outbox_prefix(41).unwrap().len(), 41);
    assert_eq!(store.drafts_prefix(61).unwrap(), store.drafts().unwrap());
    assert_eq!(store.outbox_prefix(61).unwrap(), store.outbox().unwrap());
    assert_eq!(store.to_send(false).unwrap().len(), 45);
}