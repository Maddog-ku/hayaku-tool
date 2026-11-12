//! 共用 Slint 主題與元件（ui/theme.slint、ui/components/*.slint、ui/fonts/）。
//!
//! 這個 crate 不編譯任何 .slint 給別人用；其他 crate 在自己的 build.rs 把 library path `hb`
//! 指到 `crates/hb-ui-kit/ui`，再 `import { Theme, Card, ... } from "@hb/index.slint";`。
//! 執行期另外提供：中文字型退回設定（[`init_fonts`]）。

use std::path::PathBuf;

/// `ui/` 資料夾（library path `hb` 要指向這裡）。
pub fn ui_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// 讓中文退回 Microsoft JhengHei。
///
/// 介面字型是 Inter（嵌入），Inter 沒有中文字；Slint 預設的系統退回字型在 Windows 上可能挑到簡體字形的字型。
/// 這裡把 `SLINT_FONT_PATH` 設成正黑體（msjh.ttc／msjhbd.ttc）與 Segoe UI Symbol（seguisym.ttf，補 ✦、⤒ 等符號），Slint 會把它們排進 sans-serif 退回鏈。
/// 必須在建立任何 Slint 視窗**之前**、且還沒有其他執行緒時呼叫（會改環境變數）。
/// 使用者自己設了 `SLINT_FONT_PATH` 時不覆蓋；找不到字型檔時什麼都不做。
pub fn init_fonts() {
    if std::env::var_os("SLINT_FONT_PATH").is_some() {
        return;
    }
    let windir = std::env::var_os("WINDIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let fonts = windir.join("Fonts");
    let files: Vec<PathBuf> =
        ["msjh.ttc", "msjhbd.ttc", "seguisym.ttf"].iter().map(|f| fonts.join(f)).filter(|p| p.is_file()).collect();
    if files.is_empty() {
        return;
    }
    let Ok(joined) = std::env::join_paths(files) else { return };
    // SAFETY: 文件要求在建立視窗與其他執行緒之前呼叫，此時沒有其他執行緒讀寫環境變數。
    unsafe { std::env::set_var("SLINT_FONT_PATH", joined) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_dir_has_index() {
        assert!(ui_dir().join("index.slint").is_file());
        assert!(ui_dir().join("theme.slint").is_file());
    }
}
