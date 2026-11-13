//! 對應 C# `UI/SmallDialog.cs` 的共用流程：按鈕全部停用、執行動作；失敗時顯示錯誤並恢復，成功時關掉視窗。
//! 外觀（卡片底、下面一排按鈕、訊息列）在 `ui/windows/dialog.slint`。

use std::future::Future;

use slint::ComponentHandle;

use crate::window_util;

/// 有 `busy`、`error` 兩個屬性的小視窗。
pub(crate) trait DialogUi: ComponentHandle + crate::Themed + 'static {
    fn set_busy_flag(&self, busy: bool);
    fn set_error_text(&self, text: &str);
}

macro_rules! impl_dialog_ui {
    ($($t:ty),* $(,)?) => {$(
        impl $crate::dialog::DialogUi for $t {
            fn set_busy_flag(&self, busy: bool) {
                self.set_busy(busy);
            }
            fn set_error_text(&self, text: &str) {
                self.set_error(text.into());
            }
        }
    )*};
}
pub(crate) use impl_dialog_ui;

/// 在 UI 執行緒跑一個 future；排不進去時寫紀錄。
pub(crate) fn spawn(fut: impl Future<Output = ()> + 'static) {
    if let Err(e) = slint::spawn_local(fut) {
        log::error!("無法在 UI 執行緒執行: {e}");
    }
}

/// C# `SmallDialog.RunAsync`：按鈕全部停用、執行動作；失敗時顯示錯誤並恢復，成功時關掉視窗。
/// title 是寫紀錄用的視窗標題（C# `Log.Error($"{Title}失敗", ex)`）。
pub(crate) fn run<T: DialogUi>(
    ui: &T,
    id: u64,
    title: &str,
    action: impl Future<Output = Result<(), String>> + 'static,
) {
    ui.set_busy_flag(true);
    let weak = ui.as_weak();
    let title = title.to_owned();
    spawn(async move {
        let result = action.await;
        let Some(ui) = weak.upgrade() else { return };
        match result {
            Ok(()) => window_util::close(&ui, id),
            Err(message) => {
                log::error!("{title}失敗: {message}");
                ui.set_error_text(&message);
                ui.set_busy_flag(false);
            }
        }
    });
}

/// 建立 Slint 視窗元件；失敗寫紀錄回 None。
pub(crate) fn create<T>(name: &str, new: impl FnOnce() -> Result<T, slint::PlatformError>) -> Option<T> {
    match new() {
        Ok(ui) => Some(ui),
        Err(e) => {
            log::error!("建立{name}視窗失敗: {e}");
            None
        }
    }
}
