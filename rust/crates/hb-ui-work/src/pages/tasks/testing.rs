//! 對應 C# `UI/Work/TasksPage.Testing.cs`：任務詳細的測試流程與鍵盤。
//! 測試結果顯示分類、嚴重度、第幾輪、回報的人，依狀態給〔開始修〕〔送復驗〕〔通過〕〔退回〕〔不處理〕；
//! 測試任務顯示底下測試結果的進度與各狀態數量，子任務區變成「測試結果」（可以搜尋、依狀態篩選），〔加入測試結果〕（R）。
//! ↑↓ 在同一層的任務之間移動（已完成的也跳得到）。

use std::rc::Rc;

use hb_api::{KeyFinder, TaskDetail, TaskKind, TaskKinds, TestSubStatus};
use hb_core::{CoreError, TestActionRequest, TimerEngine};
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use serde_json::json;
use slint::platform::Key;

use super::Page;
use crate::shared::{self, vec_model};
use crate::{TasksPageData, TkAction, TkChild};

/// 測試結果的分組（篩選與排序）：全部、修復中、待復驗、測試中、通過、不處理。
pub(super) const BUCKETS: [(&str, &str); 6] = [
    ("", "全部"),
    ("fixing", "修復中"),
    ("retest", "待復驗"),
    ("testing", "測試中"),
    ("pass", "通過"),
    ("closed", "不處理"),
];

/// 測試結果那一行：🐞 測試結果・分類・嚴重度・第幾輪・回報的人。
pub(super) fn result_line(d: &TaskDetail) -> String {
    let info = &d.test;
    let mut parts = vec!["🐞 測試結果".to_owned()];
    if let Some(c) = &info.category {
        parts.push(c.clone());
    }
    if let Some(s) = &info.severity {
        parts.push(format!("嚴重度 {s}"));
    }
    if info.round > 0 {
        parts.push(if info.hot() { format!("第 {} 輪 🔥", info.round) } else { format!("第 {} 輪", info.round) });
    }
    if let Some(r) = reporter_of(d) {
        parts.push(format!("回報：{r}"));
    }
    parts.join("・")
}

/// 回報的人：測試資訊沒有就用建立的人。
fn reporter_of(d: &TaskDetail) -> Option<String> {
    d.test.reporter.clone().or_else(|| d.created_by.clone()).filter(|r| !r.is_empty())
}

/// 測試任務的進度：(標題, 已結案比例, 有沒有進度條, 各狀態數量)。
pub(super) fn test_task_summary(children: &[(String, String, String)]) -> (String, f32, bool, String) {
    let count = |bucket: &str| children.iter().filter(|c| TaskKinds::bucket(&c.2, None, None) == bucket).count();
    let total = children.len();
    let finished = count("pass") + count("closed");
    let title = if total == 0 {
        "🧪 測試任務・還沒有測試結果".to_owned()
    } else {
        format!("🧪 測試任務・{finished}/{total} 已結案")
    };
    let counts = BUCKETS[1..]
        .iter()
        .filter(|b| count(b.0) > 0)
        .map(|b| format!("{} {}", b.1, count(b.0)))
        .collect::<Vec<_>>()
        .join("　");
    (title, if total == 0 { 0.0 } else { finished as f32 / total as f32 }, total > 0, counts)
}

/// 子任務清單；測試任務的是測試結果：用名稱或編號搜尋、依狀態篩選，照狀態分組排。
pub(super) fn filter_children(
    children: &[(String, String, String)],
    is_test: bool,
    query: &str,
    bucket: &str,
) -> Vec<(String, String, String)> {
    if !is_test {
        return children.to_vec();
    }
    let q = query.trim().to_lowercase();
    let bucket_index = |status: &str| {
        BUCKETS.iter().position(|b| b.0 == TaskKinds::bucket(status, None, None)).map_or(-1, |i| i as i32)
    };
    let mut list: Vec<(String, String, String)> = children
        .iter()
        .filter(|c| q.is_empty() || c.0.to_lowercase().contains(&q) || c.1.to_lowercase().contains(&q))
        .filter(|c| bucket.is_empty() || TaskKinds::bucket(&c.2, None, None) == bucket)
        .cloned()
        .collect();
    list.sort_by_key(|c| bucket_index(&c.2));
    list
}

/// 測試結果的按鈕（C# RenderResultPanel）：(按鈕, 都沒有按鈕時的說明)。
pub(super) fn result_actions(engine: &TimerEngine, d: &TaskDetail) -> (Vec<TkAction>, String) {
    let info = &d.test;
    let reporter = reporter_of(d);
    let bucket = TaskKinds::bucket(&d.status, Some(&d.status_category), info.stage.as_deref());
    let stage = TaskKinds::stage_of(&d.status, info.stage.as_deref());
    let can_retest = engine.can_retest(&d.key, reporter.as_deref());
    // 子狀態（例 待更新測試機、待上正式機）：更新好了推到下一步，不是開始修或通過
    let batch = engine.batch_step(&d.key, &d.status);
    let mut actions: Vec<TkAction> = Vec::new();
    let mut add = |id: &str, text: String, tip: String, primary: bool| {
        actions.push(TkAction { id: id.into(), text: text.into(), tip: tip.into(), primary });
    };
    if let Some(b) = &batch {
        add(
            "batch",
            format!("⇢ {}", b.batch_text().unwrap_or_default()),
            format!("{}：更新好了就推到下一步，可以填更新說明（例版本號）", b.name),
            true,
        );
    } else if d.status == "待修復" || stage == "待修復" {
        add("fix", "🔧 開始修".into(), "狀態改成修復中，可以順手填預估工時".into(), true);
    } else if stage == TimerEngine::STATUS_FIXING {
        match engine.fix_done_step(&d.key, &d.status) {
            Some(next) => add(
                "submit",
                "✓ 修好了，待更新".into(),
                format!("改成{}", next.name)
                    + &next.batch_text().map(|b| format!("，{b}後再送復驗")).unwrap_or_default(),
                true,
            ),
            None => add("submit", "✓ 送復驗".into(), "修好了，請回報的人驗".into(), true),
        }
    } else if bucket == "fixing" && d.status != TimerEngine::STATUS_FIXING {
        add("fix", "🔧 開始修".into(), "狀態改成修復中，可以順手填預估工時".into(), true);
    }
    if can_retest && matches!(bucket, "retest" | "testing") {
        if batch.is_none() {
            let tip = match engine.pass_step(&d.key, &d.status) {
                Some(next) => format!("復驗通過，改成{}（還不算完成）", next.name),
                None => "復驗通過，結案（已完成）".to_owned(),
            };
            add("pass", "✓ 通過".into(), tip, true);
        }
        add("reopen", "↩ 退回".into(), "還有問題，退回給工程師再修（輪數 +1）".into(), false);
    }
    if can_retest && !matches!(bucket, "pass" | "closed") {
        add("wontfix", "不處理".into(), "不修了，結案".into(), false);
    }
    let note = if actions.is_empty() && !can_retest && bucket == "retest" {
        "等回報的人（或專案管理員）復驗。"
    } else {
        ""
    };
    (actions, note.to_owned())
}

impl Page {
    pub(super) fn wire_testing(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_d_type_selected, |p, i| p.type_selected(i as usize));
        on!(d, self, on_d_test_action, |p, id| p.test_action(id.to_string()));
        on!(d, self, on_d_add_test_result, |p| {
            if let Some(d) = p.st.borrow().detail.clone() {
                p.open_add_test_result(&d);
            }
        });
        on!(d, self, on_d_result_filter_edited, |p, t| {
            p.st.borrow_mut().result_filter = t.to_string();
            p.rerender_children();
        });
        on!(d, self, on_d_bucket_selected, |p, i| {
            p.st.borrow_mut().result_bucket = BUCKETS.get(i as usize).map_or("", |b| b.0).to_owned();
            p.rerender_children();
        });
    }

    pub(super) fn render_testing(&self, d: &TaskDetail) {
        let reset = {
            let mut st = self.st.borrow_mut();
            st.children_of.insert(d.key.to_uppercase(), d.children.clone());
            let reset = st.result_filter_for.as_deref() != Some(d.key.as_str());
            if reset {
                st.result_filter_for = Some(d.key.clone());
                st.result_filter.clear();
                st.result_bucket.clear();
            }
            reset
        };
        let kind = d.kind();
        self.with_data(|x| {
            if reset {
                x.set_d_result_filter(Default::default());
                x.set_d_bucket_index(0);
            }
            x.set_d_type_index(match kind {
                TaskKind::Normal => 0,
                TaskKind::Test => 1,
                TaskKind::TestResult => 2,
            });
            x.set_d_test_kind(match kind {
                TaskKind::Normal => 0,
                TaskKind::Test => 1,
                TaskKind::TestResult => 2,
            });
            x.set_d_children_header(
                if kind == TaskKind::Test {
                    format!("測試結果（{}）", d.children.len())
                } else {
                    "子任務".to_owned()
                }
                .into(),
            );
        });
        match kind {
            TaskKind::TestResult => {
                self.render_result_panel(d);
                if self.engine().try_get_cached_project(KeyFinder::project_of(&d.key)).is_none()
                    && let Some(p) = self.rc()
                {
                    let d = d.clone();
                    shared::spawn("讀取測試子狀態", async move { p.load_sub_statuses(d).await });
                }
            }
            TaskKind::Test => self.render_test_task_panel(d),
            TaskKind::Normal => {}
        }
        self.render_children(d);
    }

    fn render_result_panel(&self, d: &TaskDetail) {
        let info = &d.test;
        let tone = if info.hot() {
            1
        } else {
            match TaskKinds::severity_brush(info.severity.as_deref()) {
                "ErrorBrush" => 1,
                "MutedBrush" => 2,
                _ => 0,
            }
        };
        let (actions, note) = result_actions(self.engine(), d);
        self.st.borrow_mut().test_batch = self.engine().batch_step(&d.key, &d.status);
        let parent = d.path.last().map(|p| p.0.clone()).unwrap_or_default();
        self.with_data(|x| {
            x.set_d_test_line(result_line(d).into());
            x.set_d_test_tone(tone);
            x.set_d_test_bold(info.hot());
            x.set_d_test_parent(parent.into());
            x.set_d_test_actions(vec_model(actions));
            x.set_d_test_note(note.into());
        });
    }

    /// 測試結果的按鈕要看專案的子狀態：專案還沒讀過就讀一次（每個專案只讀一次），讀到後重畫這一塊。
    async fn load_sub_statuses(self: Rc<Self>, d: TaskDetail) {
        if !self.engine().load_projects_of([d.key.as_str()]).await
            || self.st.borrow().detail.as_ref().is_none_or(|x| x.key != d.key)
            || self.engine().cached_sub_statuses(&d.key).is_empty()
        {
            return;
        }
        self.render_result_panel(&d);
    }

    fn render_test_task_panel(&self, d: &TaskDetail) {
        let (summary, progress, has_progress, counts) = test_task_summary(&d.children);
        self.with_data(|x| {
            x.set_d_test_summary(summary.into());
            x.set_d_test_progress(progress);
            x.set_d_test_has_progress(has_progress);
            x.set_d_test_counts(counts.into());
        });
    }

    fn test_action(&self, id: String) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let Some(p) = self.rc() else { return };
        let done: hb_ui_common::test_action::Done = {
            let weak = self.me.clone();
            let key = d.key.clone();
            Rc::new(move |message: &str| {
                if let Some(p) = weak.upgrade() {
                    p.show_msg(Some(StatusMessage::ok(message)));
                    let key = key.clone();
                    shared::spawn("重新讀取任務", async move { p.refresh_detail_quietly(&key).await });
                }
            })
        };
        match id.as_str() {
            "batch" => {
                let Some(step) = self.st.borrow().test_batch.clone() else { return };
                self.open_advance(&d, &step, done);
            }
            "fix" | "reopen" => {
                let request = TestActionRequest { action: id.clone(), key: d.key.clone(), title: d.title.clone() };
                hb_ui_common::test_action::open(self.engine(), &request, Some(done));
            }
            "submit" => {
                let engine = self.engine().clone();
                shared::spawn("送復驗", async move {
                    let (key, status) = (d.key.clone(), d.status.clone());
                    p.run_test(&d.key, move || async move { engine.submit_retest(&key, Some(&status)).await }).await
                });
            }
            "pass" => {
                let engine = self.engine().clone();
                shared::spawn("復驗通過", async move {
                    let key = d.key.clone();
                    p.run_test(&d.key, move || async move { engine.retest(&key, true, None).await }).await
                });
            }
            "wontfix" => {
                let engine = self.engine().clone();
                shared::spawn("不處理", async move {
                    let text = format!("{} 標為不處理（結案，不修了）？", d.key);
                    if p.confirm("不處理", &text, &["確定", "取消"]).await == 0 {
                        let key = d.key.clone();
                        p.run_test(&d.key, move || async move { engine.wont_fix(&key).await }).await;
                    }
                });
            }
            other => log::error!("測試結果沒有這個動作：{other}"),
        }
    }

    fn open_advance(&self, d: &TaskDetail, step: &TestSubStatus, done: hb_ui_common::test_action::Done) {
        hb_ui_common::advance_tests::open(
            self.engine(),
            KeyFinder::project_of(&d.key),
            step,
            Some(vec![d.key.clone()]),
            Some(done),
        );
    }

    async fn run_test<F, Fut>(&self, key: &str, action: F)
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<String, CoreError>>,
    {
        let message: Rc<std::cell::RefCell<String>> = Rc::default();
        let m = message.clone();
        let ok = inputs::run(
            |_| {},
            |msg| self.show_msg(msg),
            move || async move {
                *m.borrow_mut() = action().await.map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if ok {
            self.show_msg(Some(StatusMessage::ok(message.borrow().clone())));
            self.refresh_detail_quietly(key).await;
        }
    }

    /// 加入測試結果：關聯任務先帶入測試任務的上層，同一層的其他任務可以快速選。
    pub(super) fn open_add_test_result(&self, d: &TaskDetail) {
        let parent = d.path.last().map(|p| p.0.clone());
        let siblings: Vec<(String, String)> = parent
            .as_ref()
            .and_then(|p| self.st.borrow().children_of.get(&p.to_uppercase()).cloned())
            .map(|list| list.into_iter().map(|c| (c.0, c.1)).collect())
            .unwrap_or_default();
        let added: hb_ui_common::add_test_result::Added = {
            let weak = self.me.clone();
            let key = d.key.clone();
            Rc::new(move |new_key: &str| {
                if let Some(p) = weak.upgrade() {
                    let text = if new_key.is_empty() {
                        "已加入測試結果".to_owned()
                    } else {
                        format!("已加入測試結果 {new_key}")
                    };
                    p.show_msg(Some(StatusMessage::ok(text)));
                    let key = key.clone();
                    shared::spawn("重新讀取任務", async move { p.refresh_detail_quietly(&key).await });
                }
            })
        };
        hb_ui_common::add_test_result::open(self.engine(), &d.key, &d.title, parent.as_deref(), &siblings, Some(added));
    }

    fn rerender_children(&self) {
        if let Some(d) = self.st.borrow().detail.clone() {
            self.render_children(&d);
        }
    }

    /// 子任務清單；測試任務的是測試結果，可以用名稱或編號搜尋、依狀態篩選，照狀態分組排。
    fn render_children(&self, d: &TaskDetail) {
        let is_test = d.kind() == TaskKind::Test;
        let (q, bucket) = {
            let st = self.st.borrow();
            (st.result_filter.clone(), st.result_bucket.clone())
        };
        let rows: Vec<TkChild> = filter_children(&d.children, is_test, &q, &bucket)
            .into_iter()
            .map(|c| TkChild {
                key: c.0.into(),
                title: if is_test { format!("🐞 {}", c.1) } else { c.1 }.into(),
                status: format!("· {}", c.2).into(),
            })
            .collect();
        self.with_data(|x| {
            x.set_d_children(vec_model(rows));
            x.set_d_children_visible(!d.children.is_empty());
            x.set_d_result_filter_visible(is_test && d.children.len() > 3);
        });
    }

    fn type_selected(&self, index: usize) {
        let kinds = [TaskKind::Normal, TaskKind::Test, TaskKind::TestResult];
        let Some(kind) = kinds.get(index) else { return };
        let label = TaskKinds::label(*kind);
        if self.st.borrow().detail.as_ref().is_none_or(|d| TaskKinds::label(d.kind()) == label) {
            return;
        }
        let Some(p) = self.rc() else { return };
        let fields = json!({ "type": label }).as_object().cloned().unwrap_or_default();
        let what = format!("類型（{label}）");
        shared::spawn("更新類型", async move { p.update(fields, &what).await });
    }

    /// 詳細開著、焦點不在輸入框時：↑↓ 同一層的上一個／下一個任務，R 在測試任務加入測試結果。
    pub(super) fn dialog_key(&self, e: &slint::private_unstable_api::re_exports::KeyEvent) -> bool {
        let m = &e.modifiers;
        if m.control || m.alt || m.shift || m.meta {
            return false;
        }
        let Some(d) = self.st.borrow().detail.clone() else { return false };
        if self.with_data(|x| x.get_dialog() != 1 || !x.get_detail_ready()).unwrap_or(true) {
            return false;
        }
        let up: slint::SharedString = Key::UpArrow.into();
        let down: slint::SharedString = Key::DownArrow.into();
        if e.text == up || e.text == down {
            let step = if e.text == down { 1 } else { -1 };
            if let Some(p) = self.rc() {
                shared::spawn("切換同一層的任務", async move { p.move_sibling(step).await });
            }
            return true;
        }
        if (e.text == "r" || e.text == "R") && d.kind() == TaskKind::Test {
            self.open_add_test_result(&d);
            return true;
        }
        false
    }

    /// 同一層的上一個／下一個任務：有上層的照上層的子任務順序（含已完成的；上層沒讀過就讀一次）；
    /// 最上層的照我的任務裡同專案、也在最上層的。
    async fn move_sibling(self: Rc<Self>, step: i32) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        if self.st.borrow().moving_sibling {
            return;
        }
        self.st.borrow_mut().moving_sibling = true;
        let key = d.key.clone();
        let result: Result<Vec<String>, String> = async {
            match d.path.last().map(|p| p.0.clone()) {
                Some(parent) => {
                    let cached = self.st.borrow().children_of.get(&parent.to_uppercase()).cloned();
                    let children = match cached {
                        Some(c) => c,
                        None => {
                            self.show_msg(Some(StatusMessage::ok("讀取同一層的任務…")));
                            let detail = self.engine().api().get_task(&parent).await.map_err(|e| e.0)?;
                            self.st.borrow_mut().children_of.insert(parent.to_uppercase(), detail.children.clone());
                            detail.children
                        }
                    };
                    Ok(children.into_iter().map(|c| c.0).collect())
                }
                None => {
                    let project = KeyFinder::project_of(&key).to_owned();
                    let mut seen = hb_core::CiSet::new();
                    let all = self.engine().task_list(TimerEngine::ALL_TAB);
                    let done = self.st.borrow().done_tasks.clone();
                    let mut keys: Vec<String> = all
                        .iter()
                        .chain(done.iter())
                        .filter(|t| {
                            t.ancestors.as_ref().is_none_or(|a| a.is_empty())
                                && KeyFinder::project_of(&t.key).eq_ignore_ascii_case(&project)
                        })
                        .filter(|t| seen.insert(t.key.clone()))
                        .map(|t| t.key.clone())
                        .collect();
                    keys.sort_by_key(|k| super::vm::key_number(k));
                    Ok(keys)
                }
            }
        }
        .await;
        match result {
            Ok(keys) => {
                let index = keys.iter().position(|k| k.eq_ignore_ascii_case(&key));
                let next = index.map(|i| i as i64 + step as i64);
                match (index, next) {
                    (Some(_), Some(n)) if n >= 0 && (n as usize) < keys.len() => {
                        if self.st.borrow().detail.as_ref().is_some_and(|x| x.key == key) {
                            let next_key = keys[n as usize].clone();
                            self.clone().open_task(next_key).await;
                        }
                    }
                    _ => self.show_msg(Some(StatusMessage::ok(if step > 0 {
                        "已經是這一層的最後一個"
                    } else {
                        "已經是這一層的第一個"
                    }))),
                }
            }
            Err(message) => {
                log::error!("切換同一層的任務失敗: {message}");
                self.show_msg(Some(StatusMessage::error(format!("讀不到同一層的任務：{message}"))));
            }
        }
        self.st.borrow_mut().moving_sibling = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn child(key: &str, title: &str, status: &str) -> (String, String, String) {
        (key.into(), title.into(), status.into())
    }

    #[test]
    fn 測試任務進度() {
        let (title, progress, has, counts) = test_task_summary(&[]);
        assert_eq!("🧪 測試任務・還沒有測試結果", title);
        assert!(!has);
        assert_eq!(0.0, progress);
        assert_eq!("", counts);
        let children = vec![
            child("Q-1", "a", "已完成"),
            child("Q-2", "b", "修復中"),
            child("Q-3", "c", "不處理"),
            child("Q-4", "d", "待修復"),
        ];
        let (title, progress, has, counts) = test_task_summary(&children);
        assert_eq!("🧪 測試任務・2/4 已結案", title);
        assert!(has);
        assert_eq!(0.5, progress);
        assert_eq!("修復中 2　通過 1　不處理 1", counts);
    }

    #[test]
    fn 測試結果搜尋篩選與照狀態排() {
        let children = vec![
            child("Q-1", "登入跑版", "已完成"),
            child("Q-2", "按鈕", "修復中"),
            child("Q-3", "登入閃退", "待復驗"),
        ];
        let keys = |v: Vec<(String, String, String)>| v.into_iter().map(|c| c.0).collect::<Vec<_>>();
        assert_eq!(vec!["Q-2", "Q-3", "Q-1"], keys(filter_children(&children, true, "", "")));
        assert_eq!(vec!["Q-3", "Q-1"], keys(filter_children(&children, true, "登入", "")));
        assert_eq!(vec!["Q-2"], keys(filter_children(&children, true, "q-2", "")));
        assert_eq!(vec!["Q-1"], keys(filter_children(&children, true, "", "pass")));
        // 一般任務不篩、不排
        assert_eq!(vec!["Q-1", "Q-2", "Q-3"], keys(filter_children(&children, false, "登入", "pass")));
    }

    #[test]
    fn 測試結果那一行() {
        let mut d = hb_api::HayakuApi::parse_task_detail(&serde_json::json!({ "task": "Q-9" }));
        d.test = hb_api::TestInfo::new(TaskKind::TestResult, Some("Bug"), Some("嚴重"), 3, Some("amy"));
        assert_eq!("🐞 測試結果・Bug・嚴重度 嚴重・第 3 輪 🔥・回報：amy", result_line(&d));
        d.test = hb_api::TestInfo::new(TaskKind::TestResult, None, None, 0, None);
        d.created_by = Some("bob".into());
        assert_eq!("🐞 測試結果・回報：bob", result_line(&d));
    }
}
