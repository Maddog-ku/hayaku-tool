//! 專案、測試、待我審核、我的工時、工時紀錄、工作日誌、提醒、統計、工作負荷九頁共用的小工具
//! （C# `UI/Work/PageParts.cs`、`Inputs.Show` 的 Rust 端）。`pages/mod.rs` 不能改，所以從 `hours.rs` 以
//! `#[path]` 引入，其他頁用 `super::hours::w2_common`。

use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use chrono::{Datelike, NaiveDate};
use hb_api::DateTimeOffset;
use hb_core::TimerEngine;
use hb_ui_common::ProjectIcons;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use slint::{ModelRc, SharedString, VecModel};

use crate::W2Project;

/// 在 UI 執行緒跑一個 future；排不進去時寫紀錄（C# `async void` 的事件處理）。
pub(crate) fn spawn(what: &str, fut: impl Future<Output = ()> + 'static) {
    if let Err(e) = slint::spawn_local(fut) {
        log::error!("{what}失敗：{e}");
    }
}

pub(crate) fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

pub(crate) fn strings<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    model(items.into_iter().map(|s| SharedString::from(s.as_ref())).collect())
}

/// 訊息列的顏色（@hbc StatusLine 的 tone）：0 錯誤（ErrorBrush）、1 成功（RunBrush）。
pub(crate) fn tone(error: bool) -> i32 {
    if error { 0 } else { 1 }
}

/// `Inputs.Show(target, message, error)` 的文字與顏色；None 是收起來。
pub(crate) fn status(message: Option<&StatusMessage>) -> (SharedString, i32) {
    match message {
        Some(m) => (m.text.as_str().into(), tone(m.error)),
        None => (SharedString::new(), 0),
    }
}

/// `M/d`。
pub(crate) fn md(d: NaiveDate) -> String {
    format!("{}/{}", d.month(), d.day())
}

/// 本地時間 `HH:mm`（C# `t.ToLocalTime():HH:mm`）。
pub(crate) fn hm(t: &DateTimeOffset) -> String {
    hb_core::time::local(t).format("%H:%M").to_string()
}

/// `M/d（ddd）`（zh-TW 的星期簡稱：週一…）。
pub(crate) fn md_weekday(d: NaiveDate) -> String {
    format!("{}/{}（{}）", d.month(), d.day(), inputs::weekday_short(d))
}

/// HayakuApi 的錯誤 → `Inputs.Run` 的錯誤（Hayaku／網路錯誤直接顯示）。
pub(crate) fn mcp<T>(r: Result<T, hb_api::McpError>) -> Result<T, ActionError> {
    r.map_err(ActionError::from)
}

/// 專案下拉的項目（C# `ProjectIcons.Badge(p, p.Name, 16)`，不含封存的）；all 有值時第一項是「全部專案」（key 空字串）。
pub(crate) fn project_items(engine: &TimerEngine, all: Option<&str>) -> Vec<W2Project> {
    let mut items = Vec::new();
    if let Some(text) = all {
        items.push(W2Project { key: "".into(), name: text.into(), glyph: "".into(), tint: slint::Color::default() });
    }
    for p in engine.projects().iter().filter(|p| !p.is_archived) {
        let icon = ProjectIcons::icon(Some(p), &p.name);
        items.push(W2Project {
            key: p.key.as_str().into(),
            name: p.name.as_str().into(),
            glyph: icon.glyph.into(),
            tint: icon.color.to_slint(),
        });
    }
    items
}

/// 等使用者在訊息框按「確定」才做的事（C# `MessageBox.Show(...) == OK`）。
#[derive(Default)]
pub(crate) struct ConfirmSlot(RefCell<Option<Box<dyn FnOnce()>>>);

impl ConfirmSlot {
    pub(crate) fn set(&self, on_ok: impl FnOnce() + 'static) {
        *self.0.borrow_mut() = Some(Box::new(on_ok));
    }

    /// 按了確定：拿出要做的事（取消時丟掉）。
    pub(crate) fn take(&self) -> Option<Box<dyn FnOnce()>> {
        self.0.borrow_mut().take()
    }
}

/// 把頁面 global 的 confirm-ok／confirm-cancel 接到 [`ConfirmSlot`]。
macro_rules! wire_confirm {
    ($window:expr, $global:ty, $slot:expr) => {{
        use ::slint::ComponentHandle as _;
        let g = ($window).global::<$global>();
        let weak = ($window).as_weak();
        let slot: ::std::rc::Rc<$crate::pages::hours::w2_common::ConfirmSlot> = $slot;
        {
            let weak = weak.clone();
            let slot = slot.clone();
            g.on_confirm_ok(move || {
                if let Some(w) = weak.upgrade() {
                    w.global::<$global>().set_confirm_open(false);
                }
                if let Some(f) = slot.take() {
                    f();
                }
            });
        }
        g.on_confirm_cancel(move || {
            if let Some(w) = weak.upgrade() {
                w.global::<$global>().set_confirm_open(false);
            }
            slot.take();
        });
    }};
}
pub(crate) use wire_confirm;

/// 打開訊息框：標題、內容；按確定才呼叫 on_ok。
macro_rules! ask {
    ($window:expr, $global:ty, $slot:expr, $title:expr, $text:expr, $on_ok:expr) => {{
        use ::slint::ComponentHandle as _;
        $slot.set($on_ok);
        let g = ($window).global::<$global>();
        g.set_confirm_title(::slint::SharedString::from($title));
        g.set_confirm_text(::slint::SharedString::from($text));
        g.set_confirm_open(true);
    }};
}
pub(crate) use ask;

/// 每頁的狀態放在 thread_local（工作台關掉再開時 `init` 會換新的一份）。
pub(crate) struct PageCell<T>(RefCell<Option<Rc<T>>>);

impl<T> PageCell<T> {
    pub(crate) const fn new() -> Self {
        Self(RefCell::new(None))
    }

    pub(crate) fn set(&self, page: Rc<T>) {
        *self.0.borrow_mut() = Some(page);
    }

    pub(crate) fn get(&self) -> Option<Rc<T>> {
        self.0.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 日期文字() {
        let d = NaiveDate::from_ymd_opt(2026, 11, 16).unwrap();
        assert_eq!("11/16", md(d));
        assert_eq!("11/16（週一）", md_weekday(d));
    }

    #[test]
    fn 訊息顏色() {
        assert_eq!(0, tone(true));
        assert_eq!(1, tone(false));
        let (text, t) = status(Some(&StatusMessage::ok("已複製")));
        assert_eq!(("已複製", 1), (text.as_str(), t));
        assert_eq!("", status(None).0.as_str());
    }

    #[test]
    fn 訊息框_確定才執行() {
        let slot = ConfirmSlot::default();
        let hit = Rc::new(std::cell::Cell::new(false));
        let h = hit.clone();
        slot.set(move || h.set(true));
        if let Some(f) = slot.take() {
            f();
        }
        assert!(hit.get());
        assert!(slot.take().is_none());
    }
}
