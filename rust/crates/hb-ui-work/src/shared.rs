//! 工作台各頁共用的 Rust 小工具（Slint 元件在 `ui/shared/index.slint`）。
//!
//! - [`icon_item`]：專案圖示 → `ui/shared` 的 `IconItem`（`MiniSelect` 的 icons）。
//! - [`string_model`]、[`vec_model`]：Vec → Slint 模型；[`same_strings`]：內容一樣就不要換模型。
//! - [`spawn`]：`slint::spawn_local`，失敗只寫紀錄（C# `_ = XxxAsync()`）。
//! - [`status_parts`]：`hb_ui_common::inputs::run` 的訊息 → `StatusLine` 的文字與 tone（0 錯誤、1 成功）。
//!
//! 右鍵選單：`TaskMenuState` 整個工作台視窗共用一份，外框已經接好（`wire_task_menu!`），
//! 各頁用 [`crate::WorkCtx::set_task_menu_hooks`] 登記自己的「查看詳細」「刪除」「套用範本之後」，不要再自己 `wire_task_menu!`。

use std::future::Future;

use hb_api::ProjectInfo;
use hb_ui_common::ProjectIcons;
use hb_ui_common::inputs::StatusMessage;
use slint::{ModelRc, SharedString, VecModel};

use crate::IconItem;

/// 專案的圖示（找不到專案時用名稱的第一個字、代碼算顏色）。
pub fn icon_item(project: Option<&ProjectInfo>, name: &str) -> IconItem {
    let icon = ProjectIcons::icon(project, name);
    IconItem { glyph: icon.glyph.into(), tint: icon.color.to_slint(), has_icon: true }
}

/// 沒有圖示的一項（例「全部專案」）。
pub fn no_icon() -> IconItem {
    IconItem { glyph: SharedString::new(), tint: slint::Color::default(), has_icon: false }
}

pub fn string_model<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    let v: Vec<SharedString> = items.into_iter().map(|s| SharedString::from(s.as_ref())).collect();
    ModelRc::new(VecModel::from(v))
}

/// 模型裡的字串跟 items 一樣（一樣就不用換模型：換了 Segmented、下拉會重建，指示器會跳一下）。
pub fn same_strings<S: AsRef<str>>(model: &ModelRc<SharedString>, items: &[S]) -> bool {
    use slint::Model;
    model.row_count() == items.len()
        && items.iter().enumerate().all(|(i, s)| model.row_data(i).is_some_and(|m| m.as_str() == s.as_ref()))
}

pub fn vec_model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// 在 UI 執行緒背景跑（C# `_ = XxxAsync()`）；排不進去只寫紀錄。
pub fn spawn(what: &str, fut: impl Future<Output = ()> + 'static) {
    if let Err(e) = slint::spawn_local(fut) {
        log::error!("{what}失敗：{e}");
    }
}

/// 訊息列的文字與顏色（`StatusLine`：tone 0 錯誤、1 成功）。
pub fn status_parts(msg: Option<&StatusMessage>) -> (SharedString, i32) {
    match msg {
        Some(m) if m.visible() => (m.text.as_str().into(), if m.error { 0 } else { 1 }),
        _ => (SharedString::new(), 0),
    }
}
