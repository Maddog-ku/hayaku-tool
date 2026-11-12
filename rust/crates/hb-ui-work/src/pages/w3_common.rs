//! 會議、聊天、筆記、團隊空間、論壇、團隊狀況、專案動態、請假頁共用的小工具
//! （對應 C# `UI/Work/PageParts.cs` 的純邏輯、`MarkdownTable.Attach` 攔貼上的剪貼簿判斷、`Inputs.Show`／`Inputs.Run` 的接線）。
//!
//! `src/pages/mod.rs` 不能改，所以由 `meetings.rs` 用 `#[path]` 引入，其他頁用 `super::meetings::w3_common`。

use std::future::Future;

use hb_ui_common::MarkdownTable;
use hb_ui_common::inputs::StatusMessage;
use slint::{ModelRc, SharedString, VecModel};

/// 在 UI 執行緒跑一個 future（C# `async void` 事件處理）；排不進去時寫紀錄。
pub(crate) fn spawn(fut: impl Future<Output = ()> + 'static) {
    if let Err(e) = slint::spawn_local(fut) {
        log::error!("工作台：無法在 UI 執行緒執行: {e}");
    }
}

/// Vec → Slint 模型。
pub(crate) fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// 字串清單 → Slint 模型。
pub(crate) fn strings<S: AsRef<str>>(items: impl IntoIterator<Item = S>) -> ModelRc<SharedString> {
    model(items.into_iter().map(|s| SharedString::from(s.as_ref())).collect())
}

/// `Inputs.Show` 的文字與色調（StatusLine 的 tone：0 錯誤、1 成功）。None＝收起來。
pub(crate) fn status_parts(m: Option<&StatusMessage>) -> (SharedString, i32) {
    match m {
        Some(m) => (m.text.as_str().into(), if m.error { 0 } else { 1 }),
        None => (SharedString::new(), 0),
    }
}

/// C# `PageParts.Size`：位元組 → 「12 B」「3.4 KB」「1.2 MB」；不知道大小回空字串。
pub(crate) fn size_text(bytes: Option<i64>) -> String {
    match bytes {
        None => String::new(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{} KB", one_decimal(b as f64 / 1024.0)),
        Some(b) => format!("{} MB", one_decimal(b as f64 / 1024.0 / 1024.0)),
    }
}

/// C# `Uri.EscapeDataString`：除了 RFC 3986 的 unreserved（英數、`-_.~`）都編成 %XX（UTF-8）。
pub(crate) fn escape_data(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// .NET 的 `0.#`：最多一位小數、去掉 .0（四捨五入遠離零，跟 .NET 自訂格式一樣）。
pub(crate) fn one_decimal(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    if r.fract() == 0.0 { format!("{}", r as i64) } else { format!("{r:.1}") }
}

/// 〔▦ 表格〕與貼上：kind 0 是空白表格（C# `Template()`：3 欄 2 列），1 是剪貼簿裡試算表的儲存格轉成的表格（不是試算表回空字串）。
pub(crate) fn md_table(kind: i32) -> SharedString {
    if kind == 0 {
        return MarkdownTable::template(3, 2).into();
    }
    sheet_table().unwrap_or_default().into()
}

/// C# `MarkdownTable.Insert`：插入後的整段文字。
pub(crate) fn md_insert(text: &str, a: i32, b: i32, table: &str) -> SharedString {
    MarkdownTable::insert(text, range(a, b), table).0.into()
}

/// C# `MarkdownTable.Insert`：插入後的游標位置。
pub(crate) fn md_cursor(text: &str, a: i32, b: i32, table: &str) -> i32 {
    MarkdownTable::insert(text, range(a, b), table).1 as i32
}

fn range(a: i32, b: i32) -> std::ops::Range<usize> {
    let a = a.max(0) as usize;
    let b = b.max(0) as usize;
    a.min(b)..a.max(b)
}

/// 把 MdArea 的三個 callback 接到頁面 global（`md-table`、`md-insert`、`md-cursor`）。
macro_rules! wire_md {
    ($g:expr) => {{
        let g = $g;
        g.on_md_table(|kind| $crate::pages::meetings::w3_common::md_table(kind));
        g.on_md_insert(|text: slint::SharedString, a: i32, b: i32, table: slint::SharedString| {
            $crate::pages::meetings::w3_common::md_insert(&text, a, b, &table)
        });
        g.on_md_cursor(|text: slint::SharedString, a: i32, b: i32, table: slint::SharedString| {
            $crate::pages::meetings::w3_common::md_cursor(&text, a, b, &table)
        });
    }};
}
pub(crate) use wire_md;

/// C# `MarkdownTable.Attach`：只認試算表放進剪貼簿的格式（XML Spreadsheet、CSV、含 `<table` 的 HTML），
/// 一般有 Tab 縮排的文字（例如程式碼）照原樣貼。是試算表而且轉得成表格時回 Markdown 表格。
pub(crate) fn sheet_table() -> Option<String> {
    if !clipboard_from_sheet() {
        return None;
    }
    let text = match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
        Ok(t) => t,
        Err(e) => {
            log::debug!("讀剪貼簿文字失敗：{e}");
            return None;
        }
    };
    MarkdownTable::from_tsv(Some(&text))
}

#[cfg(windows)]
fn clipboard_from_sheet() -> bool {
    use windows::Win32::System::DataExchange::{IsClipboardFormatAvailable, RegisterClipboardFormatW};
    use windows::core::w;
    // SAFETY：只查剪貼簿有沒有某個格式，不開啟剪貼簿、不讀資料。
    unsafe {
        let available = |name: windows::core::PCWSTR| {
            let id = RegisterClipboardFormatW(name);
            id != 0 && IsClipboardFormatAvailable(id).is_ok()
        };
        if available(w!("XML Spreadsheet")) || available(w!("CSV")) {
            return true;
        }
        if !available(w!("HTML Format")) {
            return false;
        }
    }
    clipboard_html().is_some_and(|html| html.to_ascii_lowercase().contains("<table"))
}

#[cfg(not(windows))]
fn clipboard_from_sheet() -> bool {
    false
}

/// 剪貼簿的 HTML（CF_HTML，UTF-8）。
#[cfg(windows)]
fn clipboard_html() -> Option<String> {
    arboard::Clipboard::new().ok()?.get().html().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_text_跟_page_parts_一樣() {
        assert_eq!("", size_text(None));
        assert_eq!("0 B", size_text(Some(0)));
        assert_eq!("1023 B", size_text(Some(1023)));
        assert_eq!("1 KB", size_text(Some(1024)));
        assert_eq!("1.5 KB", size_text(Some(1536)));
        assert_eq!("1.4 MB", size_text(Some(1_468_006)));
        assert_eq!("2 MB", size_text(Some(2 * 1024 * 1024)));
    }

    #[test]
    fn escape_data_跟_uri_一樣() {
        assert_eq!("F2P", escape_data("F2P"));
        assert_eq!("a%20b%2Fc", escape_data("a b/c"));
        assert_eq!("%E5%B7%A5", escape_data("工"));
    }

    #[test]
    fn md_insert_前後補空行() {
        let table = "| a |";
        assert_eq!("| a |\n\n", md_insert("", 0, 0, table).as_str());
        assert_eq!("文字\n\n| a |\n\n", md_insert("文字", 6, 6, table).as_str());
        assert_eq!(6 + 2 + 5 + 2, md_cursor("文字", 6, 6, table));
        // 選取範圍會被取代；反向選取也一樣
        assert_eq!("| a |\n\n", md_insert("xyz", 3, 0, table).as_str());
    }

    #[test]
    fn md_table_空白表格是三欄兩列() {
        let t = md_table(0);
        assert_eq!(4, t.lines().count());
        assert!(t.starts_with("| 欄位 1 | 欄位 2 | 欄位 3 |"));
    }
}
