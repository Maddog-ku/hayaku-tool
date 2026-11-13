//! 工作台（對應 C# `UI/Work/WorkspaceWindow.xaml(.cs)`）：把網頁上常用的操作搬到桌面。
//! 左側導覽、右側一頁一個元件，第一次切到才建立；每頁的程式在 `pages/<頁>.rs`。

slint::include_modules!();

pub mod pages;
pub mod shared;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use chrono::NaiveDate;
use hb_core::{TimerEngine, WorkspaceArg};
use hb_ui_common::TaskMenuHooks;
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

/// 切頁時帶給那一頁的參數（對應 C# `ShowPage(page, object? arg)` 的 arg）。
#[derive(Debug, Clone, PartialEq)]
pub enum PageArg {
    Text(String),
    Number(i32),
    Date(NaiveDate),
}

impl From<WorkspaceArg> for PageArg {
    fn from(value: WorkspaceArg) -> Self {
        match value {
            WorkspaceArg::Text(s) => PageArg::Text(s),
            WorkspaceArg::Number(n) => PageArg::Number(n),
        }
    }
}

impl PageArg {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            PageArg::Text(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_date(&self) -> Option<NaiveDate> {
        match self {
            PageArg::Date(d) => Some(*d),
            PageArg::Text(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").ok(),
            _ => None,
        }
    }
}

/// 各頁共用的環境：engine、視窗、切頁。clone 很便宜。
#[derive(Clone)]
pub struct WorkCtx {
    pub engine: TimerEngine,
    window: slint::Weak<WorkspaceWindow>,
    shell: std::rc::Weak<Shell>,
}

impl WorkCtx {
    /// 工作台視窗（關閉後拿不到）。
    pub fn window(&self) -> Option<WorkspaceWindow> {
        self.window.upgrade()
    }

    /// 切到別頁（對應 C# 各頁呼叫 `_workspace.ShowPage(...)`）。
    pub fn show_page(&self, page: &str, arg: Option<PageArg>) {
        if let Some(shell) = self.shell.upgrade() {
            shell.show_page(page, arg);
        }
    }

    /// 切到「工時紀錄」看某一天（C# `OpenDay`）。
    pub fn open_day(&self, day: NaiveDate) {
        self.show_page("entries", Some(PageArg::Date(day)));
    }

    /// 目前顯示的頁面代號（tasks、projects…）。
    pub fn current_page(&self) -> Option<&'static str> {
        let shell = self.shell.upgrade()?;
        let index = shell.window.get_page_index();
        pages::PAGES.get(usize::try_from(index).ok()?).map(|p| p.0)
    }

    /// 這一頁正在顯示（工作台開著、目前是這一頁）。對應 C# 頁面的 `IsVisible`。
    pub fn is_page_visible(&self, page: &str) -> bool {
        self.current_page().is_some_and(|p| p.eq_ignore_ascii_case(page))
            && self.shell.upgrade().is_some_and(|s| s.window.window().is_visible())
    }

    /// 登記某一頁的任務右鍵選單動作（C# `TaskMenu.Build(engine, key, open, delete, templateApplied)`）。
    ///
    /// `TaskMenuState` 是整個視窗共用的一份 global（`TaskMenuArea` 都綁它），外框接好 `wire_task_menu!`，
    /// 按右鍵時交給**目前的頁面**登記的動作；切到有登記的頁面時外框會重新接上分派。
    /// 建議各頁用這個登記，不要自己再 `wire_task_menu!`（同一個 global，後接的會蓋掉先接的）。
    /// page：頁面代號（`pages::PAGES` 的 key），在頁面的 `init` 裡呼叫。
    pub fn set_task_menu_hooks(&self, page: &str, hooks: TaskMenuHooks) {
        if let Some(shell) = self.shell.upgrade() {
            shell.menu_hooks.borrow_mut().insert(page.to_ascii_lowercase(), hooks);
        }
    }
}

struct Shell {
    window: WorkspaceWindow,
    engine: TimerEngine,
    created: RefCell<Vec<bool>>,
    pending_arg: RefCell<Option<PageArg>>,
    changed_handler: Cell<Option<hb_core::HandlerId>>,
    menu_hooks: RefCell<HashMap<String, TaskMenuHooks>>,
    me: RefCell<std::rc::Weak<Shell>>,
}

impl Shell {
    fn ctx(&self) -> WorkCtx {
        WorkCtx { engine: self.engine.clone(), window: self.window.as_weak(), shell: self.me.borrow().clone() }
    }

    /// 頁面：tasks、projects、tests、reviews、hours、entries、worklog、reminders、meetings、chat、notes、spaces、forum、
    /// team、activity、workload、leaves、stats。eipleaves 併進請假頁，打開請假頁的「EIP 假單」分頁。
    fn show_page(&self, page: &str, arg: Option<PageArg>) {
        let (page, arg) = if page.eq_ignore_ascii_case("eipleaves") {
            ("leaves", arg.or_else(|| Some(PageArg::Text("eip".into()))))
        } else {
            (page, arg)
        };
        let index = pages::PAGES.iter().position(|(k, _)| k.eq_ignore_ascii_case(page)).unwrap_or(0);
        *self.pending_arg.borrow_mut() = arg;
        self.activate(index);
    }

    fn activate(&self, index: usize) {
        let first = {
            let mut created = self.created.borrow_mut();
            let first = !created[index];
            created[index] = true;
            first
        };
        if first {
            let flags: Vec<bool> = self.created.borrow().clone();
            self.window.set_created(ModelRc::new(VecModel::from(flags)));
            pages::init(index, &self.ctx());
        }
        self.window.set_page_index(index as i32);
        // 有登記右鍵選單動作的頁面：重新接上外框的分派（別的頁自己接過 wire_task_menu! 也不會蓋掉這一頁的）
        if self.menu_hooks.borrow().contains_key(pages::PAGES[index].0) {
            self.wire_task_menu();
        }
        let arg = self.pending_arg.borrow_mut().take();
        let ctx = self.ctx();
        let key = pages::PAGES[index].0;
        // 載入中新建立的清單依序浮現（ListReveal.busy > 0 時外框不關開關；載入完 450ms 後才關，之後的刷新不播）
        reveal_busy(&self.window, 1);
        let weak = self.window.as_weak();
        let task = async move {
            pages::activate(index, ctx, arg).await;
            if let Some(w) = weak.upgrade() {
                reveal_busy(&w, -1);
            }
            log::debug!("工作台 {key} 載入完成");
        };
        if let Err(e) = slint::spawn_local(task) {
            reveal_busy(&self.window, -1);
            log::error!("工作台 {key} 載入失敗：{e}");
        }
    }

    fn refresh_chrome(&self) {
        let engine = &self.engine;
        self.window
            .set_user_name(engine.user().map(|u| SharedString::from(u.display_name.as_str())).unwrap_or_default());
        let overdue = engine.overdue_count();
        let reviews = engine.pending_reviews().len();
        let fixes = engine.my_fixes().len();
        let items = self.window.get_nav_items();
        for i in 0..items.row_count() {
            let Some(mut item) = items.row_data(i) else { continue };
            let badge = chrome_badge(item.key.as_str(), overdue, reviews, fixes);
            if item.badge != badge.as_str() {
                item.badge = badge.into();
                items.set_row_data(i, item);
            }
        }
    }

    /// Ctrl+1～9、0 切換頁面。
    fn on_key(&self, event: &slint::private_unstable_api::re_exports::KeyEvent) -> bool {
        if !event.modifiers.control || event.modifiers.alt || event.modifiers.shift {
            return false;
        }
        let Some(c) = event.text.chars().next() else { return false };
        let Some(d) = c.to_digit(10) else { return false };
        let index = if d == 0 { 9 } else { d as usize - 1 };
        if index < pages::PAGES.len() {
            self.show_page(pages::PAGES[index].0, None);
            return true;
        }
        false
    }

    /// 右鍵選單（TaskMenuState 整個視窗一份）接到外框的分派：按右鍵時交給目前頁面用 set_task_menu_hooks 登記的動作。
    fn wire_task_menu(&self) {
        let me = self.me.borrow().clone();
        let open = {
            let weak = me.clone();
            Rc::new(move |key: &str| {
                if let Some(h) = weak.upgrade().and_then(|s| s.menu_hooks()) {
                    (h.open)(key);
                }
            })
        };
        let delete = {
            let weak = me.clone();
            Rc::new(move |key: &str| {
                if let Some(h) = weak.upgrade().and_then(|s| s.menu_hooks()) {
                    (h.delete)(key);
                }
            })
        };
        let template_applied = {
            let weak = me;
            Rc::new(move || {
                if let Some(f) = weak.upgrade().and_then(|s| s.menu_hooks()).and_then(|h| h.template_applied) {
                    f();
                }
            })
        };
        let _menu = hb_ui_common::wire_task_menu!(
            self.window,
            TaskMenuState,
            &self.engine,
            TaskMenuHooks { open, delete, template_applied: Some(template_applied) }
        );
    }

    /// 右鍵選單：交給目前頁面登記的動作。
    fn menu_hooks(&self) -> Option<TaskMenuHooks> {
        let index = usize::try_from(self.window.get_page_index()).ok()?;
        let page = pages::PAGES.get(index)?.0;
        self.menu_hooks.borrow().get(page).cloned()
    }
}

/// 頁面載入中計數（`ListReveal.busy`）加減；加的時候順便打開依序浮現的開關。
fn reveal_busy(window: &WorkspaceWindow, delta: i32) {
    let reveal = window.global::<ListReveal>();
    if delta > 0 {
        reveal.set_on(true);
    }
    reveal.set_busy((reveal.get_busy() + delta).max(0));
}

/// 導覽徽章的文字（C# RefreshChrome）：我的任務「逾期 N」、待我審核與測試是數字。
fn chrome_badge(key: &str, overdue: usize, reviews: usize, fixes: usize) -> String {
    match key {
        "tasks" if overdue > 0 => format!("逾期 {overdue}"),
        "reviews" if reviews > 0 => reviews.to_string(),
        "tests" if fixes > 0 => fixes.to_string(),
        _ => String::new(),
    }
}

/// 導覽的分組標題（C# 的「工作」「知識」「團隊」）：放在那一組第一項的上面。
fn nav_group(key: &str) -> &'static str {
    match key {
        "tasks" => "工作",
        "notes" => "知識",
        "team" => "團隊",
        _ => "",
    }
}

/// 導覽每一列的 y（跟 workspace.slint 的高度一致：第一個分組標題 22px、其他 34px、列 36px、間距 2px）。
fn nav_tops(groups: &[bool]) -> Vec<f32> {
    let mut y = 0.0;
    let mut tops = Vec::with_capacity(groups.len());
    for (i, has_group) in groups.iter().enumerate() {
        if *has_group {
            y += if i == 0 { 22.0 } else { 34.0 };
        }
        tops.push(y);
        y += 36.0 + 2.0;
    }
    tops
}

fn nav_items() -> Vec<NavItem> {
    let groups: Vec<bool> = pages::PAGES.iter().map(|(key, _)| !nav_group(key).is_empty()).collect();
    let tops = nav_tops(&groups);
    pages::PAGES
        .iter()
        .zip(tops)
        .map(|((key, label), top)| NavItem {
            key: (*key).into(),
            label: (*label).into(),
            badge: "".into(),
            group: nav_group(key).into(),
            badge_tone: if *key == "tasks" { 1 } else { 0 },
            badge_tip: if *key == "tests" { "要修的測試結果".into() } else { "".into() },
            top,
        })
        .collect()
}

/// 工作台視窗。App 持有一個；關掉後再開會重建（C# `_workspace = null`）。
#[derive(Clone)]
pub struct Workspace(Rc<Shell>);

impl Workspace {
    pub fn new(engine: TimerEngine) -> Result<Self, slint::PlatformError> {
        let window = WorkspaceWindow::new()?;
        window.set_nav_items(ModelRc::new(VecModel::from(nav_items())));
        let shell = Rc::new(Shell {
            window,
            engine: engine.clone(),
            created: RefCell::new(vec![false; pages::PAGES.len()]),
            pending_arg: RefCell::new(None),
            changed_handler: Cell::new(None),
            menu_hooks: RefCell::new(HashMap::new()),
            me: RefCell::new(std::rc::Weak::new()),
        });
        *shell.me.borrow_mut() = Rc::downgrade(&shell);
        let (dark, reduce_motion) = hb_ui_common::theme::current();
        let theme = shell.window.global::<Theme>();
        theme.set_dark(dark);
        theme.set_reduce_motion(reduce_motion);

        let weak = Rc::downgrade(&shell);
        shell.window.on_nav_selected(move |i| {
            if let Some(s) = weak.upgrade()
                && let Some(page) = usize::try_from(i).ok().and_then(|i| pages::PAGES.get(i))
            {
                s.show_page(page.0, None);
            }
        });
        let weak = Rc::downgrade(&shell);
        shell.window.on_key_pressed(move |e| weak.upgrade().is_some_and(|s| s.on_key(&e)));
        let base_url = engine.config().base_url.clone();
        shell.window.on_open_web(move || hb_win::open(&base_url));

        shell.wire_task_menu();

        let weak = Rc::downgrade(&shell);
        let id = engine.events().changed.subscribe(move |_| {
            if let Some(s) = weak.upgrade() {
                s.refresh_chrome();
            }
        });
        shell.changed_handler.set(Some(id));
        shell.refresh_chrome();
        Ok(Workspace(shell))
    }

    pub fn window(&self) -> &WorkspaceWindow {
        &self.0.window
    }

    pub fn show_page(&self, page: &str, arg: Option<PageArg>) {
        self.0.show_page(page, arg);
    }

    pub fn open_day(&self, day: NaiveDate) {
        self.0.show_page("entries", Some(PageArg::Date(day)));
    }

    /// 顯示並帶到前景；切到工作台時背景補抓任務分頁、團隊、待辦（C# `Activated` → `RefreshAllAsync(false)`）。
    pub fn present(&self) -> Result<(), slint::PlatformError> {
        self.0.window.show()?;
        let dark = self.0.window.global::<Theme>().get_dark();
        hb_ui_common::window_util::set_dark_title_bar(self.0.window.window(), dark);
        let engine = self.0.engine.clone();
        engine.clone().spawn(async move { engine.refresh_all(false).await });
        Ok(())
    }

    pub fn is_visible(&self) -> bool {
        self.0.window.window().is_visible()
    }

    /// 深淺色、Windows「顯示動畫」切換時呼叫（C# ThemeManager 換 DynamicResource）；標題列也跟著換。
    /// 建立時已經套用 `hb_ui_common::theme::current()`。
    pub fn set_theme(&self, dark: bool, reduce_motion: bool) {
        let theme = self.0.window.global::<Theme>();
        theme.set_dark(dark);
        theme.set_reduce_motion(reduce_motion);
        if self.is_visible() {
            hb_ui_common::window_util::set_dark_title_bar(self.0.window.window(), dark);
        }
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        if let Some(id) = self.changed_handler.take() {
            self.engine.events().changed.unsubscribe(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // C# RefreshChrome：逾期寫「逾期 N」，審核、測試只寫數字，0 不顯示
    #[test]
    fn 導覽徽章文字() {
        assert_eq!("逾期 3", chrome_badge("tasks", 3, 0, 0));
        assert_eq!("", chrome_badge("tasks", 0, 5, 5));
        assert_eq!("5", chrome_badge("reviews", 0, 5, 0));
        assert_eq!("2", chrome_badge("tests", 0, 0, 2));
        assert_eq!("", chrome_badge("hours", 9, 9, 9));
    }

    #[test]
    fn 導覽分組與指示器位置() {
        let items = nav_items();
        assert_eq!(18, items.len());
        assert_eq!("工作", items[0].group.as_str());
        assert_eq!("知識", items[10].group.as_str());
        assert_eq!("團隊", items[13].group.as_str());
        // 第一列在第一個分組標題（22px）底下；第二列再往下 36＋2
        assert_eq!(22.0, items[0].top);
        assert_eq!(60.0, items[1].top);
        // 「筆記」上面多一個 34px 的分組標題
        assert_eq!(22.0 + 10.0 * 38.0 + 34.0, items[10].top);
    }
}
