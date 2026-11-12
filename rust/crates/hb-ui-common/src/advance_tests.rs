//! 對應 C# `UI/AdvanceTestsWindow.cs`：測試結果批次推到下一步的小視窗
//! （例〔已更新到測試機〕：待更新測試機 → 待復驗，通知回報的人復驗）。
//! 可以填更新說明（例版本號），會變成每筆的留言、也放進通知。tasks 為 None 是專案裡那個子狀態的全部。

use hb_api::TestSubStatus;
use hb_core::TimerEngine;
use slint::ComponentHandle;

use crate::dialog;
use crate::test_action::Done;
use crate::ui::AdvanceTestsWindow;
use crate::window_util::{self, Placement};

dialog::impl_dialog_ui!(AdvanceTestsWindow);

/// 更新說明最多 200 字（C# TextBox.MaxLength，以 UTF-16 計）。
const MAX_COMMENT: usize = 200;

/// 標題列的「做什麼：哪些」。
pub fn heading(project: &str, from: &TestSubStatus, tasks: Option<&[String]>) -> String {
    let what = match tasks {
        None => format!("{project} 所有「{}」的測試結果", from.name),
        Some([only]) => format!("{only}（{}）", from.name),
        Some(t) => format!("{project}「{}」的 {} 筆測試結果", from.name, t.len()),
    };
    format!("{}：{what}", from.batch_text().unwrap_or_default())
}

pub fn open(engine: &TimerEngine, project: &str, from: &TestSubStatus, tasks: Option<Vec<String>>, done: Option<Done>) {
    let Some(ui) = dialog::create("推到下一步", AdvanceTestsWindow::new) else { return };
    let title = from.batch_text().unwrap_or_else(|| "推到下一步".to_owned());
    ui.set_window_title(title.as_str().into());
    ui.set_heading(heading(project, from, tasks.as_deref()).into());
    if let Some(t) = tasks.as_ref().filter(|t| t.len() > 1 && t.len() <= 12) {
        ui.set_keys(t.join("、").into());
    }
    ui.set_button_text(from.batch_text().unwrap_or_else(|| "確定".to_owned()).into());
    let id = window_util::keep(&ui, None, ());
    {
        let weak = ui.as_weak();
        ui.on_comment_edited(move |text| {
            if text.encode_utf16().count() > MAX_COMMENT
                && let Some(ui) = weak.upgrade()
            {
                let units: Vec<u16> = text.encode_utf16().take(MAX_COMMENT).collect();
                ui.set_comment(String::from_utf16_lossy(&units).into());
            }
        });
    }
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let project = project.to_owned();
        let from = from.clone();
        ui.on_advance(move |comment| {
            let Some(ui) = weak.upgrade() else { return };
            let (engine, project, from, tasks, done) =
                (engine.clone(), project.clone(), from.clone(), tasks.clone(), done.clone());
            let comment = comment.to_string();
            dialog::run(&ui, id, &title, async move {
                let message = engine
                    .advance_test_results(&project, &from, tasks.as_deref(), Some(&comment))
                    .await
                    .map_err(|e| e.message().to_owned())?;
                if let Some(done) = done {
                    done(&message);
                }
                Ok(())
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
    window_util::present(&ui, id, Placement::Screen, true, (440.0, 260.0), crate::apply_theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 標題依任務數() {
        let from = TestSubStatus::new("待修復", "待更新測試機", Some("更新到測試機"), None);
        let batch = from.batch_text().unwrap_or_default();
        assert_eq!(format!("{batch}：F2P 所有「待更新測試機」的測試結果"), heading("F2P", &from, None));
        assert_eq!(format!("{batch}：F2P-1（待更新測試機）"), heading("F2P", &from, Some(&["F2P-1".to_owned()])));
        assert_eq!(
            format!("{batch}：F2P「待更新測試機」的 2 筆測試結果"),
            heading("F2P", &from, Some(&["F2P-1".to_owned(), "F2P-2".to_owned()]))
        );
    }
}
