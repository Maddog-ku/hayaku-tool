//! 深淺色與「顯示動畫」狀態（C# `ThemeManager` 的這個 crate 端）：程式主體在啟動與系統切換時呼叫 [`set_theme`]，
//! 這裡開的小視窗會跟著換（Slint 的 global 是每個視窗各一份，要一個一個設）；標題列也跟著切深淺色。

use std::cell::{Cell, RefCell};

/// 套用到一個視窗；回傳 false 表示視窗已經沒了。
type Applier = Box<dyn Fn(bool, bool) -> bool>;

thread_local! {
    static STATE: Cell<(bool, bool)> = const { Cell::new((false, false)) };
    static APPLIERS: RefCell<Vec<Applier>> = RefCell::new(Vec::new());
}

/// 設定深色與減少動畫；已經開著的小視窗馬上套用。只能在 UI 執行緒呼叫。
pub fn set_theme(dark: bool, reduce_motion: bool) {
    STATE.with(|s| s.set((dark, reduce_motion)));
    // 先拿出來再呼叫，套用時開新視窗也不會重複借用
    let appliers = APPLIERS.with(|a| std::mem::take(&mut *a.borrow_mut()));
    let alive: Vec<_> = appliers.into_iter().filter(|f| f(dark, reduce_motion)).collect();
    APPLIERS.with(|a| {
        let mut list = a.borrow_mut();
        let added = std::mem::take(&mut *list);
        *list = alive;
        list.extend(added);
    });
}

/// 目前的 (深色, 減少動畫)。
pub fn current() -> (bool, bool) {
    STATE.with(Cell::get)
}

/// 登記一個視窗：先套用目前狀態，之後 [`set_theme`] 時再呼叫；回傳 false（視窗已經沒了）就移除。
pub(crate) fn track(apply: impl Fn(bool, bool) -> bool + 'static) {
    let (dark, reduce) = current();
    if apply(dark, reduce) {
        APPLIERS.with(|a| a.borrow_mut().push(Box::new(apply)));
    }
}
