//! 對應 C# `UI/MoveTaskWindow.cs`：移動任務。在同一個專案的任務樹裡挑新的上層，或移到最上層。
//! 自己與自己底下的任務不能當上層；MCP 沒有跨專案移動，要移到別的專案請到網頁。

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;

use hb_api::{KeyFinder, ProjectTask};
use hb_core::{TaskTree, TaskTreeNode, TimerEngine};
use slint::{ComponentHandle, ModelRc, VecModel};
use tokio::sync::oneshot;

use crate::dialog;
use crate::ui::{MoveRow, MoveTaskWindow};
use crate::window_util::{self, OnDrop, Placement};

/// 清單的一列：key 為 None 是「最上層」。
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub key: Option<String>,
    pub text: String,
    pub depth: i32,
    /// 完成、不處理的淡一點（C# Opacity 0.55）
    pub dim: bool,
}

/// 自己和自己底下的任務（不能當上層），比對不分大小寫（存大寫）。
pub fn self_and_descendants(tasks: &[ProjectTask], task: &str) -> HashSet<String> {
    let mut set: HashSet<String> = HashSet::from([task.to_uppercase()]);
    loop {
        let mut added = false;
        for t in tasks {
            if let Some(p) = &t.parent
                && set.contains(&p.to_uppercase())
                && set.insert(t.key.to_uppercase())
            {
                added = true;
            }
        }
        if !added {
            return set;
        }
    }
}

/// 畫出清單（C# `Render`）。
pub fn rows(
    tasks: &[ProjectTask],
    task: &str,
    current_parent: Option<&str>,
    hide_done: bool,
    filter: &str,
) -> Vec<Row> {
    let excluded = self_and_descendants(tasks, task);
    let roots = TaskTree::build(
        tasks.iter().filter(|t| !excluded.contains(&t.key.to_uppercase())).cloned(),
        !hide_done,
        Some(filter),
    );
    let mut rows = vec![Row {
        key: None,
        text: format!("⤒ 最上層（不屬於任何任務）{}", if current_parent.is_none() { "　· 目前" } else { "" }),
        depth: 0,
        dim: false,
    }];
    fn add(rows: &mut Vec<Row>, nodes: &[TaskTreeNode], depth: i32, current_parent: Option<&str>) {
        for n in nodes {
            let t = &n.task;
            let current =
                if current_parent.is_some_and(|p| p.eq_ignore_ascii_case(&t.key)) { "　· 目前" } else { "" };
            let category = if t.is_category { "　· 分類" } else { "" };
            rows.push(Row {
                key: Some(t.key.clone()),
                text: format!("{} {}{category}{current}", t.key, t.title),
                depth,
                dim: matches!(t.status_category.as_str(), "DONE" | "CANCELLED"),
            });
            add(rows, &n.children, depth + 1, current_parent);
        }
    }
    add(&mut rows, &roots, 0, current_parent);
    rows
}

/// 開「移動 {task}」，回傳的 future 在視窗關掉時完成：None＝取消；Some(新上層)，新上層 None＝最上層（C# `Parent`）。
/// 移動本身由呼叫端做（`engine.move_task`）。owner：置中在哪個視窗上。
pub fn pick_parent(
    engine: &TimerEngine,
    task: &str,
    current_parent: Option<&str>,
    owner: Option<&slint::Window>,
) -> impl std::future::Future<Output = Option<Option<String>>> + 'static {
    let (tx, rx) = oneshot::channel::<Option<Option<String>>>();
    let fut = async move { rx.await.unwrap_or(None) };
    let Some(ui) = dialog::create("移動任務", MoveTaskWindow::new) else {
        let _ = tx.send(None);
        return fut;
    };
    ui.set_window_title(format!("移動 {task}").into());
    ui.set_intro(format!("選 {task} 的新上層（同一個專案）。要移到別的專案請在網頁上操作。").into());
    ui.set_hide_done(engine.config().tree_hide_done);
    ui.set_message("讀取中…".into());
    ui.set_message_tone(3);

    let result: Rc<RefCell<Option<Option<String>>>> = Rc::default();
    let shown: Rc<RefCell<Vec<Row>>> = Rc::default();
    let tasks: Rc<RefCell<Vec<ProjectTask>>> = Rc::default();
    let loaded = Rc::new(Cell::new(false));
    let r = result.clone();
    let id = window_util::keep(
        &ui,
        None,
        OnDrop::new(move || {
            let _ = tx.send(r.borrow_mut().take());
        }),
    );

    let task = task.to_owned();
    let current_parent = current_parent.map(str::to_owned);
    let render = {
        let (shown, tasks, task, current_parent) = (shown.clone(), tasks.clone(), task.clone(), current_parent.clone());
        move |ui: &MoveTaskWindow| {
            let list = rows(&tasks.borrow(), &task, current_parent.as_deref(), ui.get_hide_done(), &ui.get_filter());
            let model: Vec<MoveRow> =
                list.iter().map(|r| MoveRow { text: r.text.as_str().into(), depth: r.depth, dim: r.dim }).collect();
            ui.set_rows(ModelRc::new(VecModel::from(model)));
            ui.set_current(0);
            *shown.borrow_mut() = list;
        }
    };
    {
        let weak = ui.as_weak();
        let render = render.clone();
        let loaded = loaded.clone();
        ui.on_filter_edited(move |_| {
            if let Some(ui) = weak.upgrade()
                && loaded.get()
            {
                render(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let render = render.clone();
        let engine = engine.clone();
        let loaded = loaded.clone();
        ui.on_hide_done_toggled(move |on| {
            {
                let mut config = engine.config_mut();
                config.tree_hide_done = on;
                config.save();
            }
            if let Some(ui) = weak.upgrade()
                && loaded.get()
            {
                render(&ui);
            }
        });
    }
    {
        let weak = ui.as_weak();
        let current_parent = current_parent.clone();
        ui.on_accept(move || {
            let Some(ui) = weak.upgrade() else { return };
            let Some(row) = usize::try_from(ui.get_current()).ok().and_then(|i| shown.borrow().get(i).cloned()) else {
                return;
            };
            let same = match (&row.key, &current_parent) {
                (None, None) => true,
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                _ => false,
            };
            if same {
                ui.set_message("已經在這裡了，選別的位置".into());
                ui.set_message_tone(2);
                return;
            }
            *result.borrow_mut() = Some(row.key);
            window_util::close(&ui, id);
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
    window_util::present(&ui, id, Placement::Owner(owner), true, (520.0, 560.0), crate::apply_theme);

    let weak = ui.as_weak();
    let engine = engine.clone();
    dialog::spawn(async move {
        let result = engine.api().list_project_tasks(KeyFinder::project_of(&task), None).await;
        let Some(ui) = weak.upgrade() else { return };
        match result {
            Ok(list) => {
                *tasks.borrow_mut() = list;
                loaded.set(true);
                ui.set_message("".into());
                render(&ui);
                ui.invoke_focus_filter();
            }
            Err(e) => {
                log::error!("讀取 {task} 的專案任務失敗: {}", e.0);
                ui.set_message(format!("讀不到專案的任務：{}", e.0).into());
                ui.set_message_tone(0);
            }
        }
    });
    fut
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::TestInfo;

    fn t(key: &str, parent: Option<&str>, category: &str) -> ProjectTask {
        ProjectTask {
            key: key.into(),
            title: format!("{key} 名稱"),
            status: "處理中".into(),
            status_category: category.into(),
            parent: parent.map(str::to_owned),
            planned_start: None,
            planned_end: None,
            is_category: false,
            assignees: None,
            priority: None,
            test: TestInfo::default(),
        }
    }

    #[test]
    fn 自己與子孫不能當上層() {
        let tasks = vec![
            t("A-1", None, "TODO"),
            t("A-2", Some("A-1"), "TODO"),
            t("A-3", Some("a-2"), "TODO"),
            t("A-4", None, "TODO"),
        ];
        let set = self_and_descendants(&tasks, "A-1");
        assert_eq!(3, set.len());
        let list = rows(&tasks, "A-1", None, false, "");
        assert_eq!(vec![None, Some("A-4".to_owned())], list.iter().map(|r| r.key.clone()).collect::<Vec<_>>());
        assert_eq!("⤒ 最上層（不屬於任何任務）　· 目前", list[0].text);
    }

    #[test]
    fn 目前位置與完成的淡色() {
        let tasks = vec![t("A-1", None, "TODO"), t("A-2", Some("A-1"), "DONE"), t("A-5", None, "TODO")];
        let list = rows(&tasks, "A-5", Some("a-1"), false, "");
        assert_eq!("A-1 A-1 名稱　· 目前", list[1].text);
        assert_eq!((1, true), (list[2].depth, list[2].dim));
    }
}
