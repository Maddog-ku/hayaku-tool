//! 對應 C# `UI/TaskText.cs`：清單上任務名稱的寫法（TaskText），以及點編號複製（KeyCopy）。

use hb_api::{TaskKind, TaskKinds, TaskSummary, TestInfo};
use hb_core::TimerEngine;

/// 清單上任務名稱的寫法：測試任務、測試結果前面加類型圖示，退回過的測試結果標第幾輪（第 3 輪起加 🔥）。
pub struct TaskText;

impl TaskText {
    pub fn title(title: &str, test: &TestInfo) -> String {
        let icon = TaskKinds::icon(test.kind);
        let mut text = if icon.is_empty() { title.to_owned() } else { format!("{icon} {title}") };
        if test.kind == TaskKind::TestResult && test.round >= 2 {
            if test.hot() {
                text += &format!("（第 {} 輪 🔥）", test.round);
            } else {
                text += &format!("（第 {} 輪）", test.round);
            }
        }
        text
    }

    /// C# `TaskText.Title(TaskSummary)`。
    pub fn title_of(t: &TaskSummary) -> String {
        Self::title(&t.title, &t.test)
    }
}

/// 點任務編號是複製編號（跟網頁一樣），點名稱才打開任務。
///
/// C# 用 Run／TextBlock 的 Tag="copykey" 判斷點到的是不是編號；Slint 這邊用 `@hbc` 的 `CopyKey` 元件
/// （滑過變手指、提示「點一下複製編號」），它的 `copy(key)` 接到 [`KeyCopy::copy`]。
pub struct KeyCopy;

impl KeyCopy {
    pub const TAG: &'static str = "copykey";
    /// 編號上的提示。
    pub const TIP: &'static str = "點一下複製編號";

    pub fn copy(engine: &TimerEngine, key: &str) {
        copy_text(engine, key, &format!("已複製 {key}"));
    }

    /// 複製任務在網頁上的連結（跟〔在網頁開啟〕同一個網址）。
    pub fn copy_link(engine: &TimerEngine, key: &str) {
        copy_text(engine, &engine.task_url(key), &format!("已複製 {key} 的連結"));
    }
}

fn copy_text(engine: &TimerEngine, text: &str, notice: &str) {
    match set_clipboard(text) {
        Ok(()) => engine.show_notice(notice),
        Err(err) => {
            log::info!("複製到剪貼簿失敗：{err}");
            engine.show_notice("複製失敗，剪貼簿被別的程式占用，再試一次");
        }
    }
}

/// 放進剪貼簿（C# `Clipboard.SetText`）。
pub fn set_clipboard(text: &str) -> Result<(), String> {
    arboard::Clipboard::new().and_then(|mut c| c.set_text(text.to_owned())).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test(kind: TaskKind, round: i32) -> TestInfo {
        TestInfo::new(kind, None, None, round, None)
    }

    // TestingTests.TaskText_測試結果加圖示與輪數
    #[test]
    fn task_text_測試結果加圖示與輪數() {
        assert_eq!("一般任務", TaskText::title("一般任務", &TestInfo::default()));
        assert_eq!("🧪 回歸測試", TaskText::title("回歸測試", &test(TaskKind::Test, 0)));
        assert_eq!("🐞 跑版", TaskText::title("跑版", &test(TaskKind::TestResult, 1)));
        assert_eq!("🐞 跑版（第 2 輪）", TaskText::title("跑版", &test(TaskKind::TestResult, 2)));
        assert_eq!("🐞 跑版（第 3 輪 🔥）", TaskText::title("跑版", &test(TaskKind::TestResult, 3)));
    }
}
