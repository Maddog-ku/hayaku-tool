//! 對應 C# `UI/DeleteTaskWindow.cs`：刪除任務的確認小視窗。任務詳細、工作台清單／樹狀／看板與專案頁的右鍵、面板的右鍵都用這個。
//! 刪除成功時跳提示，並馬上同步一次（清單跟著更新）。

use std::cell::Cell;
use std::rc::Rc;

use hb_core::TimerEngine;
use slint::ComponentHandle;
use tokio::sync::oneshot;

use crate::dialog;
use crate::ui::DeleteTaskWindow;
use crate::window_util::{self, OnDrop, Placement};

dialog::impl_dialog_ui!(DeleteTaskWindow);

/// 第二段說明（children：子任務數；不知道時 None）。
pub fn detail_text(children: Option<i32>) -> String {
    let sub = match children {
        Some(n) if n > 0 => format!("連同 {n} 個子任務一起刪除。"),
        None => "有子任務的話會一起刪除。".to_owned(),
        _ => String::new(),
    };
    format!("{sub}可以從網頁的垃圾桶還原；如果你是執行者，會送出刪除申請給管理者審核。")
}

/// 開確認視窗；回傳的 future 在視窗關掉時完成，值是有沒有刪掉（C# `Deleted`）。
/// owner：置中在哪個視窗上（C# Owner）；None 時置中在螢幕。
pub fn open(
    engine: &TimerEngine,
    key: &str,
    title: &str,
    children: Option<i32>,
    owner: Option<&slint::Window>,
) -> impl std::future::Future<Output = bool> + 'static {
    let (tx, rx) = oneshot::channel::<bool>();
    let deleted = Rc::new(Cell::new(false));
    let fut = async move { rx.await.unwrap_or(false) };
    let Some(ui) = dialog::create("刪除任務", DeleteTaskWindow::new) else {
        let _ = tx.send(false);
        return fut;
    };
    ui.set_heading(format!("刪除 {key} {title}？").replace("  ", " ").into());
    ui.set_detail(detail_text(children).into());
    let flag = deleted.clone();
    let id = window_util::keep(
        &ui,
        None,
        OnDrop::new(move || {
            let _ = tx.send(flag.get());
        }),
    );
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let key = key.to_owned();
        ui.on_delete(move || {
            let Some(ui) = weak.upgrade() else { return };
            let (engine, key, deleted) = (engine.clone(), key.clone(), deleted.clone());
            dialog::run(&ui, id, "刪除任務", async move {
                let message = engine.api().delete_task(&key, None).await.map_err(|e| e.0)?;
                log::info!("刪除任務 {key}：{}", message.as_deref().unwrap_or("完成"));
                engine.show_notice(&message.unwrap_or_else(|| format!("已刪除 {key}（可以從網頁的垃圾桶還原）")));
                deleted.set(true);
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
    window_util::present(&ui, id, Placement::Owner(owner), true, (440.0, 200.0), crate::apply_theme);
    fut
}

/// 問過再刪；刪了就同步一次，回傳有沒有刪（C# `ConfirmAsync`）。
pub async fn confirm(
    engine: &TimerEngine,
    key: &str,
    title: &str,
    children: Option<i32>,
    owner: Option<&slint::Window>,
) -> bool {
    let wait = open(engine, key, title, children, owner);
    if !wait.await {
        return false;
    }
    engine.after_external_change().await;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 說明依子任務數() {
        assert_eq!(
            "連同 3 個子任務一起刪除。可以從網頁的垃圾桶還原；如果你是執行者，會送出刪除申請給管理者審核。",
            detail_text(Some(3))
        );
        assert!(detail_text(None).starts_with("有子任務的話會一起刪除。"));
        assert!(detail_text(Some(0)).starts_with("可以從網頁的垃圾桶還原"));
    }
}
