//! 對應 C# `UI/Work/TasksPage.Extras.cs`：任務詳細的相依關係、附件、文件、異動紀錄與刪除。
//! 四塊都平常收起、展開才讀（同一個任務只讀一次），不會因為打開任務就多打請求。
//! （清單、樹狀、看板的右鍵選單在外框的 TaskMenuState，動作見 tasks.rs 的 TaskMenuHooks。）

use std::rc::Rc;

use chrono::{Datelike, Local};
use hb_api::{DependencyItem, HayakuApi, KeyFinder, TaskActivityItem, TaskDetail};
use hb_ui_common::inputs::{self, ActionError, StatusMessage};

use slint::ComponentHandle;

use super::Page;
use super::detail::line;
use super::vm;
use crate::shared::{self, string_model, vec_model};
use crate::{TasksPageData, TkDep, TkDoc, TkLine};

/// 相依的一列說明（狀態、規則），接在編號與名稱後面。
pub(super) fn dep_meta(d: &DependencyItem, with_rule: bool) -> String {
    let mut s = String::new();
    if let Some(status) = d.other_status.as_deref().filter(|s| !s.is_empty()) {
        s += &format!("  · {status}");
    }
    if with_rule && !d.rule.is_empty() {
        s += &format!("  · {}", d.rule);
    }
    s
}

/// 附件的說明：「  · 大小 · 上傳的人 · 多久前」。
pub(super) fn attachment_meta(a: &hb_api::AttachmentItem, now: &hb_api::DateTimeOffset) -> String {
    let parts: Vec<String> = [a.size.map(vm::file_size), a.uploader.clone(), a.at.map(|at| inputs::ago_at(&at, now))]
        .into_iter()
        .flatten()
        .collect();
    format!("  · {}", parts.join(" · "))
}

/// 異動紀錄的時間與人：「11/12 09:05  amy  」（沒有時間的空白）。
pub(super) fn history_meta(item: &TaskActivityItem) -> String {
    let when = if item.at.year() <= 1 {
        String::new()
    } else {
        item.at.with_timezone(&Local).format("%m/%d %H:%M").to_string()
    };
    format!("{when}  {}  ", item.actor)
}

impl Page {
    pub(super) fn wire_extras(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_d_deps_toggled, |p, open| {
            let key = p.st.borrow().detail.as_ref().map(|d| d.key.clone());
            if open
                && let Some(key) = key
                && p.st.borrow().deps_for.as_deref() != Some(key.as_str())
            {
                shared::spawn("讀取相依關係", async move { p.load_deps(key).await });
            }
        });
        on!(d, self, on_d_dep_remove, |p, i| shared::spawn("移除相依", async move { p.remove_dep(i as usize).await }));
        on!(d, self, on_d_dep_add, |p| shared::spawn("新增相依", async move { p.add_dep().await }));
        on!(d, self, on_d_dep_quick_selected, |p, i| {
            if i > 0
                && let Some(picked) = p.st.borrow().dep_quick.get(i as usize).and_then(|s| KeyFinder::parse(Some(s)))
            {
                p.with_data(|d| {
                    d.set_d_dep_other(picked.into());
                    d.set_d_dep_dir_index(2);
                });
            }
        });
        on!(d, self, on_d_attach_toggled, |p, open| {
            if open {
                shared::spawn("讀取附件", async move { p.load_attachments().await });
            }
        });
        on!(d, self, on_d_attach_open, |p, i| p.open_attachment(i as usize));
        on!(d, self, on_d_attach_web_open, |p| {
            if let Some(url) = p.st.borrow().attach_web.clone() {
                hb_win::open(&url);
            }
        });
        on!(d, self, on_d_docs_toggled, |p, open| {
            if open {
                shared::spawn("讀取文件", async move { p.load_docs().await });
            }
        });
        on!(d, self, on_d_history_toggled, |p, open| {
            let key = p.st.borrow().detail.as_ref().map(|d| d.key.clone());
            if open
                && let Some(key) = key
                && p.st.borrow().history_for.as_deref() != Some(key.as_str())
            {
                {
                    let mut st = p.st.borrow_mut();
                    st.history_for = Some(key.clone());
                    st.history_cursor = None;
                    st.history_lines.clear();
                }
                shared::spawn("讀取異動紀錄", async move { p.load_history(key).await });
            }
        });
        on!(d, self, on_d_history_load_more, |p| {
            let key = p.st.borrow().history_for.clone();
            if let Some(key) = key {
                p.with_data(|d| d.set_d_history_more(false));
                shared::spawn("讀取異動紀錄", async move { p.load_history(key).await });
            }
        });
    }

    /// 換了任務：四塊收起、清空，下次展開再讀；同一個任務重畫（例如改了欄位）就保留。
    pub(super) fn reset_extras(&self, d: &TaskDetail) {
        let docs_header = if d.content_block_count > 1 {
            format!("文件（{}）", d.content_block_count)
        } else {
            "文件".to_owned()
        };
        self.with_data(|x| {
            x.set_d_docs_header(docs_header.into());
            x.set_d_docs_visible(d.content_block_count > 0);
        });
        let mut st = self.st.borrow_mut();
        let key = Some(d.key.as_str());
        if st.deps_for.as_deref() == key
            || st.attach_for.as_deref() == key
            || st.docs_for.as_deref() == key
            || st.history_for.as_deref() == key
        {
            return;
        }
        st.deps_for = None;
        st.attach_for = None;
        st.docs_for = None;
        st.history_for = None;
        st.history_cursor = None;
        st.history_lines.clear();
        st.deps_rows.clear();
        drop(st);
        self.with_data(|x| {
            x.set_d_deps_expanded(false);
            x.set_d_attach_expanded(false);
            x.set_d_docs_expanded(false);
            x.set_d_history_expanded(false);
            x.set_d_deps(vec_model(Vec::new()));
            x.set_d_dep_form_visible(false);
            x.set_d_attach(vec_model(Vec::new()));
            x.set_d_attach_web(false);
            x.set_d_docs(vec_model(Vec::new()));
            x.set_d_docs_lines(vec_model(Vec::new()));
            x.set_d_history(vec_model(Vec::new()));
            x.set_d_history_more(false);
        });
    }

    // ───────────── 相依關係 ─────────────

    async fn load_deps(self: Rc<Self>, key: String) {
        self.st.borrow_mut().deps_for = Some(key.clone());
        self.with_data(|d| {
            d.set_d_deps(vec_model(vec![TkDep { kind: 3, title: "讀取中…".into(), ..Default::default() }]));
            d.set_d_dep_form_visible(false);
        });
        let deps = match self.engine().api().list_dependencies(&key).await {
            Ok(d) => d,
            Err(e) => {
                log::error!("讀取 {key} 的相依關係失敗: {}", e.0);
                self.with_data(|d| {
                    d.set_d_deps(vec_model(vec![TkDep {
                        kind: 4,
                        title: format!("讀取相依關係失敗：{}", e.0).into(),
                        ..Default::default()
                    }]))
                });
                self.st.borrow_mut().deps_for = None;
                return;
            }
        };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key) {
            return;
        }
        let mut rows: Vec<TkDep> = Vec::new();
        let mut refs: Vec<Option<(DependencyItem, bool)>> = Vec::new();
        let group = |title: &str, items: &[DependencyItem], related: bool, rows: &mut Vec<TkDep>, refs: &mut Vec<_>| {
            if items.is_empty() {
                return;
            }
            rows.push(TkDep { kind: 0, title: title.into(), ..Default::default() });
            refs.push(None);
            for d in items {
                rows.push(TkDep {
                    kind: if related { 2 } else { 1 },
                    key: d.other_key.as_str().into(),
                    title: d.other_title.as_str().into(),
                    meta: dep_meta(d, !related).into(),
                    rule: d.rule.as_str().into(),
                    blocking: !related && d.blocking,
                    removable: related || !d.id.is_empty(),
                });
                refs.push(Some((d.clone(), related)));
            }
        };
        group("這個任務在等", &deps.predecessors, false, &mut rows, &mut refs);
        group("在等這個任務", &deps.successors, false, &mut rows, &mut refs);
        group("關聯任務", deps.related_tasks(), true, &mut rows, &mut refs);
        if deps.predecessors.is_empty() && deps.successors.is_empty() && deps.related_tasks().is_empty() {
            rows.push(TkDep {
                kind: 3, title: "沒有相依關係，也沒有關聯任務".into(), ..Default::default()
            });
            refs.push(None);
        }
        // 快速選同一層的任務當關聯（打開過上層才有，不多打請求）
        let quick: Vec<String> = {
            let st = self.st.borrow();
            let parent = st.detail.as_ref().and_then(|d| d.path.last().map(|p| p.0.to_uppercase()));
            match parent.and_then(|p| st.children_of.get(&p)) {
                Some(siblings) if siblings.len() > 1 => std::iter::once("同一層的任務…".to_owned())
                    .chain(siblings.iter().filter(|c| c.0 != key).map(|c| format!("{} {}", c.0, c.1)))
                    .collect(),
                _ => Vec::new(),
            }
        };
        {
            let mut st = self.st.borrow_mut();
            st.deps_rows = refs;
            st.dep_quick = quick.clone();
        }
        self.with_data(|d| {
            d.set_d_deps(vec_model(rows));
            d.set_d_dep_form_visible(true);
            d.set_d_dep_other(Default::default());
            d.set_d_dep_dir_index(0);
            d.set_d_dep_template_index(0);
            d.set_d_dep_quick(string_model(&quick));
            d.set_d_dep_quick_index(0);
        });
    }

    async fn remove_dep(self: Rc<Self>, index: usize) {
        let Some(key) = self.st.borrow().deps_for.clone() else { return };
        let Some(Some((dep, related))) = self.st.borrow().deps_rows.get(index).cloned() else { return };
        let api = self.engine().api();
        let message: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        let (m, k, other, id) = (message.clone(), key.clone(), dep.other_key.clone(), dep.id.clone());
        let ok = inputs::run(
            |_| {},
            |msg| self.show_msg(msg),
            move || async move {
                *m.borrow_mut() =
                    if related { api.remove_related(&k, &other).await } else { api.remove_dependency(&id).await }
                        .map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if ok {
            let fallback = if related {
                format!("已移除與 {} 的關聯", dep.other_key)
            } else {
                format!("已移除與 {} 的相依", dep.other_key)
            };
            self.show_msg(Some(StatusMessage::ok(message.borrow().clone().unwrap_or(fallback))));
            self.clone().load_deps(key).await;
        }
    }

    /// 新增相依：任務編號、方向（這個任務等它／它等這個任務）、規則；或「關聯」（不擋開始、可以跨專案）。
    async fn add_dep(self: Rc<Self>) {
        let Some(key) = self.st.borrow().deps_for.clone() else { return };
        let Some((other, direction, template)) = self
            .with_data(|d| (d.get_d_dep_other().to_string(), d.get_d_dep_dir_index(), d.get_d_dep_template_index()))
        else {
            return;
        };
        let Some(target) = KeyFinder::parse(Some(&other)) else {
            self.show_msg(Some(StatusMessage::error("請填另一個任務的編號（例 F2P-12）")));
            return;
        };
        let api = self.engine().api();
        let message: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        let (m, k, t) = (message.clone(), key.clone(), target.clone());
        let code = HayakuApi::DEPENDENCY_TEMPLATES[template.max(0) as usize % HayakuApi::DEPENDENCY_TEMPLATES.len()].0;
        let ok = inputs::run(
            |busy| {
                self.with_data(|d| d.set_d_dep_busy(busy));
            },
            |msg| self.show_msg(msg),
            move || async move {
                *m.borrow_mut() = if direction == 2 {
                    api.add_related(&k, &t).await
                } else {
                    api.add_dependency(&k, &t, direction == 0, code).await
                }
                .map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if ok {
            let fallback =
                if direction == 2 { format!("已關聯 {target}") } else { format!("已設定與 {target} 的相依") };
            self.show_msg(Some(StatusMessage::ok(message.borrow().clone().unwrap_or(fallback))));
            self.clone().load_deps(key).await;
        }
    }

    // ───────────── 附件 ─────────────

    async fn load_attachments(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        if self.st.borrow().attach_for.as_deref() == Some(key.as_str()) {
            return;
        }
        self.st.borrow_mut().attach_for = Some(key.clone());
        self.with_data(|d| d.set_d_attach(vec_model(vec![line(2, "讀取中…")])));
        let (items, web_url) = match self.engine().api().list_attachments(&key).await {
            Ok(r) => r,
            Err(e) => {
                log::error!("讀取 {key} 的附件失敗: {}", e.0);
                self.with_data(|d| d.set_d_attach(vec_model(vec![line(3, format!("讀取附件失敗：{}", e.0))])));
                self.st.borrow_mut().attach_for = None;
                return;
            }
        };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key) {
            return;
        }
        let now = Local::now().fixed_offset();
        let mut lines: Vec<TkLine> = Vec::new();
        if items.is_empty() {
            lines.push(line(2, "沒有附件"));
        }
        for a in &items {
            // 有 id 就在這裡直接看內容（圖片看圖、文件讀文字），沒有才開網頁
            let tip = if a.id.is_some() {
                "點一下直接看內容（圖片、PDF、Word、Excel、文字檔）"
            } else {
                "在網頁開啟"
            };
            lines.push(TkLine {
                kind: 1,
                key: tip.into(),
                text: format!("📎 {}", a.name).into(),
                meta: attachment_meta(a, &now).into(),
                ..Default::default()
            });
        }
        let has_web = web_url.is_some();
        {
            let mut st = self.st.borrow_mut();
            st.attachments = items;
            st.attach_web = web_url;
        }
        self.with_data(|d| {
            d.set_d_attach(vec_model(lines));
            d.set_d_attach_web(has_web);
        });
    }

    fn open_attachment(&self, index: usize) {
        let (item, web) = {
            let st = self.st.borrow();
            // 沒有附件時第一列是「沒有附件」
            (st.attachments.get(index).cloned(), st.attach_web.clone())
        };
        let Some(a) = item else { return };
        let url = a.url.clone().or(web);
        if let Some(id) = &a.id {
            let owner = self.ctx.window();
            hb_ui_common::file_preview::show(
                self.engine(),
                id,
                &a.name,
                owner.as_ref().map(|w| w.window()),
                url.as_deref(),
            );
        } else if let Some(url) = url {
            hb_win::open(&url);
        }
    }

    // ───────────── 文件 ─────────────

    async fn load_docs(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        if self.st.borrow().docs_for.as_deref() == Some(key.as_str()) {
            return;
        }
        self.st.borrow_mut().docs_for = Some(key.clone());
        self.with_data(|d| {
            d.set_d_docs(vec_model(Vec::new()));
            d.set_d_docs_lines(vec_model(vec![line(2, "讀取中…")]));
        });
        let docs = match self.engine().api().get_task_content(&key).await {
            Ok(d) => d,
            Err(e) => {
                log::error!("讀取 {key} 的文件失敗: {}", e.0);
                self.with_data(|d| d.set_d_docs_lines(vec_model(vec![line(3, format!("讀取文件失敗：{}", e.0))])));
                self.st.borrow_mut().docs_for = None;
                return;
            }
        };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key) {
            return;
        }
        let mut lines = Vec::new();
        if docs.is_empty() {
            lines.push(line(2, "沒有文件"));
        }
        lines.push(line(2, "唯讀；要編輯請在網頁開啟。"));
        let rows: Vec<TkDoc> = docs
            .iter()
            .map(|doc| TkDoc {
                title: doc.title.as_str().into(),
                text: if doc.text.is_empty() { "（空白）".into() } else { doc.text.as_str().into() },
            })
            .collect();
        self.with_data(|d| {
            d.set_d_docs(vec_model(rows));
            d.set_d_docs_lines(vec_model(lines));
        });
    }

    // ───────────── 異動紀錄 ─────────────

    /// 讀一頁（50 筆）接在後面；還有更多時最後放「載入更多」。
    async fn load_history(self: Rc<Self>, key: String) {
        let cursor = {
            let mut st = self.st.borrow_mut();
            st.history_lines.push(line(2, "讀取中…"));
            st.history_cursor.clone()
        };
        self.show_history();
        let result = self.engine().api().task_activity(&key, cursor.as_deref()).await;
        let drop_loading = |p: &Page| {
            let mut st = p.st.borrow_mut();
            if st.history_lines.last().is_some_and(|l| l.kind == 2 && l.text == "讀取中…") {
                st.history_lines.pop();
            }
        };
        let (items, next) = match result {
            Ok(r) => r,
            Err(e) => {
                log::error!("讀取 {key} 的異動紀錄失敗: {}", e.0);
                drop_loading(&self);
                {
                    let mut st = self.st.borrow_mut();
                    st.history_lines.push(line(3, format!("讀取異動紀錄失敗：{}", e.0)));
                    st.history_for = None;
                }
                self.show_history();
                return;
            }
        };
        if self.st.borrow().detail.as_ref().is_none_or(|d| d.key != key) {
            return;
        }
        drop_loading(&self);
        {
            let mut st = self.st.borrow_mut();
            if items.is_empty() && cursor.is_none() {
                st.history_lines.push(line(2, "沒有異動紀錄"));
            }
            for item in &items {
                st.history_lines.push(TkLine {
                    kind: 1,
                    text: item.text.as_str().into(),
                    meta: history_meta(item).into(),
                    ..Default::default()
                });
            }
            st.history_cursor = next.clone();
        }
        self.show_history();
        self.with_data(|d| d.set_d_history_more(next.is_some()));
    }

    fn show_history(&self) {
        let lines = self.st.borrow().history_lines.clone();
        self.with_data(|d| d.set_d_history(vec_model(lines)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dep(status: Option<&str>, rule: &str) -> DependencyItem {
        DependencyItem {
            id: "1".into(),
            other_key: "F2P-2".into(),
            other_title: "登入".into(),
            other_status: status.map(str::to_owned),
            rule: rule.into(),
            blocking: false,
        }
    }

    #[test]
    fn 相依說明() {
        assert_eq!("  · 進行中  · 完成後才能開始", dep_meta(&dep(Some("進行中"), "完成後才能開始"), true));
        assert_eq!("  · 進行中", dep_meta(&dep(Some("進行中"), "完成後才能開始"), false));
        assert_eq!("", dep_meta(&dep(None, ""), true));
    }

    #[test]
    fn 附件說明() {
        let at = |s: &str| chrono::DateTime::parse_from_rfc3339(s).unwrap();
        let now = at("2026-11-12T10:00:00+08:00");
        let a = hb_api::AttachmentItem {
            name: "a.png".into(),
            size: Some(2048),
            uploader: Some("amy".into()),
            at: Some(at("2026-11-12T09:55:00+08:00")),
            url: None,
            id: None,
        };
        assert_eq!("  · 2 KB · amy · 5 分鐘前", attachment_meta(&a, &now));
        let empty =
            hb_api::AttachmentItem { name: "b".into(), size: None, uploader: None, at: None, url: None, id: None };
        assert_eq!("  · ", attachment_meta(&empty, &now));
    }

    #[test]
    fn 異動紀錄沒有時間的空白() {
        let item = TaskActivityItem { at: hb_api::json::min_value(), actor: "amy".into(), text: "改了狀態".into() };
        assert_eq!("  amy  ", history_meta(&item));
    }
}
