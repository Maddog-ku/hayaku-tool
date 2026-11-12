//! 對應 C# `UI/SettingsWindow.Slack.cs`：設定的 Slack 頁（建立 App 的步驟、貼金鑰、送測試訊息、中斷連線）。
//! 不碰畫面的部分在 hb-slack 的 `SlackSettings`。

use hb_slack::{SlackCredentials, SlackSettings, SlackStatus, TestOutcome, Tone};
use slint::{ComponentHandle, SharedString};

use super::{SettingsInner, later, with_settings};
use crate::dialogs;
use crate::panel::spawn;

fn tone_code(tone: Tone) -> i32 {
    match tone {
        Tone::Muted => 1,
        Tone::Run => 2,
        Tone::Error => 5,
    }
}

impl SettingsInner {
    pub(super) fn load_slack(&self) {
        self.show_slack_account(SlackSettings::load_account().as_ref());
        self.ui.set_slack_forward(self.engine.config().slack_forward);
        self.refresh_slack_status();
        if let Some(slack) = self.engine.slack() {
            let me = self.me.clone();
            let id = slack.subscribe_status_changed(Box::new(move || {
                if let Some(p) = me.upgrade() {
                    p.refresh_slack_status();
                }
            }));
            self.slack_status_id.set(Some(id));
        }
    }

    pub(super) fn wire_slack(&self) {
        let ui = &self.ui;
        ui.on_slack_forward_toggled(with_settings!(self.me, |p, v| {
            p.engine.config_mut().slack_forward = v;
            p.apply();
        }));
        ui.on_slack_copy_manifest(with_settings!(self.me, |p| {
            let copied = hb_ui_common::task_text::set_clipboard(SlackSettings::MANIFEST).is_ok();
            p.show_slack_status(Some(SlackSettings::copy_manifest_status(copied)));
        }));
        ui.on_slack_open_apps(with_settings!(self.me, |_p| hb_win::open(SlackSettings::APPS_URL)));
        ui.on_slack_connect(with_settings!(self.me, |p| p.slack_connect()));
        ui.on_slack_test(with_settings!(self.me, |p| p.slack_test()));
        ui.on_slack_logout(with_settings!(self.me, |p| p.slack_logout()));
    }

    fn refresh_slack_status(&self) {
        let slack = self.engine.slack();
        let status = SlackSettings::live_status(slack.as_deref());
        self.ui.set_slack_live(status.text.into());
        self.ui.set_slack_live_color(tone_code(status.tone));
    }

    /// 已連線只顯示工作區、送測試訊息與中斷連線；沒有金鑰才顯示建立 App 的步驟與輸入欄位。
    fn show_slack_account(&self, cred: Option<&SlackCredentials>) {
        self.ui.set_slack_connected(cred.is_some());
        self.ui.set_slack_account_text(cred.map(SlackSettings::account_text).unwrap_or_default().into());
        if cred.is_none() {
            self.ui.set_slack_bot(SharedString::new());
            self.ui.set_slack_app(SharedString::new());
        }
    }

    fn show_slack_status(&self, status: Option<SlackStatus>) {
        match status {
            Some(s) => {
                self.ui.set_slack_status(s.text.into());
                self.ui.set_slack_status_color(tone_code(s.tone));
            }
            None => self.ui.set_slack_status(SharedString::new()),
        }
    }

    fn slack_connect(&self) {
        let cred = match SlackSettings::validate(
            &self.ui.get_slack_bot(),
            &self.ui.get_slack_app(),
            &self.ui.get_slack_user(),
        ) {
            Ok(cred) => cred,
            Err(status) => {
                self.show_slack_status(Some(status));
                return;
            }
        };
        self.ui.set_slack_connect_enabled(false);
        self.show_slack_status(Some(SlackStatus { text: SlackSettings::CONNECTING.to_owned(), tone: Tone::Muted }));
        let me = self.me.clone();
        let slack = self.engine.slack();
        spawn(async move {
            let result = SlackSettings::connect(cred, slack).await;
            let Some(p) = me.upgrade() else { return };
            match result {
                Ok((cred, status)) => {
                    p.show_slack_account(Some(&cred));
                    p.show_slack_status(Some(status));
                }
                Err(status) => p.show_slack_status(Some(status)),
            }
            p.ui.set_slack_connect_enabled(true);
        });
    }

    fn slack_test(&self) {
        if SlackSettings::load_account().is_none() {
            self.show_slack_account(None);
            return;
        }
        self.ui.set_slack_test_enabled(false);
        self.show_slack_status(Some(SlackStatus { text: SlackSettings::SENDING.to_owned(), tone: Tone::Muted }));
        let me = self.me.clone();
        spawn(async move {
            let outcome = SlackSettings::send_test().await;
            let Some(p) = me.upgrade() else { return };
            match outcome {
                TestOutcome::NotConfigured => {
                    p.show_slack_account(None);
                    p.show_slack_status(None);
                }
                TestOutcome::Done(status) => p.show_slack_status(Some(status)),
            }
            p.ui.set_slack_test_enabled(true);
        });
    }

    fn slack_logout(&self) {
        let me = self.me.clone();
        later(move || {
            let Some(p) = me.upgrade() else { return };
            if !dialogs::confirm(
                Some(p.ui.window()),
                SlackSettings::DISCONNECT_TITLE,
                SlackSettings::DISCONNECT_CONFIRM,
            ) {
                return;
            }
            SlackSettings::disconnect(p.engine.slack().as_deref());
            p.ui.set_slack_user(SharedString::new());
            p.show_slack_account(None);
            p.show_slack_status(None);
        });
    }
}
