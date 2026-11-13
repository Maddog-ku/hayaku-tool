//! 對應 C# `UI/ThemeManager.cs`：淺色／深色主題，跟隨 Windows 的「應用程式模式」或使用者指定。
//! 切換時：hb-ui-common 的小視窗（`hb_ui_common::set_theme`）、這個 crate 的視窗（各自的 `Theme` global）一起換，
//! 標題列也切成對應的深淺色。另外記著 Windows「顯示動畫」有沒有關（`Theme.reduce-motion`）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// 套用到一個視窗；回傳 false 表示視窗已經沒了。
type Applier = Box<dyn Fn(bool, bool) -> bool>;

thread_local! {
    static MODE: RefCell<String> = RefCell::new("System".to_owned());
    static IS_DARK: Cell<bool> = const { Cell::new(false) };
    static APPLIED: Cell<bool> = const { Cell::new(false) };
    static REDUCE_MOTION: Cell<bool> = const { Cell::new(false) };
    static APPLIERS: RefCell<Vec<Applier>> = RefCell::new(Vec::new());
    static CHANGED: RefCell<Vec<Rc<dyn Fn()>>> = RefCell::new(Vec::new());
}

pub struct ThemeManager;

impl ThemeManager {
    /// System（跟隨 Windows）、Light、Dark。
    pub fn mode() -> String {
        MODE.with(|m| m.borrow().clone())
    }

    pub fn is_dark() -> bool {
        IS_DARK.with(Cell::get)
    }

    /// Windows「顯示動畫」關閉（C# `SystemParameters.ClientAreaAnimation == false`）。
    pub fn reduce_motion() -> bool {
        REDUCE_MOTION.with(Cell::get)
    }

    /// 啟動時呼叫。系統深淺色改變時由 App 呼叫 [`ThemeManager::system_changed`]（C# 訂閱 UserPreferenceChanged）。
    pub fn initialize(mode: &str) {
        MODE.with(|m| *m.borrow_mut() = normalize(mode).to_owned());
        REDUCE_MOTION.with(|r| r.set(hb_win::windows_animations_disabled()));
        Self::refresh();
    }

    pub fn set_mode(mode: &str) {
        MODE.with(|m| *m.borrow_mut() = normalize(mode).to_owned());
        Self::refresh();
    }

    /// Windows 的應用程式深淺色改了（只有跟隨 Windows 時才換）。
    pub fn system_changed() {
        if Self::mode() == "System" {
            Self::refresh();
        }
    }

    /// Windows「顯示動畫」改了。
    pub fn animation_changed(enabled: bool) {
        REDUCE_MOTION.with(|r| r.set(!enabled));
        apply_all();
    }

    /// 深淺色換了之後（C# `Changed`）。
    pub fn on_changed(f: impl Fn() + 'static) {
        CHANGED.with(|c| c.borrow_mut().push(Rc::new(f)));
    }

    /// 登記一個視窗：先套用目前的狀態，之後切換時再呼叫；回傳 false（視窗已經沒了）就移除。
    pub fn track(apply: impl Fn(bool, bool) -> bool + 'static) {
        if apply(Self::is_dark(), Self::reduce_motion()) {
            APPLIERS.with(|a| a.borrow_mut().push(Box::new(apply)));
        }
    }

    fn refresh() {
        let dark = resolve_dark(&Self::mode(), hb_win::apps_use_dark_theme());
        if APPLIED.with(Cell::get) && dark == Self::is_dark() {
            return;
        }
        IS_DARK.with(|d| d.set(dark));
        APPLIED.with(|a| a.set(true));
        apply_all();
        log::info!("主題：{}（{}）", if dark { "深色" } else { "淺色" }, Self::mode());
        let handlers: Vec<_> = CHANGED.with(|c| c.borrow().clone());
        for h in handlers {
            h();
        }
    }
}

fn apply_all() {
    let (dark, reduce) = (ThemeManager::is_dark(), ThemeManager::reduce_motion());
    hb_ui_common::set_theme(dark, reduce);
    // 先拿出來再呼叫：套用時又登記新視窗也不會重複借用
    let appliers = APPLIERS.with(|a| std::mem::take(&mut *a.borrow_mut()));
    let alive: Vec<_> = appliers.into_iter().filter(|f| f(dark, reduce)).collect();
    APPLIERS.with(|a| {
        let mut list = a.borrow_mut();
        let added = std::mem::take(&mut *list);
        *list = alive;
        list.extend(added);
    });
}

/// 標題列跟著深淺色（C# ApplyTitleBar）。winit 會依 Windows 的設定自己設標題列，所以 winit 的 theme 也一起改。
pub fn set_title_bar(window: &slint::Window, dark: bool) {
    use slint::winit_030::{WinitWindowAccessor, winit::window::Theme};
    window.with_winit_window(|w| w.set_theme(Some(if dark { Theme::Dark } else { Theme::Light })));
    hb_ui_common::window_util::set_dark_title_bar(window, dark);
}

/// C# `Normalize`：Light、Dark 以外都當 System。
pub fn normalize(mode: &str) -> &'static str {
    match mode {
        "Light" => "Light",
        "Dark" => "Dark",
        _ => "System",
    }
}

/// 要不要用深色：指定的就照指定，跟隨 Windows 時看系統。
pub fn resolve_dark(mode: &str, system_dark: bool) -> bool {
    match normalize(mode) {
        "Dark" => true,
        "Light" => false,
        _ => system_dark,
    }
}

/// 讓這個 crate 的視窗跟著深淺色：設 `Theme` global、標題列。
#[macro_export]
macro_rules! track_theme {
    ($ui:expr) => {{
        use slint::ComponentHandle as _;
        let weak = $ui.as_weak();
        $crate::theme_manager::ThemeManager::track(move |dark, reduce| match weak.upgrade() {
            Some(ui) => {
                let theme = ui.global::<$crate::ui::Theme>();
                theme.set_dark(dark);
                theme.set_reduce_motion(reduce);
                $crate::theme_manager::set_title_bar(ui.window(), dark);
                true
            }
            None => false,
        });
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 模式正規化() {
        assert_eq!("Light", normalize("Light"));
        assert_eq!("Dark", normalize("Dark"));
        assert_eq!("System", normalize("System"));
        assert_eq!("System", normalize("dark"));
        assert_eq!("System", normalize(""));
    }

    #[test]
    fn 深色判斷() {
        assert!(resolve_dark("Dark", false));
        assert!(!resolve_dark("Light", true));
        assert!(resolve_dark("System", true));
        assert!(!resolve_dark("System", false));
        assert!(resolve_dark("whatever", true));
    }
}
