//! 對應 C#：`src/HayakuBar/UI/TaskbarWidget.cs`
//!
//! 嵌進 Windows 工作列（Shell_TrayWnd 的子視窗）的工時顯示。用 UpdateLayeredWindow 畫逐像素透明的內容，
//! 背景跟著工作列的顏色與透明效果。可選基準位置、拖曳左右微調，會自動避開工作列圖示。
//! explorer 重啟或工作列重建時會自動重新嵌入（每 2 秒檢查，另外收到 TaskbarCreated 時立即重嵌）。
//!
//! 跟 C# 不同：外觀改成新設計（見 `render.rs`），數字變化時有數字滾輪動畫（SetTimer 逐格重畫，
//! Windows 關閉動畫時直接換）。計時器與 TaskbarCreated 由一個隱藏的頂層「控制視窗」接收
//! （C# 用 DispatcherTimer；嵌入的子視窗會跟著 explorer 一起消失，不能放計時器）。

mod render;
mod roller;
mod text;

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ, ReleaseDC, SelectObject,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    ReleaseCapture, SetCapture, TME_LEAVE, TRACKMOUSEEVENT, TrackMouseEvent,
};
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetCursorPos, GetWindowRect, HWND_TOP, KillTimer, RegisterWindowMessageW,
    SWP_NOACTIVATE, SWP_NOSIZE, SWP_SHOWWINDOW, SetTimer, SetWindowPos, ULW_ALPHA, UpdateLayeredWindow,
    WM_CAPTURECHANGED, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONUP, WM_SETTINGCHANGE, WM_TIMER, WS_CHILD,
    WS_CLIPSIBLINGS, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::native::{Point, Rect};
use crate::taskbar_icon_scanner::TaskbarIconScanner;
use crate::theme;
use crate::window::{Handler, OwnedWindow, WindowSpec, wide};
use render::{Rollers, Visual};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskbarAnchor {
    Left,
    BeforeStart,
    AfterIcons,
    #[default]
    BeforeTray,
}

impl TaskbarAnchor {
    /// C# `ToString()`（存進 config.json 的值）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::BeforeStart => "BeforeStart",
            Self::AfterIcons => "AfterIcons",
            Self::BeforeTray => "BeforeTray",
        }
    }

    /// C# `Enum.TryParse(ignoreCase: true)`。
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Left, Self::BeforeStart, Self::AfterIcons, Self::BeforeTray]
            .into_iter()
            .find(|a| a.as_str().eq_ignore_ascii_case(value.trim()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetStatus {
    #[default]
    Idle,
    Running,
    Error,
}

/// 工作列上的顯示方式：一排（寬）或兩排（窄）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WidgetLayout {
    OneLine,
    #[default]
    TwoLine,
}

impl WidgetLayout {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneLine => "OneLine",
            Self::TwoLine => "TwoLine",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        [Self::OneLine, Self::TwoLine].into_iter().find(|l| l.as_str().eq_ignore_ascii_case(value.trim()))
    }
}

/// 對應 C# `record WidgetContent(Total, Expected, Progress, Running, Claude, Status, Unread = 0)`。
#[derive(Debug, Clone, PartialEq)]
pub struct WidgetContent {
    pub total: String,
    pub expected: String,
    pub progress: f64,
    pub running: i32,
    pub claude: bool,
    pub status: WidgetStatus,
    pub unread: i32,
}

impl Default for WidgetContent {
    fn default() -> Self {
        Self {
            total: "—".into(),
            expected: String::new(),
            progress: 0.0,
            running: 0,
            claude: false,
            status: WidgetStatus::Idle,
            unread: 0,
        }
    }
}

/// 跨程序讀工作列圖示（UI Automation）很耗資源：每隔幾次檢查才掃一次。
const ICON_SCAN_EVERY: u32 = 8;
const WATCH_TIMER: usize = 1;
const WATCH_MS: u32 = 2000;
const ANIM_TIMER: usize = 2;
const ANIM_MS: u32 = 16;
/// Win32_UI_Controls 才有這個常數，C# Native.cs 也是自己定義
const WM_MOUSELEAVE: u32 = 0x02A3;

enum Event {
    Clicked,
    Menu(Point),
    Offset(f64),
    Hover(bool),
}

type List<T> = RefCell<Vec<Rc<T>>>;

#[derive(Default)]
struct Callbacks {
    clicked: List<dyn Fn()>,
    menu: List<dyn Fn(Point)>,
    offset: List<dyn Fn(f64)>,
    hover: List<dyn Fn(bool)>,
}

impl Callbacks {
    fn fire(&self, events: Vec<Event>) {
        for e in events {
            match e {
                Event::Clicked => {
                    let list: Vec<_> = self.clicked.borrow().clone();
                    list.iter().for_each(|f| f());
                }
                Event::Menu(p) => {
                    let list: Vec<_> = self.menu.borrow().clone();
                    list.iter().for_each(|f| f(p));
                }
                Event::Offset(o) => {
                    let list: Vec<_> = self.offset.borrow().clone();
                    list.iter().for_each(|f| f(o));
                }
                Event::Hover(h) => {
                    let list: Vec<_> = self.hover.borrow().clone();
                    list.iter().for_each(|f| f(h));
                }
            }
        }
    }
}

struct Inner {
    me: Weak<RefCell<Inner>>,
    callbacks: Rc<Callbacks>,
    control: Option<OwnedWindow>,
    host: Option<OwnedWindow>,
    icons: TaskbarIconScanner,
    taskbar_created: u32,
    tray: HWND,
    anchor: TaskbarAnchor,
    layout: WidgetLayout,
    offset: f64,
    content: WidgetContent,
    rollers: Rollers,
    clock: Instant,
    anim_running: bool,
    dark: bool,
    scale: f64,
    width: i32,
    height: i32,
    hover: bool,
    down: bool,
    dragging: bool,
    down_x: i32,
    down_offset_px: i32,
    drag_offset_px: i32,
    last_layout: Option<String>,
    watch_count: u32,
    last_tray: Rect,
    screen_rect: Rect,
    taskbar_rect: Rect,
}

fn window_rect(hwnd: HWND) -> Option<Rect> {
    let mut r = RECT::default();
    unsafe { GetWindowRect(hwnd, &mut r).ok().map(|_| r.into()) }
}

fn cursor_pos() -> Option<Point> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok().map(|_| Point { x: p.x, y: p.y }) }
}

fn find_tray() -> HWND {
    unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).unwrap_or_default() }
}

fn opt(v: Option<i32>) -> String {
    v.map(|v| v.to_string()).unwrap_or_default()
}

impl Inner {
    fn now(&self) -> f64 {
        self.clock.elapsed().as_secs_f64() * 1000.0
    }

    fn is_embedded(&self) -> bool {
        self.host.as_ref().is_some_and(|h| h.is_alive())
    }

    fn host_hwnd(&self) -> Option<HWND> {
        self.host.as_ref().map(|h| h.hwnd())
    }

    fn start(&mut self) {
        self.icons.scan_in_background();
        // 控制視窗：計時器（每 2 秒檢查、動畫逐格）、TaskbarCreated、深淺色變更
        let me = self.me.clone();
        let taskbar_created = self.taskbar_created;
        let handler: Rc<Handler> = Rc::new(move |_hwnd, msg, wparam, _lparam| {
            let inner = me.upgrade()?;
            let mut inner = inner.try_borrow_mut().ok()?;
            match msg {
                WM_TIMER if wparam.0 == WATCH_TIMER => {
                    inner.watch();
                    Some(LRESULT(0))
                }
                WM_TIMER if wparam.0 == ANIM_TIMER => {
                    inner.anim_tick();
                    Some(LRESULT(0))
                }
                WM_SETTINGCHANGE => {
                    // 工作列深淺色改了：立即重畫
                    let dark = theme::taskbar_uses_dark_theme();
                    if dark != inner.dark && inner.is_embedded() {
                        inner.render();
                    }
                    None
                }
                m if m != 0 && m == taskbar_created => {
                    log::info!("工作列重建（TaskbarCreated），重新嵌入");
                    inner.destroy();
                    inner.embed();
                    inner.icons.scan_in_background();
                    None
                }
                _ => None,
            }
        });
        match OwnedWindow::hidden("HayakuBarTaskbarControl", "HayakuBarTaskbarControl", handler) {
            Ok(w) => {
                unsafe {
                    SetTimer(Some(w.hwnd()), WATCH_TIMER, WATCH_MS, None);
                }
                self.control = Some(w);
            }
            Err(e) => log::error!("建立工作列小工具控制視窗失敗：{e}"),
        }
        self.embed();
    }

    fn update(&mut self, content: WidgetContent) {
        if content == self.content {
            return;
        }
        let now = self.now();
        self.rollers.set(&content, now, theme::client_area_animation());
        self.content = content;
        if self.is_embedded() {
            self.render();
            self.place();
        }
    }

    fn relayout(&mut self) {
        if !self.is_embedded() {
            return;
        }
        self.render();
        self.place();
    }

    fn ensure_animation(&mut self, now: f64) {
        if self.anim_running || !self.rollers.active(now) {
            return;
        }
        if let Some(control) = &self.control {
            unsafe {
                SetTimer(Some(control.hwnd()), ANIM_TIMER, ANIM_MS, None);
            }
            self.anim_running = true;
        }
    }

    fn anim_tick(&mut self) {
        let now = self.now();
        let active = self.rollers.active(now);
        if !active {
            self.rollers.settle(now);
            if let Some(control) = &self.control {
                unsafe {
                    let _ = KillTimer(Some(control.hwnd()), ANIM_TIMER);
                }
            }
            self.anim_running = false;
        }
        if self.is_embedded() {
            self.draw(now);
        }
    }

    fn watch(&mut self) {
        let tray = find_tray();
        if tray.is_invalid() {
            return;
        }
        if tray != self.tray || !self.is_embedded() {
            self.destroy();
            self.embed();
            return;
        }
        let dpi_scale = unsafe { GetDpiForWindow(self.tray) } as f64 / 96.0;
        let Some(rect) = window_rect(self.tray) else {
            log::error!("工作列小工具更新失敗：讀不到工作列位置");
            return;
        };
        let tray_changed = rect.left != self.last_tray.left
            || rect.right != self.last_tray.right
            || rect.height() != self.last_tray.height();
        self.last_tray = rect;
        if tray_changed || {
            self.watch_count = self.watch_count.wrapping_add(1);
            self.watch_count.is_multiple_of(ICON_SCAN_EVERY)
        } {
            self.icons.scan_in_background();
        }
        if (dpi_scale - self.scale).abs() > 0.001 || rect.height() != self.height {
            self.render();
        }
        self.place();
    }

    fn embed(&mut self) {
        self.tray = find_tray();
        if self.tray.is_invalid() {
            return;
        }
        let tray_rect = window_rect(self.tray).unwrap_or_default();
        let me = self.me.clone();
        let callbacks = self.callbacks.clone();
        let handler: Rc<Handler> = Rc::new(move |hwnd, msg, wparam, lparam| {
            let inner = me.upgrade()?;
            let events = {
                let mut inner = inner.try_borrow_mut().ok()?;
                inner.on_message(hwnd, msg, wparam, lparam)
            };
            callbacks.fire(events);
            None
        });
        let spec = WindowSpec {
            class: "HayakuBarTaskbarWidget",
            title: "HayakuBar",
            style: WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            ex_style: WS_EX_LAYERED | WS_EX_TOOLWINDOW,
            x: 0,
            y: 0,
            width: 1,
            height: tray_rect.height().max(1),
            parent: Some(self.tray),
        };
        match OwnedWindow::create(&spec, handler) {
            Ok(w) => {
                self.host = Some(w);
                self.render();
                self.place();
                log::info!("已嵌入工作列");
            }
            Err(e) => {
                log::error!("嵌入工作列失敗：{e}");
                self.destroy();
            }
        }
    }

    fn destroy(&mut self) {
        // OwnedWindow 的 Drop 會 DestroyWindow（已被 explorer 摧毀時略過）
        self.host = None;
    }

    fn offset_px(&self) -> i32 {
        if self.dragging { self.drag_offset_px } else { (self.offset * self.scale).round() as i32 }
    }

    fn child_left(&self, class: PCWSTR) -> Option<i32> {
        let child = unsafe { FindWindowExW(Some(self.tray), None, class, PCWSTR::null()).ok()? };
        if child.is_invalid() {
            return None;
        }
        window_rect(child).filter(|r| !r.is_empty()).map(|r| r.left)
    }

    fn place(&mut self) {
        let Some(host) = self.host_hwnd() else { return };
        let Some(tray) = window_rect(self.tray) else { return };
        self.taskbar_rect = tray;
        let width = self.width;
        let margin = (8.0 * self.scale) as i32;
        let tray_left = self.child_left(w!("TrayNotifyWnd")).map_or(tray.width(), |n| n - tray.left);
        let (mut icons_left, mut icons_right) = (None, None);
        if let Some(icons) = self.icons.range() {
            icons_left = Some(icons.left - tray.left);
            icons_right = Some(icons.right - tray.left);
        } else if let Some(start) = self.child_left(w!("Start")) {
            icons_left = Some(start - tray.left);
        }

        let before_icons = icons_left.unwrap_or(2 * margin + width) - width - margin;
        let after_icons = match icons_right {
            Some(ir) => ir + margin,
            None => tray_left - width - margin,
        };
        let mut x = match self.anchor {
            TaskbarAnchor::Left => margin,
            TaskbarAnchor::BeforeStart => before_icons,
            TaskbarAnchor::AfterIcons => after_icons,
            TaskbarAnchor::BeforeTray => tray_left - width - margin,
        } + self.offset_px();

        // 疊到圖示或通知區域時（例如開了很多 app），改放到最近一個放得下的位置
        let overlaps = |pos: i32| {
            matches!((icons_left, icons_right), (Some(l), Some(r)) if pos < r && pos + width > l)
                || pos + width > tray_left
        };
        if overlaps(x) {
            let mut candidates: Vec<i32> = [before_icons, after_icons, margin]
                .into_iter()
                .filter(|&c| c >= 0 && c + width <= tray.width() && !overlaps(c))
                .collect();
            // OrderBy 是穩定排序：距離相同時保持原順序
            candidates.sort_by_key(|c| (c - x).abs());
            if let Some(&c) = candidates.first() {
                x = c;
            }
        }
        x = x.clamp(0, (tray.width() - width).max(0));
        let layout_key = format!(
            "{} x={x} w={width} icons={}..{} tray={tray_left} offset={}",
            self.anchor.as_str(),
            opt(icons_left),
            opt(icons_right),
            self.offset_px()
        );
        if self.last_layout.as_deref() != Some(layout_key.as_str()) && !self.dragging {
            log::info!("工作列位置 {layout_key}");
            self.last_layout = Some(layout_key);
        }
        unsafe {
            let _ = SetWindowPos(host, Some(HWND_TOP), x, 0, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        self.screen_rect =
            Rect { left: tray.left + x, top: tray.top, right: tray.left + x + width, bottom: tray.top + self.height };
    }

    // ───────────── 繪製 ─────────────

    /// 重新讀 DPI、工作列高度與深淺色後重畫（C# Render）。
    fn render(&mut self) {
        let mut scale = unsafe { GetDpiForWindow(self.tray) } as f64 / 96.0;
        if scale <= 0.0 {
            scale = 1.0;
        }
        self.scale = scale;
        self.height = window_rect(self.tray).map_or(1, |r| r.height()).max(1);
        self.dark = theme::taskbar_uses_dark_theme();
        let now = self.now();
        self.draw(now);
    }

    /// 用目前的設定畫一格（動畫逐格只呼叫這個）。
    fn draw(&mut self, now: f64) {
        let Some(host) = self.host_hwnd() else { return };
        let visual = Visual {
            content: &self.content,
            rollers: &self.rollers,
            now,
            scale: self.scale,
            height: self.height,
            two_line: self.layout == WidgetLayout::TwoLine,
            dark: self.dark,
            hover: self.hover,
            down: self.down,
        };
        let Some(pixmap) = render::render(&visual) else {
            log::error!("工作列小工具繪製失敗");
            return;
        };
        self.width = pixmap.width() as i32;
        present(host, &pixmap);
        // 還在滾動（例如嵌入前就換過數字）就開逐格計時器
        self.ensure_animation(now);
    }

    // ───────────── 滑鼠 ─────────────

    fn on_message(&mut self, hwnd: HWND, msg: u32, _wparam: WPARAM, lparam: LPARAM) -> Vec<Event> {
        let mut events = Vec::new();
        match msg {
            WM_MOUSEMOVE => {
                if !self.hover {
                    self.hover = true;
                    let mut tme = TRACKMOUSEEVENT {
                        cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    unsafe {
                        let _ = TrackMouseEvent(&mut tme);
                    }
                    self.render();
                    events.push(Event::Hover(true));
                }
                if self.down
                    && let Some(p) = cursor_pos()
                {
                    if !self.dragging && ((p.x - self.down_x).abs() as f64) > 4.0 * self.scale {
                        self.dragging = true;
                    }
                    if self.dragging {
                        self.drag_offset_px = self.down_offset_px + (p.x - self.down_x);
                        self.place();
                    }
                }
            }
            WM_LBUTTONDOWN => {
                events.push(Event::Hover(false));
                if let Some(down) = cursor_pos() {
                    self.down = true;
                    self.dragging = false;
                    self.down_x = down.x;
                    self.down_offset_px = (self.offset * self.scale).round() as i32;
                    unsafe {
                        SetCapture(hwnd);
                    }
                    self.render();
                }
            }
            WM_LBUTTONUP => {
                if !self.down {
                    return events;
                }
                let was_dragging = self.dragging;
                if was_dragging {
                    self.offset = self.drag_offset_px as f64 / self.scale;
                    self.dragging = false;
                }
                self.down = false;
                unsafe {
                    let _ = ReleaseCapture();
                }
                self.render();
                self.place();
                events.push(if was_dragging { Event::Offset(self.offset) } else { Event::Clicked });
            }
            WM_CAPTURECHANGED => {
                if self.down && lparam.0 != hwnd.0 as isize {
                    self.down = false;
                    self.dragging = false;
                    self.render();
                    self.place();
                }
            }
            WM_MOUSELEAVE => {
                self.hover = false;
                self.render();
                events.push(Event::Hover(false));
            }
            WM_RBUTTONUP => {
                if let Some(at) = cursor_pos() {
                    events.push(Event::Menu(at));
                }
            }
            _ => {}
        }
        events
    }
}

fn present(hwnd: HWND, pixmap: &tiny_skia::Pixmap) {
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    unsafe {
        let screen_dc = GetDC(None);
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h, // 由上往下
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        match CreateDIBSection(Some(mem_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0) {
            Ok(bitmap) if !bits.is_null() => {
                let out = std::slice::from_raw_parts_mut(bits as *mut u8, (w * h * 4) as usize);
                render::to_bgra(pixmap, out);
                let old = SelectObject(mem_dc, HGDIOBJ(bitmap.0));
                let size = SIZE { cx: w, cy: h };
                let src = POINT::default();
                let blend = BLENDFUNCTION {
                    BlendOp: AC_SRC_OVER as u8,
                    BlendFlags: 0,
                    SourceConstantAlpha: 255,
                    AlphaFormat: AC_SRC_ALPHA as u8,
                };
                if let Err(e) = UpdateLayeredWindow(
                    hwnd,
                    Some(screen_dc),
                    None,
                    Some(&size),
                    Some(mem_dc),
                    Some(&src),
                    COLORREF(0),
                    Some(&blend),
                    ULW_ALPHA,
                ) {
                    log::error!("UpdateLayeredWindow 失敗：{e}");
                }
                SelectObject(mem_dc, old);
                let _ = DeleteObject(HGDIOBJ(bitmap.0));
            }
            Ok(_) => log::error!("UpdateLayeredWindow 失敗：CreateDIBSection 沒有像素"),
            Err(e) => log::error!("UpdateLayeredWindow 失敗：{e}"),
        }
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);
    }
}

/// 工作列上的工時小工具。在 UI 執行緒建立與使用。
pub struct TaskbarWidget {
    inner: Rc<RefCell<Inner>>,
    callbacks: Rc<Callbacks>,
}

impl TaskbarWidget {
    pub fn new(anchor: TaskbarAnchor, offset: f64, layout: WidgetLayout) -> Self {
        let callbacks = Rc::new(Callbacks::default());
        let taskbar_created = unsafe { RegisterWindowMessageW(PCWSTR(wide("TaskbarCreated").as_ptr())) };
        let inner = Rc::new_cyclic(|me| {
            RefCell::new(Inner {
                me: me.clone(),
                callbacks: callbacks.clone(),
                control: None,
                host: None,
                icons: TaskbarIconScanner::new(),
                taskbar_created,
                tray: HWND::default(),
                anchor,
                layout,
                offset,
                content: WidgetContent::default(),
                rollers: Rollers::default(),
                clock: Instant::now(),
                anim_running: false,
                dark: true,
                scale: 1.0,
                width: 0,
                height: 0,
                hover: false,
                down: false,
                dragging: false,
                down_x: 0,
                down_offset_px: 0,
                drag_offset_px: 0,
                last_layout: None,
                watch_count: 0,
                last_tray: Rect::default(),
                screen_rect: Rect::default(),
                taskbar_rect: Rect::default(),
            })
        });
        {
            let mut i = inner.borrow_mut();
            let content = i.content.clone();
            i.rollers.set(&content, 0.0, false);
        }
        Self { inner, callbacks }
    }

    fn with<R: Default>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        match self.inner.try_borrow_mut() {
            Ok(mut inner) => f(&mut inner),
            Err(_) => {
                log::error!("工作列小工具更新失敗：重入呼叫");
                R::default()
            }
        }
    }

    fn get<R: Default>(&self, f: impl FnOnce(&Inner) -> R) -> R {
        self.inner.try_borrow().map(|i| f(&i)).unwrap_or_default()
    }

    /// 開始：掃描工作列圖示、嵌入、每 2 秒檢查一次。
    pub fn start(&self) {
        self.with(|i| i.start());
    }

    pub fn update(&self, content: WidgetContent) {
        self.with(|i| i.update(content));
    }

    /// 位置或顯示方式改了之後呼叫。
    pub fn relayout(&self) {
        self.with(|i| i.relayout());
    }

    pub fn anchor(&self) -> TaskbarAnchor {
        self.get(|i| i.anchor)
    }

    pub fn set_anchor(&self, anchor: TaskbarAnchor) {
        self.with(|i| i.anchor = anchor);
    }

    pub fn layout(&self) -> WidgetLayout {
        self.get(|i| i.layout)
    }

    pub fn set_layout(&self, layout: WidgetLayout) {
        self.with(|i| i.layout = layout);
    }

    /// 拖曳微調的偏移量（DIP，正數往右）。
    pub fn offset(&self) -> f64 {
        self.get(|i| i.offset)
    }

    pub fn set_offset(&self, offset: f64) {
        self.with(|i| i.offset = offset);
    }

    pub fn is_embedded(&self) -> bool {
        self.get(|i| i.is_embedded())
    }

    /// 目前在螢幕上的位置（實體像素）。
    pub fn screen_rect(&self) -> Rect {
        self.get(|i| i.screen_rect)
    }

    /// 工作列的位置（實體像素）。
    pub fn taskbar_rect(&self) -> Rect {
        self.get(|i| i.taskbar_rect)
    }

    pub fn scale(&self) -> f64 {
        self.get(|i| i.scale)
    }

    pub fn on_clicked(&self, f: impl Fn() + 'static) {
        self.callbacks.clicked.borrow_mut().push(Rc::new(f));
    }

    /// 右鍵：參數是游標的螢幕座標（實體像素）。
    pub fn on_menu_requested(&self, f: impl Fn(Point) + 'static) {
        self.callbacks.menu.borrow_mut().push(Rc::new(f));
    }

    /// 拖曳結束：參數是新的偏移量（DIP）。
    pub fn on_offset_committed(&self, f: impl Fn(f64) + 'static) {
        self.callbacks.offset.borrow_mut().push(Rc::new(f));
    }

    /// 滑鼠移進（true）／移出、按下（false）工時。
    pub fn on_hover_changed(&self, f: impl Fn(bool) + 'static) {
        self.callbacks.hover.borrow_mut().push(Rc::new(f));
    }
}

impl Drop for TaskbarWidget {
    fn drop(&mut self) {
        if let Ok(mut i) = self.inner.try_borrow_mut() {
            if let Some(control) = &i.control {
                unsafe {
                    let _ = KillTimer(Some(control.hwnd()), WATCH_TIMER);
                    let _ = KillTimer(Some(control.hwnd()), ANIM_TIMER);
                }
            }
            i.destroy();
            i.control = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_與_layout_字串() {
        assert_eq!(TaskbarAnchor::parse("beforetray"), Some(TaskbarAnchor::BeforeTray));
        assert_eq!(TaskbarAnchor::parse("AfterIcons"), Some(TaskbarAnchor::AfterIcons));
        assert_eq!(TaskbarAnchor::parse("x"), None);
        assert_eq!(WidgetLayout::parse("oneline"), Some(WidgetLayout::OneLine));
        assert_eq!(TaskbarAnchor::Left.as_str(), "Left");
    }
}
