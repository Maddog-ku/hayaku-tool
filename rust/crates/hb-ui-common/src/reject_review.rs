//! 對應 C# `UI/RejectReviewWindow.cs`：從審核提示按「駁回」開的小視窗。
//! 可以填給申請人的說明；駁回新增任務的申請會刪掉那個任務，先講清楚再送出。

use hb_api::{ChangeRequest, WorkMath};
use hb_core::TimerEngine;
use slint::ComponentHandle;

use crate::dialog;
use crate::ui::RejectReviewWindow;
use crate::window_util::{self, Placement};

/// 摘要第二段（申請人、改了什麼、理由）。
pub fn details(request: &ChangeRequest) -> String {
    let mut details =
        vec![format!("申請人：{}", request.requester.as_ref().map_or("（不明）", |p| p.display_name.as_str()))];
    if !request.changes.is_empty() {
        details.push(request.changes.join("、"));
    }
    if let Some(reason) = request.reason.as_deref().filter(|r| !r.trim().is_empty()) {
        details.push(format!("理由：{reason}"));
    }
    details.join("\n")
}

/// 駁回新增任務的申請時的紅字；其他類型是空字串。
pub fn warning(request: &ChangeRequest) -> String {
    if request.r#type != "CREATE" {
        return String::new();
    }
    let logged = if request.logged_minutes > 0 {
        format!("，連同已經記錄的 {} 工時", WorkMath::hours_minutes(request.logged_minutes as f64))
    } else {
        String::new()
    };
    format!("駁回會刪除這個任務{logged}。")
}

/// 開駁回小視窗（同一筆申請只開一個）。demo：截圖、檢查外觀用，按「駁回」不會真的送出。
/// 結果（成功或失敗）由引擎用提示告知，這裡送出後就關掉。
pub fn open(engine: &TimerEngine, request: &ChangeRequest, demo: bool) {
    let tag = format!("reject:{}", request.id);
    if window_util::activate_tag(&tag) {
        return;
    }
    let Some(ui) = dialog::create("駁回申請", RejectReviewWindow::new) else { return };
    ui.set_heading(
        format!("{}・{} {}", request.type_text(), request.task_key.as_deref().unwrap_or(""), request.task_title)
            .trim_end()
            .into(),
    );
    ui.set_details(details(request).into());
    ui.set_warning(warning(request).into());
    ui.set_button_text(if request.r#type == "CREATE" { "駁回並刪除任務" } else { "駁回" }.into());
    let id = window_util::keep(&ui, Some(tag), ());
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let request = request.clone();
        ui.on_reject(move |note| {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_busy(true);
            let (engine, request, weak) = (engine.clone(), request.clone(), weak.clone());
            let note = note.trim().to_owned();
            dialog::spawn(async move {
                if !demo {
                    engine.review(&request, false, Some(&note)).await;
                }
                if let Some(ui) = weak.upgrade() {
                    window_util::close(&ui, id);
                }
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
    window_util::present(&ui, id, Placement::Screen, true, (440.0, 300.0), crate::apply_theme);
}
