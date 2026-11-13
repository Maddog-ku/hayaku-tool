//! 對應 C# `UI/AddTestResultWindow.cs`：在測試任務底下加入測試結果（QA 記一個問題）。
//! 名稱、內容（重現步驟）、分類、嚴重度、要誰修、關聯任務、期限，一開始是「要修」（待修復）或「之後再測」（未開始）。
//! 關聯任務先帶入測試任務的上層，也能快速選同一層的其他任務。截圖、附件 MCP 不能上傳，建好後到網頁貼。

use std::cell::RefCell;
use std::rc::Rc;

use hb_api::{KeyFinder, PersonRef, TaskKinds};
use hb_core::TimerEngine;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::dialog;
use crate::inputs;
use crate::ui::AddTestResultWindow;
use crate::window_util::{self, Placement};

/// 加好了：新測試結果的編號（回應裡找不到是空字串）（C# `event Action<string> Added`）。
pub type Added = Rc<dyn Fn(&str)>;

fn strings(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

/// 關聯任務欄位裡的編號（逗號、頓號、空白分開，去重複不分大小寫）。
pub fn related_keys(text: &str) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for part in text.split([',', '，', '、', ' ']).filter(|s| !s.is_empty()) {
        if let Some(k) = KeyFinder::parse(Some(part))
            && !keys.iter().any(|x| x.eq_ignore_ascii_case(&k))
        {
            keys.push(k);
        }
    }
    keys
}

/// 開「加入測試結果」。parent：測試任務的上層（預設的關聯任務）；siblings：同一層的其他任務（快速加為關聯）。
/// 同一個測試任務已經開著就拉到前面。
pub fn open(
    engine: &TimerEngine,
    test_task: &str,
    test_title: &str,
    parent: Option<&str>,
    siblings: &[(String, String)],
    added: Option<Added>,
) {
    let tag = format!("add:{test_task}");
    if window_util::activate_tag(&tag) {
        return;
    }
    let Some(ui) = dialog::create("加入測試結果", AddTestResultWindow::new) else { return };
    ui.set_heading(format!("🧪 {test_task} {test_title}").trim_end().into());
    ui.set_categories(strings(TaskKinds::CATEGORIES.iter().map(|s| s.to_string())));
    ui.set_category(0);
    ui.set_severities(strings(TaskKinds::SEVERITIES.iter().map(|s| s.to_string())));
    ui.set_severity(2);
    ui.set_related(parent.unwrap_or("").into());
    let others: Vec<&(String, String)> = siblings.iter().filter(|s| s.0 != test_task).collect();
    if !siblings.is_empty() {
        let mut items = vec!["＋ 加入同一層的任務…".to_owned()];
        items.extend(others.iter().map(|(k, t)| format!("{k} {t}")));
        ui.set_siblings(strings(items));
        ui.set_sibling(0);
    }
    let id = window_util::keep(&ui, Some(tag), ());
    let members: Rc<RefCell<Vec<PersonRef>>> = Rc::default();

    {
        let weak = ui.as_weak();
        ui.on_sibling_picked(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            if index <= 0 {
                return;
            }
            let item = ui.get_siblings().iter().nth(index as usize).map(|s| s.to_string());
            if let Some(key) = KeyFinder::parse(item.as_deref()) {
                let mut keys = related_keys(&ui.get_related());
                if !keys.iter().any(|k| k.eq_ignore_ascii_case(&key)) {
                    keys.push(key);
                    ui.set_related(keys.join(", ").into());
                }
            }
            ui.set_sibling(0);
        });
    }
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let test_task = test_task.to_owned();
        let members = members.clone();
        ui.on_add(move |again| {
            let Some(ui) = weak.upgrade() else { return };
            let title = ui.get_title_text().trim().to_owned();
            ui.set_notice("".into());
            if title.is_empty() {
                ui.set_error("請填名稱".into());
                ui.invoke_focus_title();
                return;
            }
            let due_text = ui.get_due().to_string();
            let mut due = None;
            if !due_text.trim().is_empty() {
                due = inputs::parse_date(Some(&due_text), None);
                if due.is_none() {
                    ui.set_error("期限看不懂，例 2026-11-05、11/5、明天、+3".into());
                    return;
                }
            }
            let who = ui.get_assignee().trim().to_owned();
            let assignee = members
                .borrow()
                .iter()
                .find(|m| m.display_name == who)
                .map(|m| m.username.clone())
                .or_else(|| (!who.is_empty()).then(|| who.clone()));
            let content = ui.get_content().trim().to_owned();
            let category = TaskKinds::CATEGORIES.get(ui.get_category().max(0) as usize).map(|s| s.to_string());
            let severity = TaskKinds::SEVERITIES.get(ui.get_severity().max(0) as usize).map(|s| s.to_string());
            let related = related_keys(&ui.get_related());
            let to_fix = ui.get_to_fix();
            ui.set_busy(true);
            ui.set_error("".into());
            let (engine, test_task, weak, added) = (engine.clone(), test_task.clone(), weak.clone(), added.clone());
            dialog::spawn(async move {
                let assignees = assignee.map(|a| vec![a]);
                let result = engine
                    .api()
                    .add_test_result(
                        &test_task,
                        &title,
                        Some(&content),
                        category.as_deref(),
                        severity.as_deref(),
                        assignees.as_deref(),
                        Some(&related),
                        to_fix,
                        due,
                    )
                    .await;
                let Some(ui) = weak.upgrade() else { return };
                match result {
                    Ok(key) => {
                        let key = key.unwrap_or_default();
                        log::info!("在 {test_task} 加入測試結果 {key}：{title}");
                        if let Some(f) = &added {
                            f(&key);
                        }
                        let e = engine.clone();
                        dialog::spawn(async move { e.after_external_change().await });
                        if !again {
                            window_util::close(&ui, id);
                            return;
                        }
                        ui.set_title_text("".into());
                        ui.set_content("".into());
                        ui.set_notice(format!("已加入 {key}，繼續記下一筆").into());
                        ui.invoke_focus_title();
                    }
                    Err(e) => {
                        log::error!("在 {test_task} 加入測試結果失敗: {}", e.0);
                        ui.set_error(e.0.into());
                    }
                }
                ui.set_busy(false);
            });
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                window_util::close(&ui, id);
            }
        });
    }
    window_util::present(&ui, id, Placement::Screen, true, (500.0, 760.0), crate::apply_theme);

    // 要誰修的選單：專案成員（專案資料有快取，通常不用多打請求）
    let weak = ui.as_weak();
    let engine = engine.clone();
    let test_task = test_task.to_owned();
    dialog::spawn(async move {
        match engine.get_project(KeyFinder::project_of(&test_task)).await {
            Ok(project) => {
                *members.borrow_mut() = project.members.clone();
                if let Some(ui) = weak.upgrade() {
                    ui.set_members(strings(project.members.iter().map(|m| m.display_name.clone())));
                }
            }
            Err(e) => log::error!("讀取 {test_task} 的專案成員失敗: {}", e.message()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 關聯任務_各種分隔與去重複() {
        assert_eq!(vec!["F2P-1", "EG-3"], related_keys("F2P-1, eg-3、f2p-1，  "));
        assert!(related_keys("沒有編號").is_empty());
    }
}
