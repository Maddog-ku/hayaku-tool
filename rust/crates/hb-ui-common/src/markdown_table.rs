//! 對應 C# `UI/Work/MarkdownTable.cs` 的純文字部分：文件、會議紀錄、空間頁面、筆記本、論壇貼文的表格。
//! 送出的是 Markdown，Hayaku 會變成表格。從 Excel 複製（Tab 分隔）貼上時自動轉成 Markdown 表格；〔▦ 表格〕插入一個空白表格。
//!
//! C# 的 `Attach`（攔貼上）、`InsertButton` 是 WPF 控制項；Slint 端在貼上／按鈕時呼叫 [`MarkdownTable::from_tsv`]、
//! [`MarkdownTable::insert`] 算出新文字與游標位置。

pub struct MarkdownTable;

/// 〔▦ 表格〕按鈕的文字與提示。
pub const INSERT_BUTTON_TEXT: &str = "▦ 表格";
pub const INSERT_BUTTON_TIP: &str = "插入表格（Markdown）；從 Excel 複製貼上也會自動變成表格";

impl MarkdownTable {
    /// Excel 複製出來的文字（Tab 分隔、儲存格有換行時用雙引號包起來）→ Markdown 表格；不像表格回 None。
    pub fn from_tsv(text: Option<&str>) -> Option<String> {
        let text = text.filter(|t| !t.is_empty() && t.contains('\t'))?;
        let mut rows = parse(text);
        while rows.last().is_some_and(|r| r.iter().all(String::is_empty)) {
            rows.pop();
        }
        if rows.is_empty() || rows.iter().all(|r| r.len() < 2) {
            return None;
        }
        let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut sb = String::new();
        let line = |sb: &mut String, cells: &[String]| {
            sb.push('|');
            for i in 0..columns {
                sb.push(' ');
                sb.push_str(&cell(cells.get(i).map_or("", String::as_str)));
                sb.push_str(" |");
            }
            sb.push('\n');
        };
        line(&mut sb, &rows[0]);
        sb.push('|');
        sb.push_str(&" --- |".repeat(columns));
        sb.push('\n');
        for row in rows.iter().skip(1) {
            line(&mut sb, row);
        }
        Some(sb.trim_end_matches('\n').to_owned())
    }

    /// 空白表格：表頭一列＋rows 列，cols 欄（C# 預設 3 欄 2 列）。
    pub fn template(cols: usize, rows: usize) -> String {
        let header = format!("|{}", (1..=cols).map(|i| format!(" 欄位 {i} |")).collect::<String>());
        let line = format!("|{}", " --- |".repeat(cols));
        let body = vec![format!("|{}", "  |".repeat(cols)); rows].join("\n");
        format!("{header}\n{line}\n{body}")
    }

    /// 在游標的位置插入（前後補空行）：text 是輸入框原本的內容，selection 是選取範圍（位元組位移，start..end）。
    /// 回傳新的內容與插入後的游標位置（位元組位移）。
    pub fn insert(text: &str, selection: std::ops::Range<usize>, table: &str) -> (String, usize) {
        let start = floor_boundary(text, selection.start.min(text.len()));
        let end = floor_boundary(text, selection.end.clamp(start, text.len()));
        let before = &text[..start];
        let prefix = if before.is_empty() || before.ends_with("\n\n") {
            ""
        } else if before.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };
        let inserted = format!("{prefix}{table}\n\n");
        let cursor = start + inserted.len();
        (format!("{before}{inserted}{}", &text[end..]), cursor)
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn cell(s: &str) -> String {
    s.trim().replace('|', "\\|").replace("\r\n", "<br>").replace('\n', "<br>")
}

/// Tab 分隔、換行分列；雙引號包起來的儲存格裡可以有換行與 Tab（"" 是一個引號）。
fn parse(text: &str) -> Vec<Vec<String>> {
    let chars: Vec<char> = text.chars().collect();
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if quoted {
            if c == '"' && chars.get(i + 1) == Some(&'"') {
                cell.push('"');
                i += 1;
            } else if c == '"' {
                quoted = false;
            } else {
                cell.push(c);
            }
            i += 1;
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            '\t' => row.push(std::mem::take(&mut cell)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            _ => cell.push(c),
        }
        i += 1;
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    // TestingTests.MarkdownTable_FromTsv_Excel貼上變表格
    #[test]
    fn from_tsv_excel貼上變表格() {
        assert_eq!(
            Some("| 名稱 | 數量 |\n| --- | --- |\n| 蘋果 | 3 |\n| a\\|b | 換<br>行 |".to_owned()),
            MarkdownTable::from_tsv(Some("名稱\t數量\r\n蘋果\t3\r\na|b\t\"換\n行\"\r\n"))
        );
        assert_eq!(None, MarkdownTable::from_tsv(Some("沒有 tab 的一般文字")));
        assert_eq!(None, MarkdownTable::from_tsv(Some("")));
    }

    // TestingTests.MarkdownTable_Template_空白表格
    #[test]
    fn template_空白表格() {
        let t = MarkdownTable::template(2, 1);
        let lines: Vec<&str> = t.split('\n').collect();
        assert_eq!(vec!["| 欄位 1 | 欄位 2 |", "| --- | --- |", "|  |  |"], lines);
    }

    #[test]
    fn insert_前後補空行() {
        assert_eq!(("T\n\n".to_owned(), 3), MarkdownTable::insert("", 0..0, "T"));
        let (text, cursor) = MarkdownTable::insert("abc", 3..3, "T");
        assert_eq!("abc\n\nT\n\n", text);
        assert_eq!(text.len(), cursor);
        assert_eq!(("a\n\nT\n\nb".to_owned(), 6), MarkdownTable::insert("a\nXb", 2..3, "T"));
    }
}
