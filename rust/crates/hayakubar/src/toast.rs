//! 對應 C# `UI/ToastWindow.xaml.cs`：工作列工時上方滑上來的提示（`ToastWindow`）與同時顯示的提示管理（`ToastManager`）。
//!
//! 跟 C# 不同：C# 的提示視窗外圈有 8 的透明陰影邊；Rust 版視窗就是卡片本身，位置計算把陰影邊扣掉，
//! 看得到的卡片位置、間距跟 C# 一樣。

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use hb_core::{ToastAction, ToastRequest};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::popup::{self, PopupKind};
use crate::theme_manager::ThemeManager;
use crate::ui::{ToastButton, ToastWindow};

/// 同時最多幾則（C# MaxVisible）。
pub const MAX_VISIBLE: usize = 4;
/// 卡片寬（C# Width 360 扣掉兩邊 8 的陰影邊）。
pub const CARD_WIDTH: f32 = 344.0;

/// 一則提示在排列時的資料（實體像素）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub width: i32,
    pub height: i32,
    pub from_bottom: bool,
}

/// C# `ToastManager.Layout`：一般提示從右下角往上疊；新通知從工作列工時的正上方往上疊，跟面板對齊同一邊。
/// slots 依顯示順序（最舊在前）；回傳每一則的左上角（實體像素）。scale 是螢幕縮放比例。
pub fn layout(slots: &[Slot], anchor: Option<(hb_win::Rect, bool)>, work: hb_win::Rect, scale: f32) -> Vec<(i32, i32)> {
    let s = |v: f32| (v * scale).round() as i32;
    let fallback = s(120.0);
    let mut pos = vec![(0, 0); slots.len()];
    // 一般提示：右下角往上疊（C# 視窗距螢幕邊 4、陰影邊 8；疊的時候陰影邊重疊 4 → 看得到的卡片間隔 12）
    let mut bottom = work.bottom - s(12.0);
    for (i, slot) in slots.iter().enumerate().rev().filter(|(_, t)| !t.from_bottom || anchor.is_none()) {
        let height = if slot.height > 0 { slot.height } else { fallback };
        pos[i] = (work.right - slot.width - s(12.0), bottom - height);
        bottom -= height + s(12.0);
    }
    // 新通知：工作列工時的正上方往上疊（看得到的卡片離工時 8、彼此間隔 8）
    let Some((a, align_left)) = anchor else { return pos };
    let margin = s(8.0);
    let mut bottom = a.top.min(work.bottom) - margin;
    for (i, slot) in slots.iter().enumerate().rev().filter(|(_, t)| t.from_bottom) {
        let height = if slot.height > 0 { slot.height } else { fallback };
        let left = if align_left { a.left } else { a.right - slot.width };
        let min = work.left + margin;
        let x = left.clamp(min, min.max(work.right - slot.width - margin));
        pos[i] = (x, bottom - height);
        bottom -= height + margin;
    }
    pos
}

/// 一則提示的停留時間（C#：Sticky 10 分鐘；否則 Duration；都沒有時有按鈕 15 秒、沒有 7 秒）。
pub fn interval_of(request: &ToastRequest) -> Duration {
    if request.sticky {
        Duration::from_secs(600)
    } else {
        request.duration.unwrap_or(Duration::from_secs(if request.actions.is_empty() { 7 } else { 15 }))
    }
}

/// C# `ShowNow` 開頭：沒指定停留時間的套用設定的秒數；設定是「不自動收回」（None）時改成要按掉才關。全部從下面滑上來。
pub fn normalize(mut request: ToastRequest, default_duration: Option<Duration>) -> ToastRequest {
    if !request.sticky && request.duration.is_none() {
        match default_duration {
            None => request.sticky = true,
            Some(d) => request.duration = Some(d),
        }
    }
    request.from_bottom = true;
    request
}

/// 一則提示（C# ToastWindow）。
pub struct Toast {
    ui: ToastWindow,
    tag: Option<String>,
    from_bottom: bool,
    actions: Vec<ToastAction>,
    timer: slint::Timer,
    interval: Duration,
    closing: Cell<bool>,
    dismissed: RefCell<Option<DismissedFn>>,
    me: Weak<Toast>,
}

impl Toast {
    fn new(request: &ToastRequest) -> Option<Rc<Toast>> {
        let ui = match popup::create(PopupKind::Toast, ToastWindow::new) {
            Ok(ui) => ui,
            Err(e) => {
                log::error!("建立提示視窗失敗: {e}");
                return None;
            }
        };
        crate::track_theme!(ui);
        ui.set_title_text(request.title.as_str().into());
        // C#：訊息是空白時收起來
        let message = if request.message.trim().is_empty() { "" } else { request.message.as_str() };
        ui.set_message(message.into());
        let buttons: Vec<ToastButton> = request
            .actions
            .iter()
            .map(|a| ToastButton { label: a.label.as_str().into(), primary: a.primary, enabled: true })
            .collect();
        ui.set_actions(ModelRc::new(VecModel::from(buttons)));
        let toast = Rc::new_cyclic(|me| Toast {
            ui,
            tag: request.tag.clone(),
            from_bottom: request.from_bottom,
            actions: request.actions.clone(),
            timer: slint::Timer::default(),
            interval: interval_of(request),
            closing: Cell::new(false),
            dismissed: RefCell::new(None),
            me: me.clone(),
        });
        let weak = Rc::downgrade(&toast);
        toast.ui.on_action(move |i| {
            if let Some(t) = weak.upgrade() {
                t.run_action(i as usize);
            }
        });
        let weak = Rc::downgrade(&toast);
        toast.ui.on_close_clicked(move || {
            if let Some(t) = weak.upgrade() {
                t.dismiss();
            }
        });
        // 滑鼠移上去時不會自動關閉
        let weak = Rc::downgrade(&toast);
        toast.ui.on_hover_changed(move |over| {
            if let Some(t) = weak.upgrade() {
                if over {
                    t.timer.stop();
                } else if !t.closing.get() {
                    t.start_timer();
                }
            }
        });
        Some(toast)
    }

    fn run_action(&self, index: usize) {
        let Some(action) = self.actions.get(index).cloned() else { return };
        let model = self.ui.get_actions();
        if let Some(mut b) = model.row_data(index) {
            b.enabled = false;
            model.set_row_data(index, b);
        }
        let weak = self.me.clone();
        let run = action.run.clone();
        if let Err(e) = slint::spawn_local(async move {
            // 錯誤都在動作裡處理掉了（C# 這裡 catch 後寫「提示按鈕「…」失敗」）
            run().await;
            if let Some(t) = weak.upgrade() {
                t.dismiss();
            }
        }) {
            log::error!("提示按鈕「{}」失敗: {e}", action.label);
        }
    }

    fn start_timer(&self) {
        let weak = self.me.clone();
        self.timer.start(slint::TimerMode::SingleShot, self.interval, move || {
            if let Some(t) = weak.upgrade() {
                t.dismiss();
            }
        });
    }

    /// 估計的大小（實體像素）：還沒顯示時用 Slint 算的內容高度。
    fn size(&self, scale: f32) -> (i32, i32) {
        popup::outer_size(self.ui.window()).unwrap_or_else(|| {
            let h = self.ui.get_pref_h();
            ((CARD_WIDTH * scale).round() as i32, (h * scale).round() as i32)
        })
    }

    fn show(&self) {
        if let Err(e) = self.ui.show() {
            log::error!("顯示提示失敗: {e}");
            return;
        }
        popup::apply_styles(self.ui.window(), PopupKind::Toast);
        self.ui.set_shown(true);
        self.start_timer();
    }

    /// C# `Dismiss`：滑回下緣後關掉（Windows 關閉動畫效果時直接關）。
    pub fn dismiss(&self) {
        if self.closing.replace(true) {
            return;
        }
        self.timer.stop();
        let weak = self.me.clone();
        let finish = move || {
            if let Some(t) = weak.upgrade() {
                let _ = t.ui.hide();
                if let Some(f) = t.dismissed.borrow_mut().take() {
                    f(&t);
                }
            }
        };
        if ThemeManager::reduce_motion() {
            finish();
            return;
        }
        self.ui.set_closing(true);
        slint::Timer::single_shot(Duration::from_millis(240), finish);
    }

    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }
}

type AnchorFn = Rc<dyn Fn() -> (hb_win::Rect, bool)>;
type DurationFn = Rc<dyn Fn() -> Option<Duration>>;
type DismissedFn = Box<dyn Fn(&Rc<Toast>)>;

#[derive(Default)]
struct ManagerInner {
    open: RefCell<Vec<Rc<Toast>>>,
    held: RefCell<Vec<ToastRequest>>,
    held_timer: RefCell<Option<slint::Timer>>,
    /// 工作列上工時的位置（實體像素）與面板是否靠左展開；新通知從這裡冒出來。
    anchor: RefCell<Option<AnchorFn>>,
    /// 一般提示停留多久（設定的秒數）；None 表示不自動收回。
    default_duration: RefCell<Option<DurationFn>>,
    /// 現在要不要先別跳提示（全螢幕、簡報…）；回 true 時先存起來，之後再顯示。
    is_quiet: RefCell<Option<Rc<dyn Fn() -> bool>>>,
}

/// C# `ToastManager`：管理同時顯示的提示，最多 4 則。
#[derive(Clone, Default)]
pub struct ToastManager(Rc<ManagerInner>);

impl ToastManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_anchor(&self, f: impl Fn() -> (hb_win::Rect, bool) + 'static) {
        *self.0.anchor.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_default_duration(&self, f: impl Fn() -> Option<Duration> + 'static) {
        *self.0.default_duration.borrow_mut() = Some(Rc::new(f));
    }

    pub fn set_is_quiet(&self, f: impl Fn() -> bool + 'static) {
        *self.0.is_quiet.borrow_mut() = Some(Rc::new(f));
    }

    fn quiet(&self) -> bool {
        let f = self.0.is_quiet.borrow().clone();
        f.is_some_and(|f| f())
    }

    /// 開著的提示（截圖用）。
    pub fn open_toasts(&self) -> Vec<Rc<Toast>> {
        self.0.open.borrow().clone()
    }

    /// 關掉這個標記的提示（在 Slack 上處理掉了）；勿擾中還沒顯示的也拿掉。
    pub fn dismiss(&self, tag: &str) {
        self.0.held.borrow_mut().retain(|r| r.tag.as_deref() != Some(tag));
        let matched: Vec<_> = self.0.open.borrow().iter().filter(|t| t.tag() == Some(tag)).cloned().collect();
        for t in matched {
            t.dismiss();
        }
    }

    pub fn show(&self, request: ToastRequest) {
        if !request.important && self.quiet() {
            self.hold(request);
            return;
        }
        self.show_now(request);
    }

    /// 勿擾中：先存起來，每 5 秒看一次，結束後依序顯示（太多則只留最新的幾則）。
    fn hold(&self, request: ToastRequest) {
        {
            let mut held = self.0.held.borrow_mut();
            held.push(request);
            if held.len() > MAX_VISIBLE {
                held.remove(0);
            }
        }
        if self.0.held_timer.borrow().is_some() {
            return;
        }
        let timer = slint::Timer::default();
        let weak = Rc::downgrade(&self.0);
        timer.start(slint::TimerMode::Repeated, Duration::from_secs(5), move || {
            let Some(inner) = weak.upgrade() else { return };
            let me = ToastManager(inner);
            if me.quiet() {
                return;
            }
            // 停掉計時器要延後：不能在它自己的回呼裡丟掉
            let stopped = me.0.held_timer.borrow_mut().take();
            slint::Timer::single_shot(Duration::ZERO, move || drop(stopped));
            let pending: Vec<_> = std::mem::take(&mut *me.0.held.borrow_mut());
            for r in pending {
                me.show_now(r);
            }
        });
        *self.0.held_timer.borrow_mut() = Some(timer);
    }

    fn show_now(&self, request: ToastRequest) {
        // 所有提示都一樣：從工作列工時的位置滑上來，停留設定的秒數後滑回去；
        // 要你處理的（會議快開始、提醒到了…）會停到你按按鈕或關掉
        let default = self.0.default_duration.borrow().clone().and_then(|f| f());
        let request = normalize(request, default);
        let Some(toast) = Toast::new(&request) else { return };
        let weak = Rc::downgrade(&self.0);
        *toast.dismissed.borrow_mut() = Some(Box::new(move |t: &Rc<Toast>| {
            if let Some(inner) = weak.upgrade() {
                inner.open.borrow_mut().retain(|o| !Rc::ptr_eq(o, t));
                ToastManager(inner).layout();
            }
        }));
        self.0.open.borrow_mut().push(toast.clone());
        loop {
            let oldest = {
                let mut open = self.0.open.borrow_mut();
                if open.len() <= MAX_VISIBLE {
                    break;
                }
                open.remove(0) // 先移出清單：Dismiss 有動畫，不會馬上觸發 Dismissed
            };
            oldest.dismiss();
        }
        // 先用估計的大小擺好位置再顯示，一出現就在定位；顯示後用實際大小再排一次
        self.layout();
        toast.show();
        self.layout();
    }

    fn layout(&self) {
        let anchor = self.0.anchor.borrow().clone().map(|f| f());
        let open = self.0.open.borrow().clone();
        if open.is_empty() {
            return;
        }
        let (px, py) = match anchor {
            Some((a, _)) => (a.left, a.top),
            None => {
                let w = popup::primary_work_area();
                (w.right - 1, w.bottom - 1)
            }
        };
        let work = popup::work_area_at(px, py.saturating_sub(1));
        let scale = popup::scale_at(px, py.saturating_sub(1));
        let slots: Vec<Slot> = open
            .iter()
            .map(|t| {
                let (width, height) = t.size(scale);
                Slot { width, height, from_bottom: t.from_bottom }
            })
            .collect();
        for (t, (x, y)) in open.iter().zip(layout(&slots, anchor, work, scale)) {
            t.ui.window().set_position(slint::PhysicalPosition::new(x, y));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> hb_win::Rect {
        hb_win::Rect { left, top, right, bottom }
    }

    const WORK: hb_win::Rect = hb_win::Rect { left: 0, top: 0, right: 1920, bottom: 1032 };

    #[test]
    fn 新通知從工時正上方往上疊_最新的在最下面() {
        let slots =
            [Slot { width: 344, height: 100, from_bottom: true }, Slot { width: 344, height: 80, from_bottom: true }];
        let anchor = rect(1700, 1032, 1780, 1080);
        let pos = layout(&slots, Some((anchor, false)), WORK, 1.0);
        // 最新的（第二則）在最下面：卡片下緣離工時 8
        assert_eq!((1780 - 344, 1032 - 8 - 80), pos[1]);
        // 舊的在上面，間隔 8
        assert_eq!((1780 - 344, 1032 - 8 - 80 - 8 - 100), pos[0]);
    }

    #[test]
    fn 新通知靠左對齊_夾在螢幕裡() {
        let slots = [Slot { width: 344, height: 100, from_bottom: true }];
        let pos = layout(&slots, Some((rect(0, 1032, 80, 1080), true)), WORK, 1.0);
        assert_eq!((8, 1032 - 8 - 100), pos[0]);
        let pos = layout(&slots, Some((rect(1900, 1032, 1915, 1080), true)), WORK, 1.0);
        assert_eq!(1920 - 344 - 8, pos[0].0);
    }

    #[test]
    fn 沒有工時位置時從右下角往上疊() {
        let slots =
            [Slot { width: 344, height: 100, from_bottom: true }, Slot { width: 344, height: 0, from_bottom: false }];
        let pos = layout(&slots, None, WORK, 1.0);
        // 高度還不知道時用 120
        assert_eq!((1920 - 344 - 12, 1032 - 12 - 120), pos[1]);
        assert_eq!((1920 - 344 - 12, 1032 - 12 - 120 - 12 - 100), pos[0]);
    }

    #[test]
    fn 縮放比例套用到間距() {
        let slots = [Slot { width: 516, height: 150, from_bottom: true }];
        let pos = layout(&slots, Some((rect(2500, 1548, 2620, 1620), false)), rect(0, 0, 2880, 1548), 1.5);
        assert_eq!((2620 - 516, 1548 - 12 - 150), pos[0]);
    }

    #[test]
    fn 停留時間() {
        let mut r = ToastRequest::new("t", "m", vec![]);
        assert_eq!(Duration::from_secs(7), interval_of(&r));
        r.actions.push(ToastAction::new("a", || async {}));
        assert_eq!(Duration::from_secs(15), interval_of(&r));
        r.duration = Some(Duration::from_secs(3));
        assert_eq!(Duration::from_secs(3), interval_of(&r));
        r.sticky = true;
        assert_eq!(Duration::from_secs(600), interval_of(&r));
    }

    #[test]
    fn 套用設定的停留秒數() {
        let r = normalize(ToastRequest::new("t", "m", vec![]), Some(Duration::from_secs(3)));
        assert_eq!((Some(Duration::from_secs(3)), false, true), (r.duration, r.sticky, r.from_bottom));
        // 不自動收回：要按掉才關
        let r = normalize(ToastRequest::new("t", "m", vec![]), None);
        assert!(r.sticky);
        // 已經指定的不動
        let r = normalize(
            ToastRequest::new("t", "m", vec![]).duration(Some(Duration::from_secs(30))),
            Some(Duration::from_secs(3)),
        );
        assert_eq!(Some(Duration::from_secs(30)), r.duration);
        let r = normalize(ToastRequest::new("t", "m", vec![]).sticky(true), Some(Duration::from_secs(3)));
        assert_eq!((None, true), (r.duration, r.sticky));
    }
}
