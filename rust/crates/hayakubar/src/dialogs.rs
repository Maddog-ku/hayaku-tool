//! C# `MessageBox.Show` 的對應：確認（確定／取消）、提示、警告、錯誤。用 rfd 的系統對話框，擺在 owner 視窗上。

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

#[derive(Clone, Copy)]
pub enum Level {
    Info,
    Warning,
    Error,
}

fn dialog(owner: Option<&slint::Window>, title: &str, text: &str, level: Level) -> rfd::MessageDialog {
    let mut d = rfd::MessageDialog::new().set_title(title).set_description(text).set_level(match level {
        Level::Info => rfd::MessageLevel::Info,
        Level::Warning => rfd::MessageLevel::Warning,
        Level::Error => rfd::MessageLevel::Error,
    });
    if let Some(owner) = owner {
        let handle = owner.window_handle();
        if handle.window_handle().is_ok() && handle.display_handle().is_ok() {
            d = d.set_parent(&handle);
        }
    }
    d
}

/// 確定／取消；按了確定回 true（C# `MessageBoxButton.OKCancel` + `MessageBoxImage.Question`）。
pub fn confirm(owner: Option<&slint::Window>, title: &str, text: &str) -> bool {
    let result = dialog(owner, title, text, Level::Info).set_buttons(rfd::MessageButtons::OkCancel).show();
    matches!(result, rfd::MessageDialogResult::Ok | rfd::MessageDialogResult::Yes)
}

/// 只有確定的訊息。
pub fn show(owner: Option<&slint::Window>, title: &str, text: &str, level: Level) {
    let _ = dialog(owner, title, text, level).set_buttons(rfd::MessageButtons::Ok).show();
}
