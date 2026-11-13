//! 對應 C# `UI/TaskMenu.cs`：工作台任務列（清單、樹狀、看板、專案頁）的右鍵選單。
//!
//! 做法：Slint 的 `ContextMenuArea`（`@hbc` 的 `TaskMenuArea`）。Windows 上 Slint 用 muda 顯示**原生** Win32 選單，
//! 外觀、鍵盤操作、跨出視窗邊界都跟系統選單一樣；選單內容綁在 `TaskMenuState` global 上，由這裡的 Rust 填。
//! 沒選 hb-win 自己做原生選單，是因為 TrackPopupMenu 會在 Slint 事件迴圈裡跑自己的模態迴圈，
//! 「改狀態」要等網路讀狀態清單時沒辦法在選單開著時更新；也沒選 PopupWindow，因為它畫在視窗裡、出不了視窗邊界。
//!
//! 原生選單打開後內容就固定了（C# 是展開「改狀態」子選單時才讀）：這裡在按右鍵時讀專案的狀態清單，
//! 有快取就馬上顯示；沒有就等讀完（最多 [`WAIT_FOR_STATUSES`]）再顯示，逾時先顯示「讀取中…」。
//!
//! 接線（呼叫端 crate 的 .slint 要 `export { TaskMenuState } from "@hbc/index.slint";`）：
//! ```ignore
//! hb_ui_common::wire_task_menu!(ui, TaskMenuState, engine, hb_ui_common::TaskMenuHooks { … });
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use hb_api::KeyFinder;
use hb_core::TimerEngine;

use crate::task_text::KeyCopy;

/// 沒有快取時，等狀態清單多久再顯示選單。
pub const WAIT_FOR_STATUSES: Duration = Duration::from_millis(600);

/// 選單項目的 id（`TaskMenuState.action(key, id)`）。
pub mod ids {
    pub const START: &str = "start";
    pub const PIN: &str = "pin";
    pub const OPEN: &str = "open";
    pub const COPY_KEY: &str = "copy-key";
    pub const COPY_LINK: &str = "copy-link";
    pub const WEB: &str = "web";
    pub const TEMPLATE: &str = "template";
    pub const DELETE: &str = "delete";
}

/// 呼叫端提供的動作（C# `Build(engine, key, open, delete, templateApplied)`）。
#[derive(Clone)]
pub struct TaskMenuHooks {
    /// 查看詳細。
    pub open: Rc<dyn Fn(&str)>,
    /// 刪除任務（呼叫端問過再刪、更新畫面；通常用 [`crate::delete_task::confirm`]）。
    pub delete: Rc<dyn Fn(&str)>,
    /// 套用子任務範本之後（呼叫端更新畫面）。
    pub template_applied: Option<Rc<dyn Fn()>>,
}

/// 按右鍵當下的選單內容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskMenuSnapshot {
    pub key: String,
    pub start_title: String,
    pub pin_title: String,
    pub pin_enabled: bool,
    pub status_title: String,
    /// 狀態子選單的項目（目前狀態前面加「● 」，其他加四個空白）；None＝還沒讀到。
    pub status_items: Option<Vec<String>>,
    /// 子選單只有一個停用項目時的文字（讀取中…／讀取失敗：…）。
    pub status_note: String,
}

/// 右鍵選單的邏輯；一個 `TaskMenuArea` 群組（一個 global）共用一份。
pub struct TaskMenu {
    engine: TimerEngine,
    hooks: TaskMenuHooks,
    /// 這次選單「改狀態」對應的狀態名稱（依子選單順序）。
    statuses: RefCell<Vec<String>>,
    /// 第幾次打開；讀完時用來確認還是同一次。
    serial: Cell<u64>,
}

impl TaskMenu {
    pub fn new(engine: TimerEngine, hooks: TaskMenuHooks) -> Self {
        Self { engine, hooks, statuses: RefCell::new(Vec::new()), serial: Cell::new(0) }
    }

    pub fn engine(&self) -> &TimerEngine {
        &self.engine
    }

    /// 按右鍵：算出選單內容；狀態清單有快取就一起給。
    pub fn snapshot(&self, key: &str) -> TaskMenuSnapshot {
        let engine = &self.engine;
        let running = engine.running_keys().contains(key);
        let pinned = engine.is_pinned(key);
        let current = engine.status_of(key);
        self.serial.set(self.serial.get() + 1);
        let cached = engine.try_get_cached_project(KeyFinder::project_of(key));
        let names: Option<Vec<String>> = cached.map(|p| p.statuses.iter().map(|s| s.name.clone()).collect());
        *self.statuses.borrow_mut() = names.clone().unwrap_or_default();
        TaskMenuSnapshot {
            key: key.to_owned(),
            start_title: if running { "❚❚  暫停計時" } else { "▶  開始計時" }.to_owned(),
            pin_title: if pinned { "📌  取消釘選" } else { "📌  釘到托盤" }.to_owned(),
            pin_enabled: pinned || engine.pins().len() < TimerEngine::MAX_PINS,
            status_title: match &current {
                None => "⇄  改狀態".to_owned(),
                Some(c) => format!("⇄  改狀態（{c}）"),
            },
            status_items: names.map(|n| status_items(&n, current.as_deref())),
            status_note: "讀取中…".to_owned(),
        }
    }

    /// 沒有快取時讀專案的狀態清單；`done` 收到（項目或錯誤說明）。讀完時已經又按了一次右鍵就不理。
    pub fn load_statuses(self: &Rc<Self>, key: &str, done: impl FnOnce(Result<Vec<String>, String>) + 'static) {
        let me = self.clone();
        let key = key.to_owned();
        let serial = self.serial.get();
        let task = async move {
            let result = me.engine.get_project(KeyFinder::project_of(&key)).await;
            if me.serial.get() != serial {
                return;
            }
            match result {
                Ok(project) => {
                    let names: Vec<String> = project.statuses.iter().map(|s| s.name.clone()).collect();
                    *me.statuses.borrow_mut() = names.clone();
                    let current = me.engine.status_of(&key);
                    done(Ok(status_items(&names, current.as_deref())));
                }
                Err(e) => {
                    log::error!("讀取狀態清單失敗: {}", e.message());
                    done(Err(format!("讀取失敗：{}", e.message())));
                }
            }
        };
        if let Err(e) = slint::spawn_local(task) {
            log::error!("讀取狀態清單失敗: {e}");
        }
    }

    /// 選了一個項目。
    pub fn invoke(&self, key: &str, id: &str) {
        let engine = self.engine.clone();
        let key_owned = key.to_owned();
        let spawn = |fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()>>>| {
            if let Err(e) = slint::spawn_local(fut) {
                log::error!("右鍵選單動作失敗: {e}");
            }
        };
        match id {
            ids::START => {
                let running = engine.running_keys().contains(key);
                spawn(Box::pin(async move {
                    if running { engine.pause_task(&key_owned).await } else { engine.start_task(&key_owned).await }
                }));
            }
            ids::PIN => {
                let pinned = engine.is_pinned(key);
                spawn(Box::pin(async move {
                    if pinned { engine.unpin_task(&key_owned).await } else { engine.pin_task(&key_owned).await }
                }));
            }
            ids::OPEN => (self.hooks.open)(key),
            ids::COPY_KEY => KeyCopy::copy(&engine, key),
            ids::COPY_LINK => KeyCopy::copy_link(&engine, key),
            ids::WEB => hb_win::open(&engine.task_url(key)),
            ids::TEMPLATE => {
                let applied = self.hooks.template_applied.clone();
                crate::apply_template::open(
                    &engine,
                    KeyFinder::project_of(key),
                    vec![key_owned],
                    None,
                    Box::new(move |ok| {
                        if ok && let Some(f) = &applied {
                            f();
                        }
                    }),
                );
            }
            ids::DELETE => (self.hooks.delete)(key),
            other => log::error!("右鍵選單沒有這個項目：{other}"),
        }
    }

    /// 「改狀態」選了第 index 個。
    pub fn pick_status(&self, key: &str, index: i32) {
        let Some(name) = usize::try_from(index).ok().and_then(|i| self.statuses.borrow().get(i).cloned()) else {
            return;
        };
        let engine = self.engine.clone();
        let key = key.to_owned();
        let task = async move {
            engine.set_task_status(&key, &name).await;
            engine.after_external_change().await; // 清單上的狀態跟著更新
        };
        if let Err(e) = slint::spawn_local(task) {
            log::error!("改狀態失敗: {e}");
        }
    }
}

/// 子選單項目：目前狀態前面加「● 」，其他加四個空白（C# 一樣）。
pub fn status_items(names: &[String], current: Option<&str>) -> Vec<String> {
    names.iter().map(|n| format!("{}{n}", if Some(n.as_str()) == current { "● " } else { "    " })).collect()
}

/// 把 [`TaskMenu`] 接到呼叫端 crate 自己編譯出的 `TaskMenuState` global（欄位、回呼名稱一致）。
///
/// `$ui`：擁有這個 global 的視窗元件（`ComponentHandle`）；`$global`：呼叫端的 `TaskMenuState` 型別；
/// `$engine`：`&TimerEngine`；`$hooks`：[`TaskMenuHooks`]。回傳 `Rc<TaskMenu>`。
#[macro_export]
macro_rules! wire_task_menu {
    ($ui:expr, $global:ty, $engine:expr, $hooks:expr) => {{
        use ::slint::ComponentHandle as _;
        let menu = ::std::rc::Rc::new($crate::task_menu::TaskMenu::new(($engine).clone(), $hooks));
        let weak = ($ui).as_weak();
        let g = ($ui).global::<$global>();
        {
            let menu = menu.clone();
            let weak = weak.clone();
            g.on_opening(move |key| {
                let Some(ui) = weak.upgrade() else { return };
                let snap = menu.snapshot(&key);
                let g = ui.global::<$global>();
                g.set_key(snap.key.as_str().into());
                g.set_start_title(snap.start_title.as_str().into());
                g.set_pin_title(snap.pin_title.as_str().into());
                g.set_pin_enabled(snap.pin_enabled);
                g.set_status_title(snap.status_title.as_str().into());
                g.set_status_note(snap.status_note.as_str().into());
                let set_items = |g: &$global, items: &[String]| {
                    let v: Vec<::slint::SharedString> = items.iter().map(|s| s.as_str().into()).collect();
                    g.set_status_items(::slint::ModelRc::new(::slint::VecModel::from(v)));
                };
                match &snap.status_items {
                    Some(items) => {
                        set_items(&g, items);
                        g.set_status_ready(true);
                        g.set_show_serial(g.get_show_serial() + 1);
                    }
                    None => {
                        set_items(&g, &[]);
                        g.set_status_ready(false);
                        let shown = ::std::rc::Rc::new(::std::cell::Cell::new(false));
                        let show = {
                            let weak = weak.clone();
                            let shown = shown.clone();
                            move || {
                                if shown.replace(true) {
                                    return;
                                }
                                if let Some(ui) = weak.upgrade() {
                                    let g = ui.global::<$global>();
                                    g.set_show_serial(g.get_show_serial() + 1);
                                }
                            }
                        };
                        let show2 = show.clone();
                        ::slint::Timer::single_shot($crate::task_menu::WAIT_FOR_STATUSES, show2);
                        let weak = weak.clone();
                        menu.load_statuses(&key, move |result| {
                            if let Some(ui) = weak.upgrade() {
                                let g = ui.global::<$global>();
                                match result {
                                    Ok(items) => {
                                        let v: Vec<::slint::SharedString> =
                                            items.iter().map(|s| s.as_str().into()).collect();
                                        g.set_status_items(::slint::ModelRc::new(::slint::VecModel::from(v)));
                                        g.set_status_ready(true);
                                    }
                                    Err(note) => g.set_status_note(note.as_str().into()),
                                }
                            }
                            show();
                        });
                    }
                }
            });
        }
        {
            let menu = menu.clone();
            g.on_action(move |key, id| menu.invoke(&key, &id));
        }
        {
            let menu = menu.clone();
            g.on_pick_status(move |key, index| menu.pick_status(&key, index));
        }
        menu
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 狀態子選單_目前狀態加圓點() {
        let names = vec!["待辦".to_owned(), "處理中".to_owned()];
        assert_eq!(vec!["    待辦".to_owned(), "● 處理中".to_owned()], status_items(&names, Some("處理中")));
        assert_eq!(vec!["    待辦".to_owned(), "    處理中".to_owned()], status_items(&names, None));
    }
}
