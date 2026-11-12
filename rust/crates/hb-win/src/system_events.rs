//! 系統事件：對應 C# `src/HayakuBar/App.xaml.cs` 用的 `SystemEvents.PowerModeChanged`（喚醒）、
//! `SystemEvents.SessionSwitch`（鎖定／解鎖／登入），以及 `UI/ThemeManager.cs` 的 `SystemEvents.UserPreferenceChanged`
//! （深淺色）。用隱藏的頂層視窗接 WM_POWERBROADCAST、WM_WTSSESSION_CHANGE、WM_SETTINGCHANGE。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::RemoteDesktop::{
    NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification, WTSUnRegisterSessionNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PBT_APMRESUMEAUTOMATIC, WM_POWERBROADCAST, WM_SETTINGCHANGE, WM_SYSCOLORCHANGE, WM_THEMECHANGED,
    WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_LOGON, WTS_SESSION_UNLOCK,
};

use crate::theme;
use crate::window::OwnedWindow;

/// 對應 .NET `SessionSwitchReason` 裡 HayakuBar 用到的三種。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionSwitchReason {
    SessionLock,
    SessionUnlock,
    SessionLogon,
}

type Callbacks<T> = Rc<RefCell<Vec<Rc<dyn Fn(T)>>>>;

#[derive(Default)]
struct Shared {
    resume: Callbacks<()>,
    session: Callbacks<SessionSwitchReason>,
    apps_dark: Callbacks<bool>,
    taskbar_dark: Callbacks<bool>,
    animation: Callbacks<bool>,
    last_apps_dark: Cell<bool>,
    last_taskbar_dark: Cell<bool>,
    last_animation: Cell<bool>,
}

fn fire<T: Copy>(list: &Callbacks<T>, value: T) {
    let callbacks: Vec<_> = list.borrow().clone();
    for cb in callbacks {
        cb(value);
    }
}

/// 在 UI 執行緒建立；Drop 時取消註冊。
pub struct SystemEvents {
    window: Option<OwnedWindow>,
    shared: Rc<Shared>,
}

impl SystemEvents {
    pub fn new() -> Self {
        let shared = Rc::new(Shared::default());
        shared.last_apps_dark.set(theme::apps_use_dark_theme());
        shared.last_taskbar_dark.set(theme::taskbar_uses_dark_theme());
        shared.last_animation.set(theme::client_area_animation());
        let s = shared.clone();
        let handler = Rc::new(move |_hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM| -> Option<LRESULT> {
            match msg {
                // 喚醒：PBT_APMRESUMEAUTOMATIC 每次從睡眠／休眠恢復都會送（不論使用者是否已操作）
                WM_POWERBROADCAST => {
                    if wparam.0 as u32 == PBT_APMRESUMEAUTOMATIC {
                        log::info!("系統從睡眠恢復");
                        fire(&s.resume, ());
                    }
                    Some(LRESULT(1))
                }
                WM_WTSSESSION_CHANGE => {
                    let reason = match wparam.0 as u32 {
                        WTS_SESSION_LOCK => Some(SessionSwitchReason::SessionLock),
                        WTS_SESSION_UNLOCK => Some(SessionSwitchReason::SessionUnlock),
                        WTS_SESSION_LOGON => Some(SessionSwitchReason::SessionLogon),
                        _ => None,
                    };
                    if let Some(reason) = reason {
                        fire(&s.session, reason);
                    }
                    Some(LRESULT(0))
                }
                WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
                    let apps = theme::apps_use_dark_theme();
                    if apps != s.last_apps_dark.replace(apps) {
                        fire(&s.apps_dark, apps);
                    }
                    let taskbar = theme::taskbar_uses_dark_theme();
                    if taskbar != s.last_taskbar_dark.replace(taskbar) {
                        fire(&s.taskbar_dark, taskbar);
                    }
                    let animation = theme::client_area_animation();
                    if animation != s.last_animation.replace(animation) {
                        fire(&s.animation, animation);
                    }
                    None
                }
                _ => None,
            }
        });
        let window = match OwnedWindow::hidden("HayakuBarSystemEvents", "HayakuBarSystemEvents", handler) {
            Ok(w) => {
                if let Err(e) = unsafe { WTSRegisterSessionNotification(w.hwnd(), NOTIFY_FOR_THIS_SESSION) } {
                    log::error!("註冊工作階段通知失敗：{e}");
                }
                Some(w)
            }
            Err(e) => {
                log::error!("建立系統事件視窗失敗：{e}");
                None
            }
        };
        Self { window, shared }
    }

    /// 從睡眠／休眠喚醒（C# `PowerModes.Resume`）。
    pub fn on_resume(&self, f: impl Fn() + 'static) {
        self.shared.resume.borrow_mut().push(Rc::new(move |_| f()));
    }

    /// 工作階段鎖定／解鎖／登入（C# `SessionSwitch`）。
    pub fn on_session_switch(&self, f: impl Fn(SessionSwitchReason) + 'static) {
        self.shared.session.borrow_mut().push(Rc::new(f));
    }

    /// 應用程式深淺色改變（參數：是否深色）。
    pub fn on_apps_dark_changed(&self, f: impl Fn(bool) + 'static) {
        self.shared.apps_dark.borrow_mut().push(Rc::new(f));
    }

    /// 工作列（系統）深淺色改變（參數：是否深色）。
    pub fn on_taskbar_dark_changed(&self, f: impl Fn(bool) + 'static) {
        self.shared.taskbar_dark.borrow_mut().push(Rc::new(f));
    }

    /// 「顯示動畫」設定改變（參數：是否開啟動畫）。
    pub fn on_client_area_animation_changed(&self, f: impl Fn(bool) + 'static) {
        self.shared.animation.borrow_mut().push(Rc::new(f));
    }
}

impl Default for SystemEvents {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for SystemEvents {
    fn drop(&mut self) {
        if let Some(w) = &self.window
            && w.is_alive()
        {
            unsafe {
                let _ = WTSUnRegisterSessionNotification(w.hwnd());
            }
        }
    }
}
