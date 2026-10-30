//! New mail as system notifications (MAIL-15).

use ely_mail_engine::{AccountId, Summary};
use ely_ui::settings::switches::{self, NOTIFY};
use gpui::{Context, SystemNotification};

use crate::mail::{Mail, MailNotice, text};

/// Tags of new-mail notices start so.
const TAG: &str = "mail-new-";
/// Senders named before "and others".
const NAMED: usize = 3;

impl Mail {
    /// Clicking a notice brings the inboxes forward.
    pub(crate) fn answer_notices(cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        cx.on_system_notification_response(move |response, cx| {
            if !response.tag.starts_with(TAG) {
                return;
            }
            let _ = this.update(cx, |mail, cx| {
                if mail.host_notices {
                    return;
                }
                log::info!("mail: a notice was opened");
                ely_ui::tray::show_main(cx);
                mail.open_inboxes(cx);
            });
        });
    }

    /// Tells of `arrived`, unless muted or the window is front.
    pub(crate) fn announce(
        &mut self,
        id: AccountId,
        arrived: Vec<Summary>,
        cx: &mut Context<Self>,
    ) {
        let count = arrived.len();
        if count == 0 {
            return;
        }
        if !switches::on(NOTIFY, cx) {
            return log::info!("mail: {count} new, notifications are off");
        }
        if !self.host_notices && ely_ui::tray::main_in_front(cx) {
            return log::info!("mail: {count} new, the window is in front");
        }
        let mut senders: Vec<String> = Vec::new();
        for from in arrived.iter().flat_map(|each| each.from.first()) {
            let name = from.name.clone().unwrap_or_else(|| from.email.clone());
            if !senders.contains(&name) {
                senders.push(name);
            }
        }
        let mut named = senders
            .iter()
            .take(NAMED)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        if senders.len() > NAMED {
            named = text(cx, "mail.notify.more", &[("senders", &named)]).to_string();
        }
        let body = ely_ui::i18n::counted(
            cx,
            "mail.notify.body",
            count as u64,
            &[("count", &count.to_string()), ("senders", &named)],
        );
        let notice = MailNotice {
            tag: format!("{TAG}{}", id.0).into(),
            title: text(cx, "mail.notify.title", &[]),
            body,
        };
        if self.host_notices {
            cx.emit(notice);
        } else {
            cx.show_system_notification(SystemNotification {
                tag: notice.tag,
                title: notice.title,
                body: notice.body,
                actions: Vec::new(),
            });
        }
        log::info!("mail: told of {count} new messages for account {}", id.0);
    }
}
