//! 對應 C# `UI/FeedbackWindow.cs`：問題回報的全文（get_feedback，要「問題回報」權限）。
//! 說明、處理說明、環境、處理紀錄、建立的任務，截圖點了直接看。處理（轉任務、改狀態）MCP 做不到，留「到網頁處理」。

use chrono::Local;
use hb_api::FeedbackDetail;
use hb_core::TimerEngine;
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::dialog;
use crate::ui::FeedbackWindow;
use crate::window_util::{self, Placement};

fn strings(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    ModelRc::new(VecModel::from(items.into_iter().map(SharedString::from).collect::<Vec<_>>()))
}

/// 標題下面那一行（狀態・回報人・建立時間）。
pub fn meta(f: &FeedbackDetail) -> String {
    [
        Some(f.status.clone()),
        f.reporter.clone(),
        f.created_at.map(|at| at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string()),
    ]
    .into_iter()
    .flatten()
    .filter(|x| !x.is_empty())
    .collect::<Vec<_>>()
    .join("・")
}

/// 截圖按鈕的文字（「🖼 截圖 N」）。
pub fn screenshot_label(n: usize) -> String {
    format!("🖼 截圖 {n}")
}

/// 打開截圖時的預覽視窗標題：C# 是 `$"#{f.Number} {label[2..]}"`（label 去掉前兩個 UTF-16 字元，也就是 🖼，留下前面的空白）。
pub fn screenshot_title(number: i32, n: usize) -> String {
    format!("#{number}  截圖 {n}")
}

/// 開問題回報 #number；同一則已經開著就拉到前面（C# App.OpenWorkspace("feedback")）。
pub fn open(engine: &TimerEngine, number: i32) {
    let tag = format!("feedback:{number}");
    if window_util::activate_tag(&tag) {
        return;
    }
    let Some(ui) = dialog::create("問題回報", FeedbackWindow::new) else { return };
    ui.set_window_title(format!("問題回報 #{number}").into());
    let id = window_util::keep(&ui, Some(tag), ());
    let shots: std::rc::Rc<std::cell::RefCell<Vec<String>>> = Default::default();
    {
        let (weak, engine, shots) = (ui.as_weak(), engine.clone(), shots.clone());
        ui.on_open_screenshot(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let n = i.max(0) as usize;
            if let Some(file) = shots.borrow().get(n) {
                crate::file_preview::show(&engine, file, &screenshot_title(number, n + 1), Some(ui.window()), None);
            }
        });
    }
    {
        let engine = engine.clone();
        ui.on_open_task(move |key| engine.open_task_in_workspace(&key));
    }
    {
        let engine = engine.clone();
        ui.on_open_web(move || hb_win::open(&engine.web_url("/notifications")));
    }
    window_util::present(&ui, id, Placement::Screen, false, (640.0, 620.0), crate::apply_theme);

    let weak = ui.as_weak();
    let engine = engine.clone();
    dialog::spawn(async move {
        let result = engine.api().get_feedback(number).await;
        let Some(ui) = weak.upgrade() else { return };
        match result {
            Err(e) => {
                log::error!("讀取問題回報 #{number} 失敗: {}", e.0);
                ui.set_error(format!("讀不到：{}（要有「問題回報」權限）", e.0).into());
            }
            Ok(f) => {
                ui.set_heading(format!("#{} {}", f.number, f.title).into());
                ui.set_meta(meta(&f).into());
                ui.set_body_text(if f.body.is_empty() { "（沒有說明）".into() } else { f.body.as_str().into() });
                ui.set_resolution(f.resolution.as_deref().filter(|r| !r.trim().is_empty()).unwrap_or("").into());
                ui.set_environment(strings(f.environment.iter().map(|(label, value)| format!("{label}：{value}"))));
                ui.set_screenshots(strings((1..=f.screenshot_ids.len()).map(screenshot_label)));
                *shots.borrow_mut() = f.screenshot_ids.clone();
                ui.set_created_tasks(strings(f.created_tasks.iter().cloned()));
                ui.set_history(strings(f.history.iter().cloned()));
                ui.set_loading(false);
            }
        }
    });
}
