//! 對應 C#：`src/HayakuBar/UI/TaskbarIconScanner.cs`
//!
//! 用 UI Automation 讀 Win11 工作列圖示的位置，讓工時避開圖示。
//! 在背景執行緒查詢：工時視窗是工作列的子視窗，UI 執行緒跨程序等 explorer 有卡住工作列的風險。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, TreeScope_Children, TreeScope_Descendants, UIA_ClassNamePropertyId,
};

/// 工作列上「開始＋搜尋＋app 圖示」那一整群的範圍（實體像素，螢幕座標）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IconRange {
    pub left: i32,
    pub right: i32,
}

#[derive(Default)]
pub struct TaskbarIconScanner {
    range: Arc<Mutex<Option<IconRange>>>,
    busy: Arc<AtomicBool>,
}

impl TaskbarIconScanner {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn range(&self) -> Option<IconRange> {
        self.range.lock().map(|r| *r).unwrap_or(None)
    }

    pub fn scan_in_background(&self) {
        if self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let range = self.range.clone();
        let busy = self.busy.clone();
        let spawned = std::thread::Builder::new().name("taskbar-icon-scan".into()).spawn(move || {
            match scan() {
                Ok(r) => {
                    if let Ok(mut slot) = range.lock() {
                        *slot = r;
                    }
                }
                Err(e) => log::error!("讀取工作列圖示位置失敗：{e}"),
            }
            busy.store(false, Ordering::Release);
        });
        if let Err(e) = spawned {
            log::error!("讀取工作列圖示位置失敗：{e}");
            self.busy.store(false, Ordering::Release);
        }
    }
}

fn scan() -> windows::core::Result<Option<IconRange>> {
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        let result = scan_com();
        if init.is_ok() {
            CoUninitialize();
        }
        result
    }
}

unsafe fn scan_com() -> windows::core::Result<Option<IconRange>> {
    unsafe {
        let automation: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER)?;
        let root = automation.GetRootElement()?;
        let tray_cond = automation.CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from("Shell_TrayWnd"))?;
        let Ok(tray) = root.FindFirst(TreeScope_Children, &tray_cond) else {
            return Ok(None);
        };
        let frame_cond = automation
            .CreatePropertyCondition(UIA_ClassNamePropertyId, &VARIANT::from("Taskbar.TaskbarFrameAutomationPeer"))?;
        let Ok(frame) = tray.FindFirst(TreeScope_Descendants, &frame_cond) else {
            return Ok(None); // 不是 Win11 的新工作列
        };

        let children = frame.FindAll(TreeScope_Children, &automation.CreateTrueCondition()?)?;
        let mut rects = Vec::new();
        for i in 0..children.Length()? {
            let Ok(child) = children.GetElement(i) else { continue };
            let Ok(r) = child.CurrentBoundingRectangle() else { continue };
            if r.right - r.left > 0 && r.bottom - r.top > 0 {
                rects.push((r.left as f64, r.right as f64));
            }
        }
        Ok(group(rects))
    }
}

/// 依間距分群，取最寬的一群（圖示群）；左邊的小工具（天氣）之類會被分開。
pub(crate) fn group(mut rects: Vec<(f64, f64)>) -> Option<IconRange> {
    if rects.is_empty() {
        return None;
    }
    rects.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut best = rects[0];
    let mut current = best;
    for &(left, right) in rects.iter().skip(1) {
        if left - current.1 <= 24.0 {
            current.1 = current.1.max(right);
        } else {
            current = (left, right);
        }
        if current.1 - current.0 > best.1 - best.0 {
            best = current;
        }
    }
    Some(IconRange { left: best.0.floor() as i32, right: best.1.ceil() as i32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 分群取最寬() {
        // 天氣小工具（0..120）與圖示群（600..640、650..690、700..740）
        let r = group(vec![(700.0, 740.0), (0.0, 120.0), (600.0, 640.0), (650.0, 690.0)]);
        assert_eq!(r, Some(IconRange { left: 600, right: 740 }));
        assert_eq!(group(vec![]), None);
        // 間距剛好 24 算同一群
        assert_eq!(group(vec![(0.0, 10.0), (34.0, 50.0)]), Some(IconRange { left: 0, right: 50 }));
        assert_eq!(group(vec![(0.0, 10.5), (35.0, 40.2)]), Some(IconRange { left: 0, right: 11 }));
    }
}
