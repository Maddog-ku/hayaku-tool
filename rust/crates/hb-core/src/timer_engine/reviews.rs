//! 對應 C# `Core/TimerEngine.Reviews.cs`：審核提示。有新的變更申請等我審核時，從工作列旁跳提示，可以直接核准或駁回。

use hb_api::ChangeRequest;

use super::{TimerEngine, ToastAction, ToastRequest};

impl TimerEngine {
    /// 待我審核的清單裡有新的申請（這次開程式之後送出的）就跳提示；一次超過 3 筆合成一則。
    /// 待審清單每 10 分鐘，或收到「有新的變更申請」通知（約 2 分鐘輪詢一次）時重抓。
    pub(super) fn check_review_alerts(&self) {
        if !self.0.config.borrow().notification_alerts {
            return;
        }
        let started_at = self.s().started_at;
        let mut fresh: Vec<ChangeRequest> = Vec::new();
        for c in self.pending_reviews().iter() {
            if c.created_at > started_at && self.s_mut().alerted.insert(format!("cr:{}", c.id)) {
                fresh.push(c.clone());
            }
        }
        if fresh.len() > 3 {
            let labels: Vec<String> = fresh
                .iter()
                .take(3)
                .map(|c| format!("{} {}", c.type_text(), c.task_key.as_deref().unwrap_or("")).trim().to_owned())
                .collect();
            let e = self.clone();
            self.toast(
                ToastRequest::new(
                    format!("有 {} 筆申請等你審核", fresh.len()),
                    format!("{}…", labels.join("、")),
                    vec![
                        ToastAction::new("查看", move || {
                            e.open_reviews();
                            async {}
                        })
                        .primary(),
                    ],
                )
                .sticky(true)
                .from_bottom(),
            );
            return;
        }
        for c in &fresh {
            self.toast(self.review_toast(c, false));
        }
    }

    /// 審核提示；demo：截圖、檢查外觀用，按鈕不會真的送出。
    pub fn review_toast(&self, c: &ChangeRequest, demo: bool) -> ToastRequest {
        let who = c.requester.as_ref().map_or("有人", |r| r.display_name.as_str());
        let mut lines =
            vec![format!("{who}：{} {}", c.task_key.as_deref().unwrap_or(""), c.task_title).trim_end().to_owned()];
        if !c.changes.is_empty() {
            lines.push(format!(
                "{}{}",
                c.changes.iter().take(3).cloned().collect::<Vec<_>>().join("、"),
                if c.changes.len() > 3 { "…" } else { "" }
            ));
        }
        if let Some(reason) = c.reason.as_deref().filter(|r| !r.trim().is_empty()) {
            lines.push(format!("理由：{reason}"));
        }
        let demo_notice = |e: TimerEngine| {
            move || {
                e.notify("（測試）這是模擬的申請，沒有真的送出".to_owned());
                async {}
            }
        };
        let approve = if demo {
            ToastAction::new("核准", demo_notice(self.clone()))
        } else {
            let (e, c) = (self.clone(), c.clone());
            ToastAction::new("核准", move || {
                let (e, c) = (e.clone(), c.clone());
                async move { e.review(&c, true, None).await }
            })
        };
        let reject = if demo {
            ToastAction::new("駁回", demo_notice(self.clone()))
        } else {
            let (e, c) = (self.clone(), c.clone());
            ToastAction::new("駁回", move || {
                e.raise_later(|ev| &ev.reject_requested, c.clone());
                async {}
            })
        };
        let e = self.clone();
        ToastRequest::new(
            format!("待審核・{}", c.type_text()),
            lines.join("\n"),
            vec![
                approve.primary(),
                reject,
                ToastAction::new("查看", move || {
                    e.open_reviews();
                    async {}
                }),
            ],
        )
        .sticky(true)
        .from_bottom()
        .tag(format!("cr:{}", c.id))
    }

    fn open_reviews(&self) {
        self.request_workspace("reviews", None);
    }

    /// 從審核提示核准或駁回（跟審核頁一樣：駁回新增任務的申請會刪掉那個任務）。
    /// 結果用提示告知；成功後把這個任務「有新的變更申請」的通知標為已讀、重抓待審清單。
    pub async fn review(&self, c: &ChangeRequest, approve: bool, note: Option<&str>) {
        let what = format!("{} {}", c.task_key.as_deref().unwrap_or(""), c.task_title).trim().to_owned();
        let verb = if approve { "核准" } else { "駁回" };
        let reject_action = if !approve && c.r#type == "CREATE" { Some("delete") } else { None };
        if let Err(err) = self.0.api.review_change_request(&c.id, approve, false, note, reject_action).await {
            log::error!("{verb}變更申請 {}（{what}）失敗: {err}", c.id);
            let e = self.clone();
            self.toast(ToastRequest::new(
                format!("{verb}失敗"),
                format!("{}・{what}：{err}", c.type_text()),
                vec![
                    ToastAction::new("到審核頁看看", move || {
                        e.open_reviews();
                        async {}
                    })
                    .primary(),
                ],
            ));
            return;
        }
        log::info!("已{verb}變更申請 {}（{} {what}）", c.id, c.type_text());
        self.toast(ToastRequest::new(format!("已{verb}"), format!("{}・{what}", c.type_text()), Vec::new()));
        self.after_review(c).await;
    }

    /// 審核完：把這個任務「有新的變更申請」的通知標為已讀、重抓待審清單。
    pub(super) async fn after_review(&self, c: &ChangeRequest) {
        let read: Vec<String> = self
            .notifications()
            .iter()
            .filter(|n| {
                n.unread
                    && n.r#type == "CHANGE_REQUESTED"
                    && c.task_key
                        .as_deref()
                        .is_some_and(|k| n.task_key.as_deref().is_some_and(|nk| crate::ci_set::eq_ic(nk, k)))
            })
            .map(|n| n.id.clone())
            .collect();
        if !read.is_empty() {
            self.mark_notifications_read(Some(read)).await;
        }
        self.refresh_reviews().await;
        let e = self.clone();
        self.spawn(async move { e.after_external_change().await });
    }
}
