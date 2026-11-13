//! 對應 C# `UI/TaskPickerWindow.xaml(.cs)`：選擇任務（搜尋，或直接輸入任務編號）。
//! 多選時可以 Ctrl／Shift 點（C# ListBox SelectionMode.Extended）。
//!
//! 跟上 Hayaku 1.5.4「全站選任務的地方改成同一種樣子」（C# 版沒有）：
//! - 每列寫所屬專案和上層任務；搜尋也比對專案、上層。
//! - 沒打字時先列「最近填過」（今天計時過的，新的在前）、再列「指派給我的」（傳進來的清單）。
//! - 不能選的灰掉並寫原因（只用已有的資料：專案狀態清單快取裡是完成類的、專案樹快取裡是分類任務）。
//! - 這些資料從 [`hb_core::TimerEngine::current`] 拿（engine 已讀到的，不發請求），所以 [`pick`] 的參數不變、呼叫端不用改；
//!   拿不到 engine 時跟原本一樣只列清單（加上「指派給我的」標題）。

use std::cell::RefCell;
use std::rc::Rc;

use hb_api::{KeyFinder, TaskItem};
use hb_core::TimerEngine;
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

/// 選擇視窗裡的一個任務（Hayaku 1.5.4 起多了專案、上層、最近填過、不能選的原因）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PickerEntry {
    pub key: String,
    pub title: String,
    pub project: String,
    pub parent: Option<String>,
    pub status: String,
    /// 今天計時過：第幾新（0 最新）
    pub recent: Option<usize>,
    /// 不能選的原因
    pub disabled: Option<String>,
}

impl PickerEntry {
    fn plain(t: &TaskItem) -> Self {
        Self {
            key: t.key.clone(),
            title: t.title.clone(),
            project: t.project.clone(),
            parent: None,
            status: t.status.clone(),
            recent: None,
            disabled: None,
        }
    }

    /// 第二行：「專案 › 上層 · 狀態」。
    pub fn detail(&self) -> String {
        let mut s = self.project.clone();
        if let Some(p) = self.parent.as_deref().filter(|p| !p.is_empty()) {
            if !s.is_empty() {
                s += " › ";
            }
            s += p;
        }
        if !self.status.is_empty() {
            if !s.is_empty() {
                s += " · ";
            }
            s += &self.status;
        }
        s
    }

    fn matches(&self, q: &str) -> bool {
        [Some(&self.key), Some(&self.title), Some(&self.project), self.parent.as_ref(), Some(&self.status)]
            .into_iter()
            .flatten()
            .any(|s| s.to_lowercase().contains(q))
    }
}

/// 畫面上的一列：分組標題或第幾個任務。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Slot {
    Header(&'static str),
    Task(usize),
}

pub(crate) const RECENT_HEADER: &str = "最近填過";
pub(crate) const MINE_HEADER: &str = "指派給我的";

/// 排出畫面上的列：沒打字時「最近填過」（新的在前）＋「指派給我的」（其他的，照原本順序）；
/// 打了字就只列符合的（編號、名稱、專案、上層、狀態，不分大小寫），不分組。
pub(crate) fn layout(entries: &[PickerEntry], query: &str) -> Vec<Slot> {
    let q = query.trim().to_lowercase();
    if !q.is_empty() {
        return entries.iter().enumerate().filter(|(_, e)| e.matches(&q)).map(|(i, _)| Slot::Task(i)).collect();
    }
    let mut recent: Vec<(usize, usize)> =
        entries.iter().enumerate().filter_map(|(i, e)| e.recent.map(|r| (r, i))).collect();
    recent.sort();
    let mut slots = Vec::new();
    if !recent.is_empty() {
        slots.push(Slot::Header(RECENT_HEADER));
        slots.extend(recent.iter().map(|&(_, i)| Slot::Task(i)));
    }
    let others: Vec<Slot> =
        entries.iter().enumerate().filter(|(_, e)| e.recent.is_none()).map(|(i, _)| Slot::Task(i)).collect();
    if !others.is_empty() {
        slots.push(Slot::Header(MINE_HEADER));
        slots.extend(others);
    }
    slots
}

/// 用 engine 已有的資料補上專案名稱、上層、最近填過、不能選的原因（不發請求）。
fn enrich(tasks: &[TaskItem], engine: Option<&TimerEngine>) -> Vec<PickerEntry> {
    let Some(engine) = engine else { return tasks.iter().map(PickerEntry::plain).collect() };
    // 今天計時過的任務，新的在前
    let mut today: Vec<(Option<hb_api::DateTimeOffset>, String)> =
        engine.entries().iter().map(|e| (e.started_at, e.task.to_uppercase())).collect();
    today.sort_by_key(|t| std::cmp::Reverse(t.0));
    let mut recent_keys: Vec<String> = Vec::new();
    for (_, k) in today {
        if !recent_keys.contains(&k) {
            recent_keys.push(k);
        }
    }
    tasks
        .iter()
        .map(|t| {
            let project_key = KeyFinder::project_of(&t.key);
            let project = engine.project_of(&t.project).map(|p| p.name).unwrap_or_else(|| t.project.clone());
            let category_of_status = engine
                .try_get_cached_project(project_key)
                .and_then(|p| p.statuses.iter().find(|s| s.name == t.status).map(|s| s.category.clone()));
            let is_category = engine
                .cached_project_tree(project_key)
                .and_then(|(tree, _)| tree.iter().find(|x| x.key.eq_ignore_ascii_case(&t.key)).map(|x| x.is_category));
            let disabled = match (category_of_status.as_deref(), is_category) {
                (Some("DONE"), _) => Some("已完成，不能選".to_owned()),
                (Some("CANCELLED" | "CANCELED"), _) => Some("已取消，不能選".to_owned()),
                (_, Some(true)) => Some("分類任務只用來分組，不能計時".to_owned()),
                _ => None,
            };
            PickerEntry {
                key: t.key.clone(),
                title: t.title.clone(),
                project,
                parent: engine.parent_title_of(&t.key),
                status: t.status.clone(),
                recent: recent_keys.iter().position(|k| k.eq_ignore_ascii_case(&t.key)),
                disabled,
            }
        })
        .collect()
}

/// 列高（px）：分組標題、任務。
const HEADER_HEIGHT: f32 = 30.0;
const ROW_HEIGHT: f32 = 52.0;

struct State {
    entries: Vec<PickerEntry>,
    slots: Vec<Slot>,
    selected: Vec<bool>,
    anchor: Option<usize>,
    multi: bool,
}

impl State {
    /// 這一列能不能選（分組標題、不能選的任務不行）。
    fn selectable(&self, i: usize) -> bool {
        matches!(self.slots.get(i), Some(Slot::Task(t)) if self.entries.get(*t).is_some_and(|e| e.disabled.is_none()))
    }

    fn key_at(&self, i: usize) -> Option<&str> {
        match self.slots.get(i) {
            Some(Slot::Task(t)) => self.entries.get(*t).map(|e| e.key.as_str()),
            _ => None,
        }
    }

    /// 從 from 往上（step -1）或往下（1）找下一個能選的列。
    fn next_selectable(&self, from: i32, step: i32) -> Option<i32> {
        let n = self.slots.len() as i32;
        let mut i = from;
        loop {
            i += step;
            if i < 0 || i >= n {
                return None;
            }
            if self.selectable(i as usize) {
                return Some(i);
            }
        }
    }
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
    let entries = enrich(tasks, TimerEngine::current().as_ref());
    let slots = layout(&entries, "");
    let state = Rc::new(RefCell::new(State {
        selected: vec![false; slots.len()],
        slots,
        entries,
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
            let mut top = 0.0_f32;
            let rows: Vec<PickRow> = s
                .slots
                .iter()
                .zip(&s.selected)
                .map(|(slot, sel)| {
                    let row = match slot {
                        Slot::Header(text) => PickRow {
                            header: true,
                            title: (*text).into(),
                            top,
                            height: HEADER_HEIGHT,
                            ..Default::default()
                        },
                        Slot::Task(i) => {
                            let e = &s.entries[*i];
                            PickRow {
                                key: e.key.as_str().into(),
                                title: e.title.as_str().into(),
                                detail: e.detail().into(),
                                selected: *sel,
                                header: false,
                                disabled: e.disabled.is_some(),
                                reason: e.disabled.clone().unwrap_or_default().into(),
                                top,
                                height: ROW_HEIGHT,
                            }
                        }
                    };
                    top += row.height;
                    row
                })
                .collect();
            ui.set_list_height(top);
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
                s.slots = layout(&s.entries, &text);
                s.selected = vec![false; s.slots.len()];
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
                // 分組標題、不能選的點了沒反應
                if !s.selectable(i) {
                    return;
                }
                if s.multi
                    && shift
                    && let Some(a) = s.anchor
                {
                    let (lo, hi) = (a.min(i), a.max(i));
                    for k in 0..s.selected.len() {
                        let on = (lo..=hi).contains(&k) && s.selectable(k);
                        s.selected[k] = on;
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
            // 跳過分組標題、不能選的；到頭了就不動
            let Some(next) = state.borrow().next_selectable(ui.get_current(), if delta < 0 { -1 } else { 1 }) else {
                return;
            };
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
                (0..s.slots.len())
                    .filter(|&i| s.selected[i] && s.selectable(i))
                    .filter_map(|i| s.key_at(i).map(str::to_owned))
                    .collect()
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
    window_util::present(&ui, id, Placement::Screen, true, (520.0, 560.0), crate::apply_theme);
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

    fn entry(key: &str, parent: Option<&str>, recent: Option<usize>, disabled: Option<&str>) -> PickerEntry {
        PickerEntry {
            key: key.into(),
            title: format!("{key} 名稱"),
            project: "點數系統".into(),
            parent: parent.map(str::to_owned),
            status: "進行中".into(),
            recent,
            disabled: disabled.map(str::to_owned),
        }
    }

    #[test]
    fn 說明_專案上層與狀態() {
        assert_eq!("點數系統 › 第二階段 · 進行中", entry("F2P-1", Some("第二階段"), None, None).detail());
        assert_eq!("點數系統 · 進行中", entry("F2P-1", None, None, None).detail());
        let mut e = entry("F2P-1", None, None, None);
        e.project.clear();
        e.status.clear();
        assert_eq!("", e.detail());
    }

    #[test]
    fn 沒打字先列最近填過再列指派給我的() {
        let entries = vec![
            entry("F2P-1", None, None, None),
            entry("F2P-2", None, Some(1), None),
            entry("F2P-3", Some("第二階段"), Some(0), None),
            entry("F2P-4", None, None, Some("已完成，不能選")),
        ];
        assert_eq!(
            vec![
                Slot::Header(RECENT_HEADER),
                Slot::Task(2),
                Slot::Task(1),
                Slot::Header(MINE_HEADER),
                Slot::Task(0),
                Slot::Task(3)
            ],
            layout(&entries, "  ")
        );
        // 沒有最近填過：只有「指派給我的」
        let plain = vec![entry("F2P-1", None, None, None)];
        assert_eq!(vec![Slot::Header(MINE_HEADER), Slot::Task(0)], layout(&plain, ""));
        assert!(layout(&[], "").is_empty());
    }

    #[test]
    fn 打字只列符合的_也比對專案與上層() {
        let entries = vec![entry("F2P-1", None, Some(0), None), entry("F2P-2", Some("第二階段"), None, None)];
        assert_eq!(vec![Slot::Task(1)], layout(&entries, "第二"));
        assert_eq!(vec![Slot::Task(0), Slot::Task(1)], layout(&entries, "點數"));
        assert_eq!(vec![Slot::Task(1)], layout(&entries, "f2p-2"));
        assert!(layout(&entries, "沒有").is_empty());
    }

    #[test]
    fn 不能選的與分組標題跳過() {
        let entries = vec![
            entry("F2P-1", None, None, Some("已完成，不能選")),
            entry("F2P-2", None, None, None),
            entry("F2P-3", None, None, None),
        ];
        let slots = layout(&entries, "");
        let s = State { selected: vec![false; slots.len()], slots, entries, anchor: None, multi: false };
        assert!(!s.selectable(0)); // 分組標題
        assert!(!s.selectable(1));
        assert!(s.selectable(2));
        assert_eq!(Some("F2P-2"), s.key_at(2));
        assert_eq!(None, s.key_at(0));
        // 鍵盤：從沒選（-1）往下跳到第一個能選的；往上到頭不動
        assert_eq!(Some(2), s.next_selectable(-1, 1));
        assert_eq!(Some(3), s.next_selectable(2, 1));
        assert_eq!(None, s.next_selectable(3, 1));
        assert_eq!(None, s.next_selectable(2, -1));
    }
}
