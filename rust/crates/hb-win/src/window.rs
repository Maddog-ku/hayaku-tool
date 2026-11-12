//! 建立隱藏視窗／子視窗的共用工具：視窗程序轉給 Rust 閉包處理。
//! （C# 用 WinForms NativeWindow／WPF HwndSource，這裡自己註冊視窗類別。）

use std::cell::RefCell;
use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_DBLCLKS, CreateWindowExW, DefWindowProcW, DestroyWindow, GWLP_USERDATA, GetWindowLongPtrW,
    IDC_ARROW, IsWindow, LoadCursorW, RegisterClassExW, SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_NCCREATE,
    WM_NCDESTROY, WNDCLASSEXW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::PCWSTR;

/// 視窗訊息處理：回傳 `Some` 表示已處理（不再呼叫 DefWindowProc）。
pub(crate) type Handler = dyn Fn(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>;

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

thread_local! {
    static REGISTERED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn ensure_class(name: &str) -> Vec<u16> {
    let class = wide(name);
    let already = REGISTERED.with(|r| r.borrow().contains(name));
    if !already {
        unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let wc = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                style: CS_DBLCLKS,
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                lpszClassName: PCWSTR(class.as_ptr()),
                ..Default::default()
            };
            // 已註冊過（例如另一個執行緒）會失敗，不影響建立視窗
            RegisterClassExW(&wc);
        }
        REGISTERED.with(|r| r.borrow_mut().insert(name.to_string()));
    }
    class
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let cs = lparam.0 as *const CREATESTRUCTW;
            if !cs.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*cs).lpCreateParams as isize);
            }
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Rc<Handler>;
        if msg == WM_NCDESTROY {
            if !ptr.is_null() {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        if !ptr.is_null() {
            // 先複製一份 Rc：處理中視窗被摧毀（NCDESTROY 釋放）也不會讓正在執行的閉包失效
            let handler = (*ptr).clone();
            match std::panic::catch_unwind(AssertUnwindSafe(|| handler(hwnd, msg, wparam, lparam))) {
                Ok(Some(result)) => return result,
                Ok(None) => {}
                Err(_) => log::error!("視窗訊息處理失敗（msg=0x{msg:04X}）"),
            }
        }
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

/// 自己建立的視窗；Drop 時摧毀。
pub(crate) struct OwnedWindow {
    hwnd: HWND,
}

pub(crate) struct WindowSpec<'a> {
    pub class: &'a str,
    pub title: &'a str,
    pub style: WINDOW_STYLE,
    pub ex_style: WINDOW_EX_STYLE,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub parent: Option<HWND>,
}

impl OwnedWindow {
    pub fn create(spec: &WindowSpec, handler: Rc<Handler>) -> windows::core::Result<Self> {
        let class = ensure_class(spec.class);
        let title = wide(spec.title);
        let param = Box::into_raw(Box::new(handler));
        let result = unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            CreateWindowExW(
                spec.ex_style,
                PCWSTR(class.as_ptr()),
                PCWSTR(title.as_ptr()),
                spec.style,
                spec.x,
                spec.y,
                spec.width,
                spec.height,
                spec.parent,
                None,
                Some(instance.into()),
                Some(param as *const core::ffi::c_void),
            )
        };
        // 失敗時無法確定是否已走到 WM_NCDESTROY（已釋放），寧可漏掉一個小閉包也不重複釋放
        result.map(|hwnd| Self { hwnd })
    }

    /// 隱藏的頂層視窗（不是 message-only：要收得到 TaskbarCreated、WM_POWERBROADCAST、WM_SETTINGCHANGE 這類廣播）。
    pub fn hidden(class: &str, title: &str, handler: Rc<Handler>) -> windows::core::Result<Self> {
        Self::create(
            &WindowSpec {
                class,
                title,
                style: WS_POPUP,
                ex_style: WS_EX_TOOLWINDOW,
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                parent: None,
            },
            handler,
        )
    }

    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    pub fn is_alive(&self) -> bool {
        unsafe { IsWindow(Some(self.hwnd)).as_bool() }
    }
}

impl Drop for OwnedWindow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.hwnd)).as_bool() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
