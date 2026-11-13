//! 從文字找出 Hayaku 任務編號（例 F2P-97、EG-7）。對應 C# `Core/KeyFinder.cs`。
//!
//! C# 用 `(?<![A-Za-z0-9])([A-Za-z][A-Za-z0-9]{0,14})-(\d{1,6})(?![0-9])`；Rust 的 regex 不支援前後環視，
//! 這裡照 .NET 的比對與回溯規則手寫掃描（`\d` 是 Unicode 數字、只認 BMP 內的字，跟 .NET 的 UTF-16 一樣）。

use std::sync::LazyLock;

static UNICODE_DIGIT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"^\d$").expect("digit regex"));

/// .NET `\d`：Unicode 十進位數字（Nd），只認 BMP（UTF-16 的代理對不算）。
fn is_net_digit(c: char) -> bool {
    if c.is_ascii_digit() {
        return true;
    }
    if c.is_ascii() || (c as u32) > 0xFFFF {
        return false;
    }
    let mut buf = [0u8; 4];
    UNICODE_DIGIT.is_match(c.encode_utf8(&mut buf))
}

/// 一個比對結果：代碼（第 1 組）與數字（第 2 組）。
struct KeyMatch<'a> {
    prefix: &'a str,
    digits: &'a str,
}

/// `KeyRegex().Matches(text)`：由左到右、不重疊。
fn matches(text: &str) -> Vec<KeyMatch<'_>> {
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = chars.len();
    let byte_at = |i: usize| if i < n { chars[i].0 } else { text.len() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if let Some((prefix_end, digits_end)) = match_at(&chars, i) {
            out.push(KeyMatch {
                prefix: &text[byte_at(i)..byte_at(prefix_end)],
                digits: &text[byte_at(prefix_end + 1)..byte_at(digits_end)],
            });
            i = digits_end;
        } else {
            i += 1;
        }
    }
    out
}

/// 從第 i 個字開始比對；成功回（代碼結束位置＝「-」的位置、數字結束位置）。
fn match_at(chars: &[(usize, char)], i: usize) -> Option<(usize, usize)> {
    let n = chars.len();
    // (?<![A-Za-z0-9])
    if i > 0 && chars[i - 1].1.is_ascii_alphanumeric() {
        return None;
    }
    // [A-Za-z][A-Za-z0-9]{0,14}：後面一定要接「-」，回溯成較短的也不會遇到「-」，所以只看貪婪的結果
    if !chars[i].1.is_ascii_alphabetic() {
        return None;
    }
    let mut j = i + 1;
    while j < n && j - (i + 1) < 14 && chars[j].1.is_ascii_alphanumeric() {
        j += 1;
    }
    if j >= n || chars[j].1 != '-' {
        return None;
    }
    // (\d{1,6})(?![0-9])：貪婪後往回退
    let start = j + 1;
    let mut m = start;
    while m < n && m - start < 6 && is_net_digit(chars[m].1) {
        m += 1;
    }
    (start + 1..=m).rev().find(|&e| e >= n || !chars[e].1.is_ascii_digit()).map(|e| (j, e))
}

fn key_of(m: &KeyMatch<'_>) -> String {
    format!("{}-{}", m.prefix.to_ascii_uppercase(), m.digits)
}

/// 從文字找出 Hayaku 任務編號（例 F2P-97、EG-7）。
pub struct KeyFinder;

impl KeyFinder {
    /// 對話內容：只認大寫代碼，取最後一個提到的。
    pub fn find_in_prompt(text: Option<&str>, is_known: impl Fn(&str) -> bool) -> Option<String> {
        let text = text.filter(|t| !t.is_empty())?;
        let mut last = None;
        for m in matches(text) {
            if m.prefix != m.prefix.to_ascii_uppercase() {
                continue;
            }
            let key = format!("{}-{}", m.prefix, m.digits);
            if is_known(&key) {
                last = Some(key);
            }
        }
        last
    }

    /// git 分支名稱：大小寫都認（feature/f2p-97-xxx），取第一個。
    pub fn find_in_branch(branch: Option<&str>, is_known: impl Fn(&str) -> bool) -> Option<String> {
        let branch = branch.filter(|t| !t.is_empty())?;
        matches(branch).iter().map(key_of).find(|k| is_known(k))
    }

    /// 使用者輸入的文字（「F2P-97  報表頁面…」或「f2p-97」）→ 任務編號。
    pub fn parse(text: Option<&str>) -> Option<String> {
        let text = text.filter(|t| !crate::json::is_blank(t))?;
        matches(text.trim()).first().map(key_of)
    }

    /// 文字裡出現的所有任務編號（照出現順序，代碼轉大寫）。
    pub fn all(text: Option<&str>) -> Vec<String> {
        match text {
            Some(t) if !t.is_empty() => matches(t).iter().map(key_of).collect(),
            _ => Vec::new(),
        }
    }

    pub fn project_of(key: &str) -> &str {
        match key.rfind('-') {
            Some(dash) if dash > 0 => &key[..dash],
            _ => key,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::KeyFinder;

    // 對應 KeyFinderTests
    fn known(key: &str) -> bool {
        let project = KeyFinder::project_of(key).to_uppercase();
        ["F2P", "EG", "REVIVAL"].contains(&project.as_str())
    }

    #[test]
    fn prompt_取最後提到的已知任務() {
        assert_eq!(
            KeyFinder::find_in_prompt(Some("先看 F2P-97，再處理 EG-7 的 UTF-8 問題"), known).as_deref(),
            Some("EG-7")
        );
    }

    #[test]
    fn prompt_不認小寫與未知專案() {
        assert_eq!(KeyFinder::find_in_prompt(Some("f2p-97 跟 UTF-8、ISO-8859"), known), None);
    }

    #[test]
    fn branch_大小寫都認() {
        let cases = [
            ("feature/f2p-97-reset-button", Some("F2P-97")),
            ("F2P-14", Some("F2P-14")),
            ("bugfix/EG-18_sync", Some("EG-18")),
            ("main", None),
            ("release-2026-10", None),
        ];
        for (branch, expected) in cases {
            assert_eq!(KeyFinder::find_in_branch(Some(branch), known).as_deref(), expected, "{branch}");
        }
    }

    #[test]
    fn parse_使用者輸入() {
        let cases = [("F2P-97  報表頁面新增歸0按鈕", Some("F2P-97")), (" f2p-5 ", Some("F2P-5")), ("沒有編號", None)];
        for (text, expected) in cases {
            assert_eq!(KeyFinder::parse(Some(text)).as_deref(), expected, "{text}");
        }
    }

    #[test]
    fn 比對邊界_跟_net_一樣() {
        // 前面是英數字不算；數字後面接數字（超過 6 位）往回退也不行
        assert_eq!(KeyFinder::all(Some("9F2P-1 A-1234567 B-12x C-3")), vec!["B-12", "C-3"]);
        // 代碼最多 15 個字
        assert_eq!(KeyFinder::parse(Some("ABCDEFGHIJKLMNOP-1")), None);
        assert_eq!(KeyFinder::parse(Some("ABCDEFGHIJKLMNO-1")).as_deref(), Some("ABCDEFGHIJKLMNO-1"));
        assert_eq!(KeyFinder::project_of("F2P-97"), "F2P");
        assert_eq!(KeyFinder::project_of("F2P"), "F2P");
    }
}
