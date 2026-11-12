//! 對應 C# `UI/BatchCreateWindow.cs`：批次新增任務。貼上清單（一行一個，用縮排表示子任務），
//! 一次建立整棵樹（create_tasks，全部成功或全部不建）。專案頁的「＋ 批次新增」開這個。

use std::rc::Rc;

use hb_api::{KeyFinder, OutlineNode};
use hb_core::{TaskOutline, TimerEngine};
use slint::ComponentHandle;

use crate::dialog;
use crate::ui::BatchCreateWindow;
use crate::window_util::{self, Placement};

dialog::impl_dialog_ui!(BatchCreateWindow);

const MAX: usize = 100;

/// 清單下面的預覽文字與是不是超過上限（紅字）。
pub fn preview(text: &str) -> (String, bool) {
    let nodes = TaskOutline::parse(Some(text));
    let count = TaskOutline::count(&nodes);
    fn depth(n: &OutlineNode) -> usize {
        1 + n.children.iter().map(depth).max().unwrap_or(0)
    }
    if count == 0 {
        return ("還沒有任務".to_owned(), false);
    }
    let deepest = nodes.iter().map(depth).max().unwrap_or(0);
    let mut s = format!("會建立 {count} 個任務（最上層 {} 個、最深 {deepest} 層）", nodes.len());
    if count > MAX {
        s += &format!("，一次最多 {MAX} 個");
    }
    (s, count > MAX)
}

/// 在 byte 位移的選取範圍插入 Tab，回傳新文字與游標位置（C# AcceptsTab）。
pub fn insert_tab(text: &str, anchor: usize, cursor: usize) -> (String, usize) {
    let (a, b) = (anchor.min(cursor).min(text.len()), anchor.max(cursor).min(text.len()));
    if !text.is_char_boundary(a) || !text.is_char_boundary(b) {
        return (text.to_owned(), cursor.min(text.len()));
    }
    (format!("{}\t{}", &text[..a], &text[b..]), a + 1)
}

/// 開批次新增；建好後同步一次再呼叫 created（呼叫端據此重新整理，C# `Created`）。
pub fn open(engine: &TimerEngine, project: &str, parent: Option<&str>, created: Option<Rc<dyn Fn()>>) {
    let Some(ui) = dialog::create("批次新增任務", BatchCreateWindow::new) else { return };
    let title = format!("批次新增任務 · {project}");
    ui.set_window_title(title.as_str().into());
    ui.set_parent_key(parent.unwrap_or("").into());
    let refresh = |ui: &BatchCreateWindow| {
        let (text, error) = preview(&ui.get_list_text());
        ui.set_preview(text.into());
        ui.set_preview_error(error);
    };
    refresh(&ui);
    let id = window_util::keep(&ui, None, ());
    {
        let weak = ui.as_weak();
        ui.on_list_edited(move |_| {
            if let Some(ui) = weak.upgrade() {
                refresh(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_insert_tab(move |text, anchor, cursor| {
            let Some(ui) = weak.upgrade() else { return };
            let (new_text, at) = insert_tab(&text, anchor.max(0) as usize, cursor.max(0) as usize);
            ui.set_list_text(new_text.into());
            ui.invoke_place_cursor(at as i32);
            refresh(&ui);
        });
    }
    {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let project = project.to_owned();
        ui.on_create(move || {
            let Some(ui) = weak.upgrade() else { return };
            let text = ui.get_list_text().to_string();
            let nodes = TaskOutline::parse(Some(&text));
            let count = TaskOutline::count(&nodes);
            if count == 0 {
                ui.set_error("請先貼上任務清單".into());
                return;
            }
            if count > MAX {
                ui.set_error(format!("一次最多 {MAX} 個任務，請分批建立").into());
                return;
            }
            let parent_text = ui.get_parent_key().to_string();
            let mut parent = None;
            if !parent_text.trim().is_empty() {
                parent = KeyFinder::parse(Some(&parent_text));
                if parent.is_none() {
                    ui.set_error("上層請填任務編號，例 F2P-5".into());
                    return;
                }
            }
            let assign_me = ui.get_assign_me();
            let (engine, project, created) = (engine.clone(), project.clone(), created.clone());
            dialog::run(&ui, id, &format!("批次新增任務 · {project}"), async move {
                let message =
                    engine.api().create_tasks(&project, parent.as_deref(), &nodes, assign_me).await.map_err(|e| e.0)?;
                let under = parent.as_ref().map(|p| format!("（在 {p} 底下）")).unwrap_or_default();
                log::info!("批次新增 {project} {count} 個任務{under}：{}", message.as_deref().unwrap_or("完成"));
                engine.show_notice(&message.unwrap_or_else(|| format!("已建立 {count} 個任務")));
                // 視窗關掉後同步一次，再讓呼叫端重新整理
                dialog::spawn(async move {
                    engine.after_external_change().await;
                    if let Some(f) = created {
                        f();
                    }
                });
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
    window_util::present(&ui, id, Placement::Screen, true, (520.0, 520.0), crate::apply_theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 預覽_數量與層數() {
        assert_eq!(("還沒有任務".to_owned(), false), preview(""));
        let (text, error) = preview("A\n\tA1\n\t\tA11\nB");
        assert_eq!("會建立 4 個任務（最上層 2 個、最深 3 層）", text);
        assert!(!error);
    }

    #[test]
    fn 插入_tab() {
        assert_eq!(("a\tb".to_owned(), 2), insert_tab("ab", 1, 1));
        assert_eq!(("\tc".to_owned(), 1), insert_tab("abc", 0, 2));
    }
}
