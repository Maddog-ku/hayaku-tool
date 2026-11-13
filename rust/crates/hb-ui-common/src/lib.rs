//! 面板、工作台、設定共用的介面（對應 C# `UI/` 底下的共用元件與小視窗）。
//!
//! - 純邏輯：[`task_text`]（TaskText、KeyCopy）、[`project_icons`]、[`inputs`]（UI/Work/Inputs.cs）、[`markdown_table`]、
//!   [`view_models`]、[`day_timeline`]、[`date_range`]、[`task_menu`]。
//! - 共用 Slint 元件在 `ui/`（其他 crate 用 library path `hbc` 匯入 `@hbc/index.slint`）。
//! - 小視窗：每個一個模組，開窗函式見 `API.md`。
//!
//! 所有視窗函式都只能在 UI 執行緒（Slint 事件迴圈）呼叫。

pub mod date_range;
pub mod day_timeline;
pub mod inputs;
pub mod markdown_table;
pub mod project_icons;
pub mod task_menu;
pub mod task_text;
pub mod theme;
pub mod view_models;
pub mod window_util;

mod dialog;

pub mod add_test_result;
pub mod advance_tests;
pub mod apply_template;
pub mod batch_create;
pub mod delete_task;
pub mod feedback;
pub mod file_preview;
pub mod move_task;
pub mod poll;
pub mod quick_create;
pub mod reject_review;
pub mod task_picker;
pub mod test_action;

pub use day_timeline::{DayTimelineData, DayTimelineParts};
pub use markdown_table::MarkdownTable;
pub use project_icons::{ProjectIconData, ProjectIcons, Rgb};
pub use quick_create::{QuickCreate, QuickCreateResult};
pub use task_menu::{TaskMenu, TaskMenuHooks, TaskMenuSnapshot};
pub use task_text::{KeyCopy, TaskText};
pub use theme::set_theme;
pub use view_models::{ColorKey, MeetingRowVm, NotificationRowVm, ReminderRowVm, RepoRowVm, TaskRowVm, TodoRowVm};
pub use window_util::Placement;

/// 這個 crate 自己編譯的 Slint（ui/windows.slint）。
#[allow(clippy::all)]
pub(crate) mod ui {
    slint::include_modules!();
}

/// `ui/` 資料夾（library path `hbc` 要指向這裡）。
pub fn ui_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// 給每個視窗套 Theme（深色、減少動畫），以及開關動畫的狀態（ui/windows/dialog.slint 的 DialogMotion）。
pub(crate) trait Themed {
    fn apply_theme(&self, dark: bool, reduce_motion: bool);
    /// 視窗顯示了：內容開始依序出現
    fn set_motion_shown(&self, shown: bool);
    /// 要關了：內容淡出（之後由 window_util 藏起視窗）
    fn set_motion_closing(&self, closing: bool);
    fn motion_closing(&self) -> bool;
    fn reduce_motion(&self) -> bool;
}

macro_rules! impl_themed {
    ($($t:ty),* $(,)?) => {$(
        impl Themed for $t {
            fn apply_theme(&self, dark: bool, reduce_motion: bool) {
                use slint::ComponentHandle as _;
                let theme = self.global::<ui::Theme>();
                theme.set_dark(dark);
                theme.set_reduce_motion(reduce_motion);
            }
            fn set_motion_shown(&self, shown: bool) {
                use slint::ComponentHandle as _;
                self.global::<ui::DialogMotion>().set_shown(shown);
            }
            fn set_motion_closing(&self, closing: bool) {
                use slint::ComponentHandle as _;
                self.global::<ui::DialogMotion>().set_closing(closing);
            }
            fn motion_closing(&self) -> bool {
                use slint::ComponentHandle as _;
                self.global::<ui::DialogMotion>().get_closing()
            }
            fn reduce_motion(&self) -> bool {
                use slint::ComponentHandle as _;
                self.global::<ui::Theme>().get_reduce_motion()
            }
        }
    )*};
}

impl_themed!(
    ui::TestActionWindow,
    ui::DeleteTaskWindow,
    ui::AdvanceTestsWindow,
    ui::RejectReviewWindow,
    ui::BatchCreateWindow,
    ui::ApplyTemplateWindow,
    ui::AddTestResultWindow,
    ui::MoveTaskWindow,
    ui::TaskPickerWindow,
    ui::PollWindow,
    ui::FeedbackWindow,
    ui::FilePreviewWindow,
    ui::QuickCreateWindow,
);

/// window_util::present 要的 fn 指標。
pub(crate) fn apply_theme<T: Themed>(ui: &T, dark: bool, reduce_motion: bool) {
    ui.apply_theme(dark, reduce_motion);
}

#[cfg(test)]
mod tests {
    use crate::ui::{DayTimelineModel, TaskMenuState};

    /// day_timeline_model! 能轉成本 crate 編譯出的 DayTimelineModel（其他 crate 同理）。
    #[test]
    fn day_timeline_model_巨集() {
        let day = chrono::NaiveDate::from_ymd_opt(2026, 11, 12).unwrap();
        let data = crate::DayTimelineData::new(day, None, &[], day.and_hms_opt(10, 0, 0).unwrap(), true);
        let m: DayTimelineModel = crate::day_timeline_model!(DayTimelineModel, data);
        assert_eq!(10 * 60, m.from_min as i32);
    }

    /// wire_task_menu! 能接到本 crate 編譯出的 TaskMenuState（只檢查編譯）。
    #[allow(dead_code)]
    fn wire_task_menu_編譯(ui: &crate::ui::TestActionWindow, engine: &hb_core::TimerEngine) {
        let hooks = crate::TaskMenuHooks {
            open: std::rc::Rc::new(|_| {}),
            delete: std::rc::Rc::new(|_| {}),
            template_applied: None,
        };
        let _menu = crate::wire_task_menu!(ui, TaskMenuState, engine, hooks);
    }
}
