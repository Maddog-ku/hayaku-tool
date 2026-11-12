//! 命令列參數（對應 C# `App.OnStartup` 裡對 `e.Args` 的判斷）：
//! - `--mcp-call <工具> <參數 JSON> <輸出檔>`：診斷用，呼叫一支 MCP 工具後結束（一定要是第一個參數）。
//! - `--settings` 或 `--settings:頁`：啟動後直接開設定（頁：account、taskbar、schedule、repos、claude、eip、slack、general、about）。
//! - `--toast-test`：顯示一則範例提示；`--meeting-test`：模擬兩場快開始的會議。
//! - `--entries`：啟動後直接開工時紀錄。
//! - `--workspace` 或 `--workspace:頁`：啟動後直接開工作台（預設 tasks）。
//! - `--shot:頁,頁…`：開啟各頁後把畫面存成 PNG（預設 tasks）。
//! - `--panel` 或 `--panel:分頁`：啟動後直接打開面板（失去焦點也不收起）；分頁 tasks、today、notify，其他是計時。
//!
//! 開頭比對都不分大小寫（C# `StringComparison.OrdinalIgnoreCase`），冒號後面原樣保留。

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CliOptions {
    /// `--mcp-call` 是第一個參數。
    pub mcp_call: bool,
    /// `--settings`：Some(None) 開設定、Some(Some(頁)) 開到那一頁。
    pub settings: Option<Option<String>>,
    pub toast_test: bool,
    pub meeting_test: bool,
    pub entries: bool,
    /// `--workspace`：要開的頁（沒寫是 tasks）。
    pub workspace: Option<String>,
    /// `--shot`：要拍的頁，逗號分隔（沒寫是 tasks）。
    pub shot: Option<String>,
    /// `--panel`：Some(None) 只打開面板、Some(Some(分頁)) 先切到那個分頁（Timer、Tasks、Today、Notify）。
    pub panel: Option<Option<&'static str>>,
}

fn starts_with_ic(arg: &str, prefix: &str) -> bool {
    arg.len() >= prefix.len() && arg.is_char_boundary(prefix.len()) && arg[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// C# `arg[(arg.IndexOf(':') + 1)..]`：第一個冒號後面的部分；沒有冒號時 None。
fn after_colon(arg: &str) -> Option<String> {
    arg.find(':').map(|i| arg[i + 1..].to_owned())
}

/// `--panel:分頁` 的分頁名稱（C# 的 switch）。
pub fn panel_tab(name: &str) -> &'static str {
    match name {
        "tasks" => "Tasks",
        "today" => "Today",
        "notify" => "Notify",
        _ => "Timer",
    }
}

/// `--shot` 裡 panel 開頭的頁（panel、panel-today…）要切到哪個分頁：連字號後第一個字母大寫（C# 的寫法），沒有連字號是 Timer。
pub fn shot_panel_tab(page: &str) -> String {
    match page.find('-') {
        Some(i) => {
            let rest = &page[i + 1..];
            let mut chars = rest.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        }
        None => "Timer".to_owned(),
    }
}

/// `--shot` 的工作台頁：等號後面是要帶給那一頁的參數（tasks=F2P-97、meetings=new）。
pub fn split_page_arg(page: &str) -> (&str, Option<&str>) {
    match page.split_once('=') {
        Some((p, a)) => (p, Some(a)),
        None => (page, None),
    }
}

/// `--shot:a, b ,,c` → ["a", "b", "c"]（C# `Split(',', RemoveEmptyEntries | TrimEntries)`）。
pub fn shot_pages(pages: &str) -> Vec<String> {
    pages.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
}

pub fn parse(args: &[String]) -> CliOptions {
    let first = |prefix: &str| args.iter().find(|a| starts_with_ic(a, prefix));
    let any = |name: &str| args.iter().any(|a| a.eq_ignore_ascii_case(name));
    CliOptions {
        mcp_call: args.first().is_some_and(|a| a == "--mcp-call"),
        settings: first("--settings").map(|a| after_colon(a)),
        toast_test: any("--toast-test"),
        meeting_test: any("--meeting-test"),
        entries: any("--entries"),
        workspace: first("--workspace").map(|a| after_colon(a).unwrap_or_else(|| "tasks".to_owned())),
        shot: first("--shot").map(|a| after_colon(a).unwrap_or_else(|| "tasks".to_owned())),
        panel: first("--panel").map(|a| after_colon(a).map(|t| panel_tab(&t))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn 沒有參數() {
        assert_eq!(CliOptions::default(), parse(&[]));
    }

    #[test]
    fn 設定頁() {
        assert_eq!(Some(None), parse(&args(&["--settings"])).settings);
        assert_eq!(Some(Some("claude".to_owned())), parse(&args(&["--SETTINGS:claude"])).settings);
        // 第一個符合的才算
        assert_eq!(Some(Some("eip".to_owned())), parse(&args(&["--settings:eip", "--settings:slack"])).settings);
    }

    #[test]
    fn 工作台與截圖預設_tasks() {
        let o = parse(&args(&["--workspace", "--shot"]));
        assert_eq!(Some("tasks".to_owned()), o.workspace);
        assert_eq!(Some("tasks".to_owned()), o.shot);
        let o = parse(&args(&["--workspace:hours", "--shot:panel-today,settings-general"]));
        assert_eq!(Some("hours".to_owned()), o.workspace);
        assert_eq!(Some("panel-today,settings-general".to_owned()), o.shot);
    }

    #[test]
    fn 面板分頁() {
        assert_eq!(Some(None), parse(&args(&["--panel"])).panel);
        assert_eq!(Some(Some("Tasks")), parse(&args(&["--panel:tasks"])).panel);
        assert_eq!(Some(Some("Today")), parse(&args(&["--panel:today"])).panel);
        assert_eq!(Some(Some("Notify")), parse(&args(&["--panel:notify"])).panel);
        assert_eq!(Some(Some("Timer")), parse(&args(&["--panel:xyz"])).panel);
        // 分頁名稱分大小寫（C# switch）
        assert_eq!(Some(Some("Timer")), parse(&args(&["--panel:Today"])).panel);
    }

    #[test]
    fn 開關型參數不分大小寫() {
        let o = parse(&args(&["--Toast-Test", "--MEETING-test", "--entries"]));
        assert!(o.toast_test && o.meeting_test && o.entries);
        // 要完全相同（不是開頭比對）
        assert!(!parse(&args(&["--toast-tests"])).toast_test);
    }

    #[test]
    fn mcp_call_要是第一個參數() {
        assert!(parse(&args(&["--mcp-call", "whoami", "{}", "out.txt"])).mcp_call);
        assert!(!parse(&args(&["--panel", "--mcp-call"])).mcp_call);
    }

    #[test]
    fn 截圖頁拆解() {
        assert_eq!(vec!["a", "b", "c"], shot_pages("a, b ,,c"));
        assert_eq!("Timer", shot_panel_tab("panel"));
        assert_eq!("Today", shot_panel_tab("panel-today"));
        assert_eq!("Notify", shot_panel_tab("panel-notify"));
        assert_eq!(("tasks", Some("F2P-97")), split_page_arg("tasks=F2P-97"));
        assert_eq!(("meetings", Some("a=b")), split_page_arg("meetings=a=b"));
        assert_eq!(("hours", None), split_page_arg("hours"));
    }
}
