//! 工作台「待我審核」頁（對應 C# `UI/Work/ReviewsPage.cs`）。
//!
//! 等我審核的變更申請依專案分組，可以直接核准、核准並關注、拒絕；
//! 「我送出的」列出自己還在等審核的申請，可以撤回。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use hb_api::{ChangeRequest, DateTimeOffset, WorkMath};
use hb_ui_common::ProjectIcons;
use hb_ui_common::inputs::{self, StatusMessage};
use slint::{ComponentHandle, Model, SharedString};

use super::hours::w2_common::{self, ConfirmSlot, PageCell, model, spawn, strings};
use crate::{PageArg, ReviewsItem, ReviewsPageData, WorkCtx};

/// 分頁的文字（後面加數量）。
pub(crate) fn tab_labels(pending: &[ChangeRequest], me: Option<&str>) -> (String, String) {
    let to_review = pending.iter().filter(|c| c.requester.as_ref().map(|r| r.username.as_str()) != me).count();
    let mine = pending.len() - to_review;
    (
        if to_review > 0 { format!("待我審核 {to_review}") } else { "待我審核".to_owned() },
        if mine > 0 { format!("我送出的 {mine}") } else { "我送出的".to_owned() },
    )
}

/// 這個分頁要列的申請（新的在前）。
pub(crate) fn items_for(pending: &[ChangeRequest], me: Option<&str>, mine: bool) -> Vec<ChangeRequest> {
    let mut items: Vec<ChangeRequest> =
        pending.iter().filter(|c| (c.requester.as_ref().map(|r| r.username.as_str()) == me) == mine).cloned().collect();
    items.sort_by_key(|c| std::cmp::Reverse(c.created_at));
    items
}

/// 上方的說明。
pub(crate) fn summary(items: &[ChangeRequest], mine: bool, can_review: bool) -> String {
    if mine {
        if items.is_empty() {
            "沒有在等審核的申請。".to_owned()
        } else {
            format!("{} 筆申請還在等審核，撤回後就不會再送給管理者。", items.len())
        }
    } else if items.is_empty() {
        if can_review {
            "沒有要審核的申請 🎉".to_owned()
        } else {
            "沒有要審核的申請。（一般申請由專案管理員、編輯者審核；測試結果改派執行者的申請，回報的人和測試員也能審核）"
                .to_owned()
        }
    } else {
        let mut projects: Vec<&str> = Vec::new();
        for c in items {
            if !projects.contains(&c.project.as_str()) {
                projects.push(&c.project);
            }
        }
        format!("{} 筆申請等你審核（{} 個專案）", items.len(), projects.len())
    }
}

/// 卡片的說明列：送出的人、多久以前、已記錄工時、子任務數。
pub(crate) fn meta(c: &ChangeRequest, mine: bool, now: &DateTimeOffset) -> String {
    let who = if mine {
        "送出".to_owned()
    } else {
        c.requester.as_ref().map(|r| r.display_name.clone()).unwrap_or_default()
    };
    let mut meta = format!("{who} · {}", inputs::ago_at(&c.created_at, now));
    if c.logged_minutes > 0 {
        meta.push_str(&format!(" · 已記錄 {}", WorkMath::hours_minutes(c.logged_minutes as f64)));
    }
    if c.descendant_count > 0 {
        meta.push_str(&format!(" · 含 {} 個子任務", c.descendant_count));
    }
    meta
}

/// 拒絕新增申請前的確認文字。
pub(crate) fn reject_create_text(c: &ChangeRequest) -> String {
    let logged = if c.logged_minutes > 0 {
        format!("（已經記錄 {} 工時）", WorkMath::hours_minutes(c.logged_minutes as f64))
    } else {
        String::new()
    };
    format!("拒絕會刪除 {} {}{logged}，確定？", c.task_key.as_deref().unwrap_or(""), c.task_title)
}

struct Page {
    ctx: WorkCtx,
    pending: RefCell<Vec<ChangeRequest>>,
    /// 畫面上每一項對應的申請（專案標題是 None）
    shown: RefCell<Vec<Option<ChangeRequest>>>,
    tab: Cell<usize>,
    confirm: Rc<ConfirmSlot>,
}

thread_local! {
    static PAGE: PageCell<Page> = const { PageCell::new() };
}

fn current() -> Option<Rc<Page>> {
    PAGE.with(|p| p.get())
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&ReviewsPageData) -> R) -> Option<R> {
        let window = self.ctx.window()?;
        Some(f(&window.global::<ReviewsPageData>()))
    }

    fn me(&self) -> Option<String> {
        self.ctx.engine.user().map(|u| u.username)
    }

    fn request(&self, index: i32) -> Option<ChangeRequest> {
        usize::try_from(index).ok().and_then(|i| self.shown.borrow().get(i).cloned().flatten())
    }

    /// 改畫面上某一張卡（忙碌中的按鈕、卡片自己的訊息）。
    fn update_item(&self, index: i32, f: impl FnOnce(&mut ReviewsItem)) {
        self.with(|g| {
            let items = g.get_items();
            if let Some(mut item) = usize::try_from(index).ok().and_then(|i| items.row_data(i)) {
                f(&mut item);
                items.set_row_data(index as usize, item);
            }
        });
    }
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page {
        ctx: ctx.clone(),
        pending: RefCell::new(Vec::new()),
        shown: RefCell::new(Vec::new()),
        tab: Cell::new(0),
        confirm: Rc::new(ConfirmSlot::default()),
    });
    PAGE.with(|p| p.set(page.clone()));
    let Some(window) = ctx.window() else { return };
    w2_common::wire_confirm!(window, ReviewsPageData, page.confirm.clone());
    let g = window.global::<ReviewsPageData>();
    g.set_tabs(strings(["待我審核", "我送出的"]));
    g.on_tab_selected(|i| {
        let Some(page) = current() else { return };
        page.tab.set(usize::try_from(i).unwrap_or(0));
        render(&page);
    });
    g.on_refresh(|| {
        if let Some(page) = current() {
            spawn("讀取變更申請", load(page));
        }
    });
    g.on_open_task(|key| {
        if let Some(page) = current() {
            page.ctx.show_page("tasks", Some(PageArg::Text(key.to_string())));
        }
    });
    g.on_withdraw(|i| {
        let Some(page) = current() else { return };
        let Some(c) = page.request(i) else { return };
        spawn("撤回申請", async move {
            let api = page.ctx.engine.api();
            if card_action(
                &page,
                i,
                "withdraw",
                async move { w2_common::mcp(api.withdraw_change_request(&c.id).await) },
            )
            .await
            {
                after_action(page).await;
            }
        });
    });
    g.on_review(|i, decision, note| {
        let Some(page) = current() else { return };
        let Some(c) = page.request(i) else { return };
        let (approve, watch) = match decision.as_str() {
            "approve" => (true, false),
            "watch" => (true, true),
            _ => (false, false),
        };
        let note = note.to_string();
        if !approve && c.r#type == "CREATE" {
            let Some(window) = page.ctx.window() else { return };
            let p = page.clone();
            let text = reject_create_text(&c);
            w2_common::ask!(window, ReviewsPageData, page.confirm, "拒絕新增申請", text, move || {
                spawn("審核申請", review(p, i, c, approve, watch, note, decision.to_string()));
            });
            return;
        }
        spawn("審核申請", review(page, i, c, approve, watch, note, decision.to_string()));
    });
}

/// arg："mine" 切到「我送出的」。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = current() else { return };
    if arg.as_ref().and_then(PageArg::as_text) == Some("mine") {
        page.tab.set(1);
        page.with(|g| g.set_tab_index(1));
    }
    load(page).await;
}

async fn load(page: Rc<Page>) {
    page.with(|g| {
        g.set_status_text(SharedString::new());
        g.set_summary("讀取中…".into());
    });
    match page.ctx.engine.api().list_change_requests("PENDING").await {
        Ok(list) => *page.pending.borrow_mut() = list,
        Err(e) => {
            log::error!("讀取變更申請失敗: {}", e.0);
            page.with(|g| {
                g.set_summary(SharedString::new());
                g.set_status_text(format!("讀取失敗：{}", e.0).into());
                g.set_status_tone(0);
            });
            return;
        }
    }
    render(&page);
}

fn render(page: &Page) {
    let mine = page.tab.get() == 1;
    let me = page.me();
    let pending = page.pending.borrow().clone();
    let items = items_for(&pending, me.as_deref(), mine);
    let (to_review, mine_label) = tab_labels(&pending, me.as_deref());
    let now = hb_core::time::now_offset();
    let engine = &page.ctx.engine;
    let mut rows: Vec<ReviewsItem> = Vec::new();
    let mut shown: Vec<Option<ChangeRequest>> = Vec::new();
    let mut groups: Vec<(String, Vec<ChangeRequest>)> = Vec::new();
    for c in &items {
        match groups.iter_mut().find(|(k, _)| *k == c.project) {
            Some((_, list)) => list.push(c.clone()),
            None => groups.push((c.project.clone(), vec![c.clone()])),
        }
    }
    for (key, list) in groups {
        let project = engine.project_of(&key);
        let icon = ProjectIcons::icon(project.as_ref(), &key);
        rows.push(ReviewsItem {
            header: true,
            glyph: icon.glyph.into(),
            tint: icon.color.to_slint(),
            project: project.as_ref().map_or(key.clone(), |p| p.name.clone()).into(),
            count: format!("  {}", list.len()).into(),
            ..Default::default()
        });
        shown.push(None);
        for c in list {
            rows.push(ReviewsItem {
                header: false,
                type_text: c.type_text().into(),
                is_delete: c.r#type == "DELETE",
                key: c.task_key.clone().unwrap_or_default().into(),
                title: c.task_title.as_str().into(),
                meta: meta(&c, mine, &now).into(),
                changes: strings(c.changes.iter().map(|x| format!("• {x}"))),
                reason: c
                    .reason
                    .as_deref()
                    .filter(|r| !r.is_empty())
                    .map(|r| format!("原因：{r}"))
                    .unwrap_or_default()
                    .into(),
                mine,
                can_watch: c.r#type != "DELETE",
                ..Default::default()
            });
            shown.push(Some(c));
        }
    }
    let can_review = engine.can_review();
    page.with(|g| {
        g.set_tabs(strings([to_review, mine_label]));
        g.set_summary(summary(&items, mine, can_review).into());
        g.set_items(model(rows));
    });
    *page.shown.borrow_mut() = shown;
}

/// 卡片上的按鈕（C# `Inputs.Run(button, status, …)`）：按鈕變成處理中，訊息顯示在卡片底下。
async fn card_action(
    page: &Rc<Page>,
    index: i32,
    which: &str,
    action: impl std::future::Future<Output = Result<(), inputs::ActionError>>,
) -> bool {
    let which = which.to_owned();
    inputs::run(
        |busy| page.update_item(index, |item| item.busy = if busy { which.as_str() } else { "" }.into()),
        |m: Option<StatusMessage>| {
            let (text, tone) = w2_common::status(m.as_ref());
            page.update_item(index, |item| {
                item.status_text = text;
                item.status_tone = tone;
            });
        },
        || action,
        None,
    )
    .await
}

async fn review(page: Rc<Page>, index: i32, c: ChangeRequest, approve: bool, watch: bool, note: String, which: String) {
    let api = page.ctx.engine.api();
    let reject_action = if !approve && c.r#type == "CREATE" { Some("delete") } else { None };
    let ok = card_action(&page, index, &which, async move {
        w2_common::mcp(api.review_change_request(&c.id, approve, watch, Some(&note), reject_action).await)
    })
    .await;
    if ok {
        after_action(page).await;
    }
}

async fn after_action(page: Rc<Page>) {
    page.ctx.engine.refresh_reviews().await;
    load(page.clone()).await;
    let engine = page.ctx.engine.clone();
    engine.clone().spawn(async move { engine.after_external_change().await });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::DateTime;
    use hb_api::PersonRef;

    fn cr(id: &str, project: &str, by: &str, at: &str, ty: &str) -> ChangeRequest {
        ChangeRequest {
            id: id.into(),
            project: project.into(),
            r#type: ty.into(),
            status: "PENDING".into(),
            task_key: Some(format!("{project}-1")),
            task_title: "登入頁".into(),
            requester: Some(PersonRef::new(by, by.to_uppercase())),
            reviewer: None,
            created_at: DateTime::parse_from_rfc3339(at).unwrap(),
            changes: vec![],
            reason: None,
            review_note: None,
            logged_minutes: 0,
            descendant_count: 0,
        }
    }

    #[test]
    fn 分頁與排序() {
        let pending = vec![
            cr("1", "F2P", "amy", "2026-11-10T09:00:00+08:00", "UPDATE"),
            cr("2", "REV", "me", "2026-11-11T09:00:00+08:00", "CREATE"),
            cr("3", "F2P", "bob", "2026-11-12T09:00:00+08:00", "DELETE"),
        ];
        assert_eq!(("待我審核 2".to_owned(), "我送出的 1".to_owned()), tab_labels(&pending, Some("me")));
        let items = items_for(&pending, Some("me"), false);
        assert_eq!(vec!["3", "1"], items.iter().map(|c| c.id.as_str()).collect::<Vec<_>>());
        assert_eq!("2 筆申請等你審核（1 個專案）", summary(&items, false, true));
        let mine = items_for(&pending, Some("me"), true);
        assert_eq!("1 筆申請還在等審核，撤回後就不會再送給管理者。", summary(&mine, true, true));
        assert_eq!("沒有要審核的申請 🎉", summary(&[], false, true));
        assert_eq!(("待我審核".to_owned(), "我送出的".to_owned()), tab_labels(&[], Some("me")));
    }

    #[test]
    fn 說明與拒絕確認() {
        let mut c = cr("1", "F2P", "amy", "2026-11-12T07:00:00+08:00", "CREATE");
        c.logged_minutes = 90;
        c.descendant_count = 2;
        let now = DateTime::parse_from_rfc3339("2026-11-12T10:00:00+08:00").unwrap();
        assert_eq!("AMY · 3 小時前 · 已記錄 1 小時 30 分 · 含 2 個子任務", meta(&c, false, &now));
        assert_eq!("送出 · 3 小時前 · 已記錄 1 小時 30 分 · 含 2 個子任務", meta(&c, true, &now));
        assert_eq!("拒絕會刪除 F2P-1 登入頁（已經記錄 1 小時 30 分 工時），確定？", reject_create_text(&c));
    }
}
