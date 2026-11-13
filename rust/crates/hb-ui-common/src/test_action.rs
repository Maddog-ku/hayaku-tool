//! 對應 C# `UI/TestActionWindow.cs`：測試結果的小視窗。
//! fix＝開始修（可以順手填預估工時，也可以跳過；預設一起開始計時）；reopen＝復驗沒過、退回（說明會變成一則留言，輪數 +1）。
//! 從提示、通知、任務詳細都開這個，不用開網頁。

use std::rc::Rc;

use hb_core::{TestActionRequest, TimerEngine};
use slint::ComponentHandle;

use crate::dialog;
use crate::ui::TestActionWindow;
use crate::window_util::{self, Placement};

dialog::impl_dialog_ui!(TestActionWindow);

/// 做完了（成功）：給呼叫端更新畫面；參數是伺服器的說明（C# `event Action<string> Done`）。
pub type Done = Rc<dyn Fn(&str)>;

/// 開「開始修」（action = "fix"）或「退回測試結果」（其他）小視窗：固定在最上層、不佔工作列、置中。
/// 同一筆（action＋編號）已經開著就拉到前面（C# App.OpenTestAction 的 Tag）。
pub fn open(engine: &TimerEngine, request: &TestActionRequest, done: Option<Done>) {
    let tag = format!("{}:{}", request.action, request.key);
    if window_util::activate_tag(&tag) {
        return;
    }
    let Some(ui) = dialog::create("測試結果", TestActionWindow::new) else { return };
    let fix = request.action == "fix";
    let title = if fix { "開始修" } else { "退回測試結果" };
    ui.set_fix(fix);
    ui.set_heading(format!("{} {}", request.key, request.title).trim_end().into());
    let id = window_util::keep(&ui, Some(tag), ());
    let key = request.key.clone();

    // 開始修：預估工時可以不填；填了要是 0–1666 的小時數
    let start = {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let key = key.clone();
        let done = done.clone();
        move |hours: Option<f64>| {
            let Some(ui) = weak.upgrade() else { return };
            let start_timer = ui.get_start_timer();
            let (engine, key, done) = (engine.clone(), key.clone(), done.clone());
            dialog::run(&ui, id, title, async move {
                let message = engine.start_fix(&key, hours, start_timer).await.map_err(|e| e.message().to_owned())?;
                if let Some(done) = done {
                    done(&message);
                }
                Ok(())
            });
        }
    };
    {
        let weak = ui.as_weak();
        let start = start.clone();
        ui.on_start_fix(move |text| {
            let text = text.trim();
            let mut hours = None;
            if !text.is_empty() {
                match text.parse::<f64>() {
                    Ok(h) if (0.0..=1666.0).contains(&h) => hours = Some(h),
                    _ => {
                        if let Some(ui) = weak.upgrade() {
                            ui.set_error("預估工時請填小時數，例 2 或 0.5；不想填就留空".into());
                        }
                        return;
                    }
                }
            }
            start(hours);
        });
    }
    ui.on_skip_estimate(move || start(None));
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        ui.on_reject(move |comment| {
            let Some(ui) = weak.upgrade() else { return };
            let comment = comment.trim().to_owned();
            if comment.is_empty() {
                ui.set_error("寫一下哪裡還有問題，工程師才知道要修什麼".into());
                return;
            }
            let (engine, key, done) = (engine.clone(), key.clone(), done.clone());
            dialog::run(&ui, id, title, async move {
                let message = engine.retest(&key, false, Some(&comment)).await.map_err(|e| e.message().to_owned())?;
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
