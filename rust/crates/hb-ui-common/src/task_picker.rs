//! 對應 C# `UI/TaskPickerWindow.xaml(.cs)`：選擇任務（搜尋，或直接輸入任務編號）。
//! 多選時可以 Ctrl／Shift 點（C# ListBox SelectionMode.Extended）。

use std::cell::RefCell;
use std::rc::Rc;

use hb_api::{KeyFinder, TaskItem};
use slint::{ComponentHandle, ModelRc, VecModel};
use tokio::sync::oneshot;

use crate::dialog;
use crate::ui::{PickRow, TaskPickerWindow};
use crate::window_util::{self, OnDrop, Placement};

/// 清單的一項（C# `PickItem`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickItem {
    pub key: String,
    pub title: String,
    pub detail: String,
}

impl PickItem {
    pub fn from(t: &TaskItem) -> Self {
        Self { key: t.key.clone(), title: t.title.clone(), detail: format!("{} · {}", t.project, t.status) }
    }
}

/// 搜尋：編號、名稱、說明包含（不分大小寫）；空白時全部。
pub fn filter(all: &[PickItem], query: &str) -> Vec<PickItem> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return all.to_vec();
    }
    all.iter()
        .filter(|t| {
            t.key.to_lowercase().contains(&q)
                || t.title.to_lowercase().contains(&q)
                || t.detail.to_lowercase().contains(&q)
        })
        .cloned()
        .collect()
}

struct State {
    all: Vec<PickItem>,
    shown: Vec<PickItem>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    multi: bool,
}

/// 開選擇任務，回傳的 future 在視窗關掉時完成：None＝取消；Some(選的編號)（C# `SelectedKeys`，至少一個）。
pub fn pick(
    tasks: &[TaskItem],
    prompt: &str,
    multi_select: bool,
) -> impl std::future::Future<Output = Option<Vec<String>>> + 'static {
    let (tx, rx) = oneshot::channel::<Option<Vec<String>>>();
    let fut = async move { rx.await.unwrap_or(None) };
    let Some(ui) = dialog::create("選擇任務", TaskPickerWindow::new) else {
        let _ = tx.send(None);
        return fut;
    };
    ui.set_prompt(prompt.into());
    let all: Vec<PickItem> = tasks.iter().map(PickItem::from).collect();
    let state = Rc::new(RefCell::new(State {
        selected: vec![false; all.len()],
        shown: all.clone(),
        all,
        anchor: None,
        multi: multi_select,
    }));
    let result: Rc<RefCell<Option<Vec<String>>>> = Rc::default();
    let r = result.clone();
    let id = window_util::keep(
        &ui,
        None,
        OnDrop::new(move || {
            let _ = tx.send(r.borrow_mut().take());
        }),
    );

    let render = {
        let state = state.clone();
        move |ui: &TaskPickerWindow| {
            let s = state.borrow();
            let rows: Vec<PickRow> = s
                .shown
                .iter()
                .zip(&s.selected)
                .map(|(p, sel)| PickRow {
                    key: p.key.as_str().into(),
                    title: p.title.as_str().into(),
                    detail: p.detail.as_str().into(),
                    selected: *sel,
                })
                .collect();
            ui.set_rows(ModelRc::new(VecModel::from(rows)));
        }
    };
    render(&ui);
    {
        let (weak, state, render) = (ui.as_weak(), state.clone(), render.clone());
        ui.on_search_edited(move |text| {
            let Some(ui) = weak.upgrade() else { return };
            {
                let mut s = state.borrow_mut();
                s.shown = filter(&s.all, &text);
                s.selected = vec![false; s.shown.len()];
                s.anchor = None;
            }
            ui.set_current(-1);
            render(&ui);
        });
    }
    {
        let (weak, state, render) = (ui.as_weak(), state.clone(), render.clone());
        ui.on_row_clicked(move |index, ctrl, shift| {
            let Some(ui) = weak.upgrade() else { return };
            let i = index.max(0) as usize;
            {
                let mut s = state.borrow_mut();
                if i >= s.selected.len() {
                    return;
                }
                if s.multi
                    && shift
                    && let Some(a) = s.anchor
                {
                    let (lo, hi) = (a.min(i), a.max(i));
                    for (k, sel) in s.selected.iter_mut().enumerate() {
                        *sel = (lo..=hi).contains(&k);
                    }
                } else if s.multi && ctrl {
                    s.selected[i] = !s.selected[i];
                    s.anchor = Some(i);
                } else {
                    s.selected.iter_mut().for_each(|x| *x = false);
                    s.selected[i] = true;
                    s.anchor = Some(i);
                }
            }
            ui.set_current(index);
            render(&ui);
        });
    }
    {
        let (weak, state, render) = (ui.as_weak(), state.clone(), render.clone());
        ui.on_move_current(move |delta| {
            let Some(ui) = weak.upgrade() else { return };
            let n = state.borrow().shown.len() as i32;
            if n == 0 {
                return;
            }
            let next = (ui.get_current() + delta).clamp(0, n - 1);
            {
                let mut s = state.borrow_mut();
                s.selected.iter_mut().for_each(|x| *x = false);
                s.selected[next as usize] = true;
                s.anchor = Some(next as usize);
            }
            ui.set_current(next);
            render(&ui);
        });
    }
    {
        let (weak, state) = (ui.as_weak(), state.clone());
        ui.on_accept(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut keys: Vec<String> = {
                let s = state.borrow();
                s.shown.iter().zip(&s.selected).filter(|(_, sel)| **sel).map(|(p, _)| p.key.clone()).collect()
            };
            if keys.is_empty()
                && let Some(typed) = KeyFinder::parse(Some(&ui.get_search()))
            {
                keys.push(typed);
            }
            if keys.is_empty() {
                ui.invoke_focus_search();
                return;
            }
            *result.borrow_mut() = Some(keys);
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
    window_util::present(&ui, id, Placement::Screen, true, (480.0, 520.0), crate::apply_theme);
    ui.invoke_focus_search();
    fut
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 搜尋_編號名稱說明都比對() {
        let all = vec![
            PickItem { key: "F2P-97".into(), title: "報表頁面".into(), detail: "F2P · 處理中".into() },
            PickItem { key: "EG-7".into(), title: "登入".into(), detail: "EG · 待辦".into() },
        ];
        assert_eq!(2, filter(&all, " ").len());
        assert_eq!("F2P-97", filter(&all, "f2p")[0].key);
        assert_eq!("EG-7", filter(&all, "待辦")[0].key);
        assert!(filter(&all, "沒有").is_empty());
    }
}
