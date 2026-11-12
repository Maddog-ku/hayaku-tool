//! 對應 C#：`src/HayakuBar/Infra/GlobalHotkey.cs`
//!
//! 全域快捷鍵（RegisterHotKey），格式例「Win+Shift+H」「Ctrl+Alt+F9」。
//! C# 的按鍵名稱來自 WPF `Key` 列舉（`Enum.TryParse` 不分大小寫，也接受數字）；這裡內建同一張名稱表，
//! 並照 `KeyInterop.VirtualKeyFromKey` 換成虛擬鍵碼。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey};
use windows::Win32::UI::WindowsAndMessaging::WM_HOTKEY;

use crate::window::OwnedWindow;

const HOTKEY_ID: i32 = 0x4859;

type Pressed = Rc<RefCell<Vec<Rc<dyn Fn()>>>>;
pub const MOD_ALT: u32 = 0x1;
pub const MOD_CONTROL: u32 = 0x2;
pub const MOD_SHIFT: u32 = 0x4;
pub const MOD_WIN: u32 = 0x8;
const MOD_NOREPEAT: u32 = 0x4000;

/// 對應 WPF `ModifierKeys`（Alt=1、Control=2、Shift=4、Windows=8）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModifierKeys(pub u32);

impl ModifierKeys {
    pub const NONE: Self = Self(0);
    pub const ALT: Self = Self(1);
    pub const CONTROL: Self = Self(2);
    pub const SHIFT: Self = Self(4);
    pub const WINDOWS: Self = Self(8);

    pub fn has(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0 && flag.0 != 0
    }
}

impl std::ops::BitOr for ModifierKeys {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// WPF `Key` 名稱、列舉值、虛擬鍵碼（0 表示沒有對應）。同一個值有多個名稱時，第一個是 `format` 用的名稱。
const KEYS: &[(&str, i32, u32)] = &[
    ("None", 0, 0),
    ("Cancel", 1, 0x03),
    ("Back", 2, 0x08),
    ("Tab", 3, 0x09),
    ("LineFeed", 4, 0),
    ("Clear", 5, 0x0C),
    ("Enter", 6, 0x0D),
    ("Return", 6, 0x0D),
    ("Pause", 7, 0x13),
    ("CapsLock", 8, 0x14),
    ("Capital", 8, 0x14),
    ("KanaMode", 9, 0x15),
    ("HangulMode", 9, 0x15),
    ("JunjaMode", 10, 0x17),
    ("FinalMode", 11, 0x18),
    ("HanjaMode", 12, 0x19),
    ("KanjiMode", 12, 0x19),
    ("Escape", 13, 0x1B),
    ("ImeConvert", 14, 0x1C),
    ("ImeNonConvert", 15, 0x1D),
    ("ImeAccept", 16, 0x1E),
    ("ImeModeChange", 17, 0x1F),
    ("Space", 18, 0x20),
    ("PageUp", 19, 0x21),
    ("Prior", 19, 0x21),
    ("PageDown", 20, 0x22),
    ("Next", 20, 0x22),
    ("End", 21, 0x23),
    ("Home", 22, 0x24),
    ("Left", 23, 0x25),
    ("Up", 24, 0x26),
    ("Right", 25, 0x27),
    ("Down", 26, 0x28),
    ("Select", 27, 0x29),
    ("Print", 28, 0x2A),
    ("Execute", 29, 0x2B),
    ("PrintScreen", 30, 0x2C),
    ("Snapshot", 30, 0x2C),
    ("Insert", 31, 0x2D),
    ("Delete", 32, 0x2E),
    ("Help", 33, 0x2F),
    ("D0", 34, 0x30),
    ("D1", 35, 0x31),
    ("D2", 36, 0x32),
    ("D3", 37, 0x33),
    ("D4", 38, 0x34),
    ("D5", 39, 0x35),
    ("D6", 40, 0x36),
    ("D7", 41, 0x37),
    ("D8", 42, 0x38),
    ("D9", 43, 0x39),
    ("A", 44, 0x41),
    ("B", 45, 0x42),
    ("C", 46, 0x43),
    ("D", 47, 0x44),
    ("E", 48, 0x45),
    ("F", 49, 0x46),
    ("G", 50, 0x47),
    ("H", 51, 0x48),
    ("I", 52, 0x49),
    ("J", 53, 0x4A),
    ("K", 54, 0x4B),
    ("L", 55, 0x4C),
    ("M", 56, 0x4D),
    ("N", 57, 0x4E),
    ("O", 58, 0x4F),
    ("P", 59, 0x50),
    ("Q", 60, 0x51),
    ("R", 61, 0x52),
    ("S", 62, 0x53),
    ("T", 63, 0x54),
    ("U", 64, 0x55),
    ("V", 65, 0x56),
    ("W", 66, 0x57),
    ("X", 67, 0x58),
    ("Y", 68, 0x59),
    ("Z", 69, 0x5A),
    ("LWin", 70, 0x5B),
    ("RWin", 71, 0x5C),
    ("Apps", 72, 0x5D),
    ("Sleep", 73, 0x5F),
    ("NumPad0", 74, 0x60),
    ("NumPad1", 75, 0x61),
    ("NumPad2", 76, 0x62),
    ("NumPad3", 77, 0x63),
    ("NumPad4", 78, 0x64),
    ("NumPad5", 79, 0x65),
    ("NumPad6", 80, 0x66),
    ("NumPad7", 81, 0x67),
    ("NumPad8", 82, 0x68),
    ("NumPad9", 83, 0x69),
    ("Multiply", 84, 0x6A),
    ("Add", 85, 0x6B),
    ("Separator", 86, 0x6C),
    ("Subtract", 87, 0x6D),
    ("Decimal", 88, 0x6E),
    ("Divide", 89, 0x6F),
    ("F1", 90, 0x70),
    ("F2", 91, 0x71),
    ("F3", 92, 0x72),
    ("F4", 93, 0x73),
    ("F5", 94, 0x74),
    ("F6", 95, 0x75),
    ("F7", 96, 0x76),
    ("F8", 97, 0x77),
    ("F9", 98, 0x78),
    ("F10", 99, 0x79),
    ("F11", 100, 0x7A),
    ("F12", 101, 0x7B),
    ("F13", 102, 0x7C),
    ("F14", 103, 0x7D),
    ("F15", 104, 0x7E),
    ("F16", 105, 0x7F),
    ("F17", 106, 0x80),
    ("F18", 107, 0x81),
    ("F19", 108, 0x82),
    ("F20", 109, 0x83),
    ("F21", 110, 0x84),
    ("F22", 111, 0x85),
    ("F23", 112, 0x86),
    ("F24", 113, 0x87),
    ("NumLock", 114, 0x90),
    ("Scroll", 115, 0x91),
    ("LeftShift", 116, 0xA0),
    ("RightShift", 117, 0xA1),
    ("LeftCtrl", 118, 0xA2),
    ("RightCtrl", 119, 0xA3),
    ("LeftAlt", 120, 0xA4),
    ("RightAlt", 121, 0xA5),
    ("BrowserBack", 122, 0xA6),
    ("BrowserForward", 123, 0xA7),
    ("BrowserRefresh", 124, 0xA8),
    ("BrowserStop", 125, 0xA9),
    ("BrowserSearch", 126, 0xAA),
    ("BrowserFavorites", 127, 0xAB),
    ("BrowserHome", 128, 0xAC),
    ("VolumeMute", 129, 0xAD),
    ("VolumeDown", 130, 0xAE),
    ("VolumeUp", 131, 0xAF),
    ("MediaNextTrack", 132, 0xB0),
    ("MediaPreviousTrack", 133, 0xB1),
    ("MediaStop", 134, 0xB2),
    ("MediaPlayPause", 135, 0xB3),
    ("LaunchMail", 136, 0xB4),
    ("SelectMedia", 137, 0xB5),
    ("LaunchApplication1", 138, 0xB6),
    ("LaunchApplication2", 139, 0xB7),
    ("OemSemicolon", 140, 0xBA),
    ("Oem1", 140, 0xBA),
    ("OemPlus", 141, 0xBB),
    ("OemComma", 142, 0xBC),
    ("OemMinus", 143, 0xBD),
    ("OemPeriod", 144, 0xBE),
    ("OemQuestion", 145, 0xBF),
    ("Oem2", 145, 0xBF),
    ("OemTilde", 146, 0xC0),
    ("Oem3", 146, 0xC0),
    ("AbntC1", 147, 0xC1),
    ("AbntC2", 148, 0xC2),
    ("OemOpenBrackets", 149, 0xDB),
    ("Oem4", 149, 0xDB),
    ("OemPipe", 150, 0xDC),
    ("Oem5", 150, 0xDC),
    ("OemCloseBrackets", 151, 0xDD),
    ("Oem6", 151, 0xDD),
    ("OemQuotes", 152, 0xDE),
    ("Oem7", 152, 0xDE),
    ("Oem8", 153, 0xDF),
    ("OemBackslash", 154, 0xE2),
    ("Oem102", 154, 0xE2),
    ("ImeProcessed", 155, 0xE5),
    ("System", 156, 0x12),
    ("OemAttn", 157, 0xF0),
    ("DbeAlphanumeric", 157, 0xF0),
    ("OemFinish", 158, 0xF1),
    ("DbeKatakana", 158, 0xF1),
    ("OemCopy", 159, 0xF2),
    ("DbeHiragana", 159, 0xF2),
    ("OemAuto", 160, 0xF3),
    ("DbeSbcsChar", 160, 0xF3),
    ("OemEnlw", 161, 0xF4),
    ("DbeDbcsChar", 161, 0xF4),
    ("OemBackTab", 162, 0xF5),
    ("DbeRoman", 162, 0xF5),
    ("Attn", 163, 0xF6),
    ("DbeNoRoman", 163, 0xF6),
    ("CrSel", 164, 0xF7),
    ("DbeEnterWordRegisterMode", 164, 0xF7),
    ("ExSel", 165, 0xF8),
    ("DbeEnterImeConfigureMode", 165, 0xF8),
    ("EraseEof", 166, 0xF9),
    ("DbeFlushString", 166, 0xF9),
    ("Play", 167, 0xFA),
    ("DbeCodeInput", 167, 0xFA),
    ("Zoom", 168, 0xFB),
    ("DbeNoCodeInput", 168, 0xFB),
    ("NoName", 169, 0xFC),
    ("DbeDetermineString", 169, 0xFC),
    ("Pa1", 170, 0xFD),
    ("DbeEnterDialogConversionMode", 170, 0xFD),
    ("OemClear", 171, 0xFE),
    ("DeadCharProcessed", 172, 0),
];

/// 按鍵名稱（不分大小寫）或 WPF 列舉數值 → 虛擬鍵碼；認不得回傳 None，認得但沒有鍵碼回傳 Some(0)。
fn parse_key(raw: &str) -> Option<u32> {
    if let Some((_, _, vk)) = KEYS.iter().find(|(name, _, _)| name.eq_ignore_ascii_case(raw)) {
        return Some(*vk);
    }
    // Enum.TryParse 也接受數字（未定義的值也算成功，但換不出鍵碼）
    let n: i32 = raw.parse().ok()?;
    Some(KEYS.iter().find(|(_, v, _)| *v == n).map_or(0, |(_, _, vk)| *vk))
}

/// 解析快捷鍵文字；回傳（修飾鍵 MOD_*、虛擬鍵碼）。沒有主鍵、沒有修飾鍵、按鍵名稱不對時回傳 None。
pub fn try_parse(text: &str) -> Option<(u32, u32)> {
    let mut modifiers = 0;
    let mut key: Option<u32> = None;
    for raw in text.split('+').map(str::trim).filter(|s| !s.is_empty()) {
        match raw.to_lowercase().as_str() {
            "win" => modifiers |= MOD_WIN,
            "ctrl" | "control" => modifiers |= MOD_CONTROL,
            "shift" => modifiers |= MOD_SHIFT,
            "alt" => modifiers |= MOD_ALT,
            _ => {
                let mut chars = raw.chars();
                let name = match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii_digit() => format!("D{raw}"),
                    _ => raw.to_string(),
                };
                key = Some(parse_key(&name)?);
            }
        }
    }
    let vk = key?;
    if modifiers == 0 || vk == 0 {
        return None;
    }
    Some((modifiers, vk))
}

/// 虛擬鍵碼的顯示名稱（WPF `Key.ToString()`）；D0～D9 顯示成數字由 `format` 處理。
pub fn key_name(vk: u32) -> Option<&'static str> {
    KEYS.iter().find(|(_, _, v)| *v == vk && *v != 0).map(|(name, _, _)| *name)
}

/// 把按下的組合鍵轉成文字；只有修飾鍵、或沒有修飾鍵時回傳 None。
/// C# 收 WPF `Key`；這裡收虛擬鍵碼（vk），沒有名稱的鍵碼也回傳 None。
pub fn format(modifiers: ModifierKeys, win: bool, vk: u32) -> Option<String> {
    // 修飾鍵本身（Ctrl、Shift、Alt、Win，含左右）
    if matches!(vk, 0x10 | 0x11 | 0x12 | 0xA0..=0xA5 | 0x5B | 0x5C) {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    if win {
        parts.push("Win".into());
    }
    if modifiers.has(ModifierKeys::CONTROL) {
        parts.push("Ctrl".into());
    }
    if modifiers.has(ModifierKeys::ALT) {
        parts.push("Alt".into());
    }
    if modifiers.has(ModifierKeys::SHIFT) {
        parts.push("Shift".into());
    }
    if parts.is_empty() {
        return None;
    }
    let name = if (0x30..=0x39).contains(&vk) { (vk - 0x30).to_string() } else { key_name(vk)?.to_string() };
    parts.push(name);
    Some(parts.join("+"))
}

/// 全域快捷鍵。在 UI 執行緒建立；按下時呼叫 `on_pressed` 註冊的回呼。
pub struct GlobalHotkey {
    window: Option<OwnedWindow>,
    registered: Cell<bool>,
    pressed: Pressed,
}

impl GlobalHotkey {
    pub fn new() -> Self {
        let pressed: Pressed = Rc::default();
        let list = pressed.clone();
        let handler = Rc::new(move |_hwnd: HWND, msg: u32, wparam: WPARAM, _lparam: LPARAM| -> Option<LRESULT> {
            if msg == WM_HOTKEY && wparam.0 as i32 == HOTKEY_ID {
                let callbacks: Vec<_> = list.borrow().clone();
                for cb in callbacks {
                    cb();
                }
                return Some(LRESULT(0));
            }
            None
        });
        let window = match OwnedWindow::hidden("HayakuBarHotkey", "HayakuBarHotkey", handler) {
            Ok(w) => Some(w),
            Err(e) => {
                log::error!("建立快捷鍵視窗失敗：{e}");
                None
            }
        };
        Self { window, registered: Cell::new(false), pressed }
    }

    pub fn on_pressed(&self, f: impl Fn() + 'static) {
        self.pressed.borrow_mut().push(Rc::new(f));
    }

    /// 註冊快捷鍵；空字串表示取消。被其他程式占用或格式不對時回傳 false。
    pub fn register(&self, text: Option<&str>) -> bool {
        self.unregister();
        let Some(text) = text.filter(|t| !t.trim().is_empty()) else {
            return true;
        };
        let Some((modifiers, vk)) = try_parse(text) else {
            return false;
        };
        let Some(window) = &self.window else {
            return false;
        };
        let ok = unsafe {
            RegisterHotKey(Some(window.hwnd()), HOTKEY_ID, HOT_KEY_MODIFIERS(modifiers | MOD_NOREPEAT), vk).is_ok()
        };
        self.registered.set(ok);
        if !ok {
            log::error!("快捷鍵 {text} 註冊失敗（可能被其他程式占用）");
        }
        ok
    }

    fn unregister(&self) {
        if self.registered.get()
            && let Some(window) = &self.window
        {
            unsafe {
                let _ = UnregisterHotKey(Some(window.hwnd()), HOTKEY_ID);
            }
        }
        self.registered.set(false);
    }
}

impl Default for GlobalHotkey {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for GlobalHotkey {
    fn drop(&mut self) {
        self.unregister();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 移植自 tests/HayakuBar.Tests/ExtrasTests.cs：Hotkey_解析
    #[test]
    fn hotkey_解析() {
        for (text, ok) in [
            ("Win+Shift+H", true),
            ("Ctrl+Alt+F9", true),
            ("Ctrl+1", true),
            ("H", false),          // 沒有修飾鍵
            ("Win+Shift+", false), // 沒有主鍵
            ("Ctrl+NotAKey", false),
        ] {
            assert_eq!(try_parse(text).is_some(), ok, "{text}");
        }
    }

    // 移植自 tests/HayakuBar.Tests/ExtrasTests.cs：Hotkey_格式化（WPF Key 換成虛擬鍵碼：H=0x48、D1=0x31、LeftShift=0xA0）
    #[test]
    fn hotkey_格式化() {
        assert_eq!(format(ModifierKeys::SHIFT, true, 0x48).as_deref(), Some("Win+Shift+H"));
        assert_eq!(format(ModifierKeys::CONTROL | ModifierKeys::ALT, false, 0x31).as_deref(), Some("Ctrl+Alt+1"));
        assert_eq!(format(ModifierKeys::SHIFT, false, 0xA0), None); // 只有修飾鍵
        assert_eq!(format(ModifierKeys::NONE, false, 0x48), None); // 沒有修飾鍵
    }

    #[test]
    fn hotkey_解析值() {
        assert_eq!(try_parse("Win+Shift+H"), Some((MOD_WIN | MOD_SHIFT, 0x48)));
        assert_eq!(try_parse("ctrl + alt + f9"), Some((MOD_CONTROL | MOD_ALT, 0x78)));
        assert_eq!(try_parse("Control+1"), Some((MOD_CONTROL, 0x31)));
        assert_eq!(try_parse("Ctrl+Return"), Some((MOD_CONTROL, 0x0D)));
        assert_eq!(try_parse("Ctrl+OemTilde"), Some((MOD_CONTROL, 0xC0)));
        // 數字：WPF 列舉值（12 = HanjaMode）
        assert_eq!(try_parse("Ctrl+12"), Some((MOD_CONTROL, 0x19)));
        assert_eq!(try_parse("Ctrl+999"), None);
        assert_eq!(try_parse("Ctrl+None"), None);
        assert_eq!(try_parse(""), None);
    }

    #[test]
    fn hotkey_格式化後可解析回來() {
        for vk in [0x41u32, 0x30, 0x39, 0x70, 0x87, 0x0D, 0x20, 0xC0, 0x21] {
            let text = format(ModifierKeys::CONTROL | ModifierKeys::SHIFT, true, vk).unwrap_or_default();
            assert_eq!(try_parse(&text), Some((MOD_WIN | MOD_CONTROL | MOD_SHIFT, vk)), "{text}");
        }
    }
}
