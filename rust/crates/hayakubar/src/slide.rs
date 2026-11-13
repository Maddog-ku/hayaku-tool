//! 浮動視窗（面板、滑鼠停留的摘要、提示）的進出場動畫：從工作列「長出來」、收起時滑回工作列。
//!
//! 介面固定用軟體繪圖，不能縮放旋轉；這裡直接移動原生視窗：用 `slint::Timer` 每格 `SetWindowPos`，
//! 同時用 `SetWindowRgn` 把跑到裁切線外（工作列那一側）的部分裁掉，看起來像從工作列長出來。
//! 工作列在上／左／右時方向跟著換。動畫途中可以反轉（從目前的位置接著播，時間依剩下的距離縮短），
//! 播完一定拿掉裁切（`SetWindowRgn(None)`，Windows 11 的圓角與邊框才會回來）。
//! 另外有「平移」：提示一則關掉、其他則補位時，從舊位置平滑移到新位置。
//! 每一格只移動視窗、改裁切範圍，不需要 Slint 重畫內容；Windows「顯示動畫」關閉時呼叫端直接跳到結果。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{
    CombineRgn, CreateRectRgn, CreateRoundRectRgn, DeleteObject, HGDIOBJ, HRGN, RGN_AND, SetWindowRgn,
};
use windows::Win32::UI::WindowsAndMessaging::{SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SetWindowPos};

use hb_win::Rect;

/// 一格的間隔（約 60 fps）。
const FRAME: Duration = Duration::from_millis(15);

/// 視窗從哪一邊長出來（工作列在哪一邊）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Edge {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

/// 工作列在哪一邊：anchor（工作列上工時的位置）在工作區的哪一側。
/// 自動隱藏的工作列（anchor 在工作區裡面）當成在下面。
pub fn taskbar_edge(anchor: Rect, work: Rect) -> Edge {
    if anchor.top >= work.bottom {
        Edge::Bottom
    } else if anchor.bottom <= work.top {
        Edge::Top
    } else if anchor.right <= work.left {
        Edge::Left
    } else if anchor.left >= work.right {
        Edge::Right
    } else {
        Edge::Bottom
    }
}

/// 裁切範圍：工作區；工作列那一側改用 anchor 的邊（自動隱藏的工作列蓋在工作區裡面時，從工時的上緣長出來）。
pub fn clip_rect(anchor: Rect, work: Rect, edge: Edge) -> Rect {
    let mut clip = work;
    match edge {
        Edge::Bottom => clip.bottom = clip.bottom.min(anchor.top),
        Edge::Top => clip.top = clip.top.max(anchor.bottom),
        Edge::Left => clip.left = clip.left.max(anchor.right),
        Edge::Right => clip.right = clip.right.min(anchor.left),
    }
    clip
}

/// 一格的結果：視窗左上角（螢幕座標）與看得到的範圍（視窗座標）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub visible: Rect,
}

/// 進度 p（0 完全藏在裁切線外、1 在定位）時視窗的位置與看得到的範圍。
/// 只裁工作列那一側：其他三邊照視窗本身的大小。
pub fn frame(target: Rect, clip: Rect, edge: Edge, p: f32) -> Frame {
    let (w, h) = (target.width(), target.height());
    // 完全藏起來要移動的距離：視窗那一側的邊要退到裁切線外
    let distance = match edge {
        Edge::Bottom => clip.bottom - target.top,
        Edge::Top => target.bottom - clip.top,
        Edge::Left => target.right - clip.left,
        Edge::Right => clip.right - target.left,
    }
    .max(0);
    let offset = ((1.0 - p.clamp(0.0, 1.0)) * distance as f32).round() as i32;
    let (x, y) = match edge {
        Edge::Bottom => (target.left, target.top + offset),
        Edge::Top => (target.left, target.top - offset),
        Edge::Left => (target.left - offset, target.top),
        Edge::Right => (target.left + offset, target.top),
    };
    Frame { x, y, visible: visible_at(x, y, (w, h), clip, edge) }
}

/// 視窗左上角在 (x, y) 時看得到的範圍（視窗座標）：只裁工作列那一側。
pub fn visible_at(x: i32, y: i32, size: (i32, i32), clip: Rect, edge: Edge) -> Rect {
    let (w, h) = size;
    match edge {
        Edge::Bottom => Rect { left: 0, top: 0, right: w, bottom: (clip.bottom - y).clamp(0, h) },
        Edge::Top => Rect { left: 0, top: (clip.top - y).clamp(0, h), right: w, bottom: h },
        Edge::Left => Rect { left: (clip.left - x).clamp(0, w), top: 0, right: w, bottom: h },
        Edge::Right => Rect { left: 0, top: 0, right: (clip.right - x).clamp(0, w), bottom: h },
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)`：t（時間比例 0～1）→ 進度。
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t == 0.0 || t == 1.0 {
        return t;
    }
    let bez = |a: f32, b: f32, s: f32| {
        let u = 1.0 - s;
        3.0 * u * u * s * a + 3.0 * u * s * s * b + s * s * s
    };
    // 用二分法找 x(s) = t 的 s（x 單調遞增）
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    let mut s = t;
    for _ in 0..30 {
        let x = bez(x1, x2, s);
        if (x - t).abs() < 1e-5 {
            break;
        }
        if x < t {
            lo = s;
        } else {
            hi = s;
        }
        s = (lo + hi) / 2.0;
    }
    bez(y1, y2, s)
}

/// 出場（長出來）：平滑減速（hb-ui-kit `Motion.out`）。
pub fn ease_out(t: f32) -> f32 {
    cubic_bezier(0.22, 1.0, 0.36, 1.0, t)
}

/// 收起（滑回去）：加速離開。
pub fn ease_in(t: f32) -> f32 {
    cubic_bezier(0.55, 0.055, 0.675, 0.19, t)
}

type Done = Box<dyn FnOnce()>;

#[derive(Default)]
struct Inner {
    timer: slint::Timer,
    hwnd: Cell<Option<HWND>>,
    target: Cell<Rect>,
    clip: Cell<Rect>,
    edge: Cell<Edge>,
    /// 圓角半徑（實體像素；Windows 11 視窗圓角 8）
    radius: Cell<i32>,
    /// 目前進度（0 藏起、1 在定位）
    p: Cell<f32>,
    from: Cell<f32>,
    to: Cell<f32>,
    started: Cell<Option<Instant>>,
    duration: Cell<Duration>,
    /// 平移：開始時與目標位置的差，隨時間縮到 0
    glide: Cell<(i32, i32)>,
    glide_started: Cell<Option<Instant>>,
    glide_duration: Cell<Duration>,
    /// 目前有裁切
    clipped: Cell<bool>,
    done: RefCell<Option<Done>>,
    /// 裁切變了（露出更多或拿掉）：要整個視窗重畫。被裁掉的部分 Windows 畫圖時會跳過，
    /// 之後露出來時 Slint 只補畫有變動的地方，那塊就一直是空白（面板、提示一圈深色、底部錯位）
    on_reveal: RefCell<Option<Rc<dyn Fn()>>>,
}

/// 一個視窗的進出場動畫。
#[derive(Clone, Default)]
pub struct WindowSlide(Rc<Inner>);

impl WindowSlide {
    pub fn new() -> Self {
        Self::default()
    }

    /// 裁切變了就呼叫（呼叫端讓視窗整個重畫，見 widgets.slint 的 FullRepaint）。
    pub fn set_on_reveal(&self, f: impl Fn() + 'static) {
        *self.0.on_reveal.borrow_mut() = Some(Rc::new(f));
    }

    fn revealed(&self) {
        let f = self.0.on_reveal.borrow().clone();
        if let Some(f) = f {
            f();
        }
    }

    /// 現在正在滑動或平移。
    pub fn is_running(&self) -> bool {
        self.0.timer.running()
    }

    /// 目前的進度（0 藏起、1 在定位）。
    pub fn progress(&self) -> f32 {
        self.0.p.get()
    }

    /// 目前進度下視窗左上角該在的位置（還沒有原生視窗時，用 Slint 先擺好）。
    pub fn position(&self) -> (i32, i32) {
        let s = &self.0;
        let f = frame(s.target.get(), s.clip.get(), s.edge.get(), s.p.get());
        let (gx, gy) = self.glide_offset();
        (f.x + gx, f.y + gy)
    }

    /// 設定目標位置與裁切（還沒顯示時先擺好）；p 是起點。不會開始動畫。
    pub fn prepare(&self, hwnd: Option<HWND>, target: Rect, clip: Rect, edge: Edge, radius: i32, p: f32) {
        let s = &self.0;
        if hwnd.is_some() {
            s.hwnd.set(hwnd);
        }
        s.target.set(target);
        s.clip.set(clip);
        s.edge.set(edge);
        s.radius.set(radius);
        s.p.set(p);
        self.apply();
    }

    /// 動畫途中位置或大小變了（內容高度變了、重新排列）：之後的每一格用新的目標；沒在動時直接移過去。
    pub fn retarget(&self, hwnd: Option<HWND>, target: Rect, clip: Rect, edge: Edge) {
        let s = &self.0;
        if hwnd.is_some() {
            s.hwnd.set(hwnd);
        }
        s.target.set(target);
        s.clip.set(clip);
        s.edge.set(edge);
        if !self.is_running() {
            self.apply();
        }
    }

    /// 從目前的進度滑到 to（1 長出來、0 收回去）；duration 是完整走一趟的時間，反轉時依剩下的距離縮短。
    /// 播完呼叫 done（中途被反轉時不呼叫，改由新的那一次負責）。
    pub fn run(&self, to: f32, duration: Duration, done: impl FnOnce() + 'static) {
        let s = &self.0;
        let from = s.p.get();
        s.from.set(from);
        s.to.set(to);
        let scaled = duration.mul_f32((to - from).abs().clamp(0.0, 1.0));
        s.duration.set(scaled);
        s.started.set(Some(Instant::now()));
        *s.done.borrow_mut() = Some(Box::new(done));
        if scaled.is_zero() {
            s.started.set(None);
            s.p.set(to);
            self.apply();
            self.finish_slide();
            return;
        }
        self.start_timer();
    }

    /// 立刻跳到進度 p（停掉動畫、不呼叫 done）；在定位時拿掉裁切。
    /// 事件迴圈還沒開始時 Slint 還沒建立原生視窗，沒辦法播：直接出現在定位。
    pub fn jump(&self, p: f32) {
        let s = &self.0;
        s.timer.stop();
        s.started.set(None);
        s.glide_started.set(None);
        s.glide.set((0, 0));
        s.done.borrow_mut().take();
        s.p.set(p);
        self.apply();
        if p >= 1.0 {
            self.clear_clip();
        }
    }

    /// 已經顯示、拿到原生視窗後：記下視窗並套用目前的進度（裁切）。
    pub fn attach(&self, hwnd: Option<HWND>) {
        if hwnd.is_some() {
            self.0.hwnd.set(hwnd);
        }
        self.apply();
    }

    /// 從現在的位置平移到新的目標（提示補位）；正在滑進來的也一起移。目標沒變時不動（不打斷進行中的平移）。
    pub fn glide_to(&self, target: Rect, duration: Duration) {
        let s = &self.0;
        let old = s.target.get();
        if old == target {
            return;
        }
        let (gx, gy) = self.glide_offset();
        let dx = old.left + gx - target.left;
        let dy = old.top + gy - target.top;
        // 新的範圍跟著平移；裁切線（工作列那一側）不動
        s.target.set(target);
        if (dx == 0 && dy == 0) || duration.is_zero() || s.hwnd.get().is_none() {
            s.glide.set((0, 0));
            s.glide_started.set(None);
            if !self.is_running() {
                self.apply();
            }
            return;
        }
        s.glide.set((dx, dy));
        s.glide_started.set(Some(Instant::now()));
        s.glide_duration.set(duration);
        self.apply();
        self.start_timer();
    }

    /// 立刻停止：直接跳到這一趟的終點（在定位時移到定位），拿掉裁切，不呼叫 done。
    pub fn stop(&self) {
        let s = &self.0;
        s.timer.stop();
        if s.started.take().is_some() {
            s.p.set(s.to.get());
        }
        s.glide_started.set(None);
        s.glide.set((0, 0));
        s.done.borrow_mut().take();
        if s.p.get() >= 1.0 {
            self.apply();
        }
        self.clear_clip();
    }

    fn glide_offset(&self) -> (i32, i32) {
        let s = &self.0;
        let Some(started) = s.glide_started.get() else { return (0, 0) };
        let total = s.glide_duration.get().as_secs_f32();
        let t = if total > 0.0 { started.elapsed().as_secs_f32() / total } else { 1.0 };
        let k = 1.0 - ease_out(t);
        let (dx, dy) = s.glide.get();
        ((dx as f32 * k).round() as i32, (dy as f32 * k).round() as i32)
    }

    fn start_timer(&self) {
        if self.0.timer.running() {
            return;
        }
        let weak = Rc::downgrade(&self.0);
        self.0.timer.start(slint::TimerMode::Repeated, FRAME, move || {
            if let Some(inner) = weak.upgrade() {
                WindowSlide(inner).tick();
            }
        });
    }

    fn tick(&self) {
        let s = &self.0;
        let mut slide_done = false;
        if let Some(started) = s.started.get() {
            let total = s.duration.get().as_secs_f32();
            let t = if total > 0.0 { (started.elapsed().as_secs_f32() / total).min(1.0) } else { 1.0 };
            let (from, to) = (s.from.get(), s.to.get());
            let eased = if to >= from { ease_out(t) } else { ease_in(t) };
            s.p.set(from + (to - from) * eased);
            if t >= 1.0 {
                s.p.set(to);
                s.started.set(None);
                slide_done = true;
            }
        }
        if let Some(started) = s.glide_started.get()
            && started.elapsed() >= s.glide_duration.get()
        {
            s.glide_started.set(None);
            s.glide.set((0, 0));
        }
        self.apply();
        if s.started.get().is_none() && s.glide_started.get().is_none() {
            s.timer.stop();
        }
        if slide_done {
            self.finish_slide();
        }
    }

    /// 滑完：在定位時拿掉裁切；藏起時先呼叫 done（藏視窗）再拿掉裁切，下次顯示才不會殘留。
    fn finish_slide(&self) {
        let s = &self.0;
        let done = s.done.borrow_mut().take();
        if s.p.get() >= 1.0 {
            self.clear_clip();
            if let Some(f) = done {
                f();
            }
        } else {
            if let Some(f) = done {
                f();
            }
            self.clear_clip();
        }
    }

    /// 把目前的進度套到視窗：移動、裁切。
    fn apply(&self) {
        let s = &self.0;
        let Some(h) = s.hwnd.get() else { return };
        let target = s.target.get();
        let f = frame(target, s.clip.get(), s.edge.get(), s.p.get());
        let (gx, gy) = self.glide_offset();
        let (x, y) = (f.x + gx, f.y + gy);
        let (w, ht) = (target.width(), target.height());
        // 平移中的視窗不在 frame 算的位置：用實際位置算裁切
        let visible = visible_at(x, y, (w, ht), s.clip.get(), s.edge.get());
        let full = visible == Rect { left: 0, top: 0, right: w, bottom: ht };
        unsafe {
            let _ = SetWindowPos(h, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
        // 沒在播動畫時：完整露出就拿掉裁切；算出來整個看不到（例如自動隱藏的工作列在上、左、右）也拿掉，
        // 不要套空的裁切讓視窗消失
        let empty = visible.right <= visible.left || visible.bottom <= visible.top;
        if (full || empty) && s.started.get().is_none() {
            self.clear_clip();
            return;
        }
        set_region(h, visible, w, ht, s.radius.get());
        s.clipped.set(true);
        self.revealed();
    }

    fn clear_clip(&self) {
        let s = &self.0;
        if !s.clipped.replace(false) {
            return;
        }
        if let Some(h) = s.hwnd.get() {
            unsafe {
                SetWindowRgn(h, None, true);
            }
        }
        self.revealed();
    }
}

/// 視窗只露出 visible（視窗座標）那一塊，四角照 Windows 11 的圓角（動畫途中 DWM 的圓角會因為有裁切而消失）。
fn set_region(h: HWND, visible: Rect, w: i32, ht: i32, radius: i32) {
    unsafe {
        let d = (radius * 2).max(0);
        let round: HRGN = CreateRoundRectRgn(0, 0, w + 1, ht + 1, d, d);
        let rect = CreateRectRgn(visible.left, visible.top, visible.right, visible.bottom);
        if round.is_invalid() || rect.is_invalid() {
            if !round.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(round.0));
            }
            if !rect.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(rect.0));
            }
            return;
        }
        CombineRgn(Some(round), Some(round), Some(rect), RGN_AND);
        let _ = DeleteObject(HGDIOBJ(rect.0));
        // 成功後區域歸系統管，不能再刪
        if SetWindowRgn(h, Some(round), true) == 0 {
            let _ = DeleteObject(HGDIOBJ(round.0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect { left, top, right, bottom }
    }

    const WORK: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1032 };

    #[test]
    fn 工作列在哪一邊() {
        assert_eq!(Edge::Bottom, taskbar_edge(rect(1700, 1032, 1780, 1080), WORK));
        assert_eq!(Edge::Top, taskbar_edge(rect(1700, 0, 1780, 48), rect(0, 48, 1920, 1080)));
        assert_eq!(Edge::Left, taskbar_edge(rect(0, 900, 48, 960), rect(48, 0, 1920, 1080)));
        assert_eq!(Edge::Right, taskbar_edge(rect(1872, 900, 1920, 960), rect(0, 0, 1872, 1080)));
        // 自動隱藏（工作區是整個螢幕）：當成下面
        assert_eq!(Edge::Bottom, taskbar_edge(rect(1700, 1032, 1780, 1080), rect(0, 0, 1920, 1080)));
    }

    #[test]
    fn 自動隱藏時從工時上緣長出來() {
        let clip = clip_rect(rect(1700, 1032, 1780, 1080), rect(0, 0, 1920, 1080), Edge::Bottom);
        assert_eq!(1032, clip.bottom);
        assert_eq!(WORK, clip_rect(rect(1700, 1032, 1780, 1080), WORK, Edge::Bottom));
    }

    #[test]
    fn 工作列在下面_從下緣往上長出來() {
        // 面板 432×600，下緣離工作列 8
        let target = rect(1348, 424, 1780, 1024);
        let hidden = frame(target, WORK, Edge::Bottom, 0.0);
        // 藏起時上緣剛好在工作列上緣，整個看不到
        assert_eq!((1348, 1032), (hidden.x, hidden.y));
        assert_eq!(0, hidden.visible.height());
        let half = frame(target, WORK, Edge::Bottom, 0.5);
        assert_eq!(424 + 304, half.y);
        // 只露出工作區裡面的部分（從上面算起）
        assert_eq!(rect(0, 0, 432, 1032 - 728), half.visible);
        let shown = frame(target, WORK, Edge::Bottom, 1.0);
        assert_eq!((1348, 424), (shown.x, shown.y));
        assert_eq!(rect(0, 0, 432, 600), shown.visible);
    }

    #[test]
    fn 工作列在上面_從上緣往下長出來() {
        let work = rect(0, 48, 1920, 1080);
        let target = rect(1348, 48, 1780, 648);
        let hidden = frame(target, work, Edge::Top, 0.0);
        assert_eq!(48 - 600, hidden.y);
        assert_eq!(0, hidden.visible.height());
        let half = frame(target, work, Edge::Top, 0.5);
        assert_eq!(48 - 300, half.y);
        // 露出下半部
        assert_eq!(rect(0, 300, 432, 600), half.visible);
    }

    #[test]
    fn 工作列在左右_橫向長出來() {
        let work = rect(48, 0, 1920, 1080);
        let target = rect(48, 300, 480, 900);
        let half = frame(target, work, Edge::Left, 0.5);
        assert_eq!((48 - 216, 300), (half.x, half.y));
        assert_eq!(rect(216, 0, 432, 600), half.visible);
        let work = rect(0, 0, 1872, 1080);
        let target = rect(1440, 300, 1872, 900);
        let half = frame(target, work, Edge::Right, 0.5);
        assert_eq!((1440 + 216, 300), (half.x, half.y));
        assert_eq!(rect(0, 0, 216, 600), half.visible);
    }

    #[test]
    fn 提示往右滑出_裁在自己的右緣() {
        // 提示關閉：裁切範圍就是自己，往右滑到看不見
        let target = rect(1436, 900, 1780, 1000);
        let f = frame(target, target, Edge::Right, 0.25);
        assert_eq!(1436 + 258, f.x);
        assert_eq!(rect(0, 0, 86, 100), f.visible);
    }

    #[test]
    fn 緩動曲線() {
        assert_eq!(0.0, ease_out(0.0));
        assert_eq!(1.0, ease_out(1.0));
        // 減速：一開始就走了大半
        assert!(ease_out(0.3) > 0.6);
        // 加速：一開始走得慢
        assert!(ease_in(0.3) < 0.1);
        // 單調遞增
        let mut last = 0.0;
        for i in 0..=20 {
            let v = ease_out(i as f32 / 20.0);
            assert!(v >= last);
            last = v;
        }
        // linear 曲線就是本身
        assert!((cubic_bezier(0.25, 0.25, 0.75, 0.75, 0.4) - 0.4).abs() < 1e-3);
    }
}
