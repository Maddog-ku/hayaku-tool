//! 模組間共用的小工具（C# 內建行為的對應：OrdinalIgnoreCase、string.Length（UTF-16）、FileInfo、JsonElement 讀欄位）。

use std::io::Read;
use std::path::Path;
use std::time::SystemTime;

use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveDateTime};
use serde_json::Value;

/// OrdinalIgnoreCase 的比較鍵（逐字轉大寫；一個字會變成多個字的不轉，跟 .NET 一樣）。
pub(crate) fn fold(s: &str) -> String {
    s.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

/// `string.Equals(a, b, StringComparison.OrdinalIgnoreCase)`。
pub fn eq_ic(a: &str, b: &str) -> bool {
    fold(a) == fold(b)
}

/// `s.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)`。
pub(crate) fn starts_with_ic(s: &str, prefix: &str) -> bool {
    fold(s).starts_with(&fold(prefix))
}

/// `s.Contains(part, StringComparison.OrdinalIgnoreCase)`。
pub(crate) fn contains_ic(s: &str, part: &str) -> bool {
    fold(s).contains(&fold(part))
}

/// C# `string.Length`（UTF-16 單位數）。
pub(crate) fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// C# `s[..n]`（以 UTF-16 單位計）；切在代理對中間時少取那個字。
pub(crate) fn left_utf16(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// 檔案時間 → 本機時間（C# `FileInfo.LastWriteTime`）。
pub(crate) fn local_time(t: SystemTime) -> NaiveDateTime {
    DateTime::<Local>::from(t).naive_local()
}

/// `DateTime.Now`。
pub(crate) fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

/// JsonElement 的字串屬性（不是物件、沒有或不是字串回 None）。
pub(crate) fn str_prop<'a>(e: &'a Value, name: &str) -> Option<&'a str> {
    e.as_object()?.get(name)?.as_str()
}

/// 屬性是 JSON true。
pub(crate) fn flag(e: &Value, name: &str) -> bool {
    e.as_object().and_then(|o| o.get(name)).is_some_and(|v| v.as_bool() == Some(true))
}

/// zh-TW 的 `ToString("M/d（ddd）")`：10/30（週五）。
pub(crate) fn zh_month_day(d: NaiveDate) -> String {
    const DAYS: [&str; 7] = ["週一", "週二", "週三", "週四", "週五", "週六", "週日"];
    format!("{}/{}（{}）", d.month(), d.day(), DAYS[d.weekday().num_days_from_monday() as usize])
}

/// 一個檔案（C# `FileInfo` 用到的部分）。
#[derive(Debug, Clone)]
pub(crate) struct FileEntry {
    pub full_name: String,
    pub name: String,
    pub last_write: NaiveDateTime,
    pub creation: NaiveDateTime,
    pub length: u64,
}

impl FileEntry {
    /// `Path.GetFileNameWithoutExtension`。
    pub fn stem(&self) -> String {
        match self.name.rfind('.') {
            Some(i) => self.name[..i].to_owned(),
            None => self.name.clone(),
        }
    }

    pub fn from_path(path: &Path) -> Option<FileEntry> {
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        Some(FileEntry {
            full_name: path.to_string_lossy().into_owned(),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            last_write: meta.modified().map(local_time).unwrap_or(NaiveDateTime::MIN),
            creation: meta.created().map(local_time).unwrap_or(NaiveDateTime::MIN),
            length: meta.len(),
        })
    }
}

fn is_dir_type(ft: &std::fs::FileType) -> bool {
    use std::os::windows::fs::FileTypeExt;
    ft.is_dir() || ft.is_symlink_dir()
}

fn is_file_type(ft: &std::fs::FileType) -> bool {
    use std::os::windows::fs::FileTypeExt;
    ft.is_file() || ft.is_symlink_file()
}

/// `Directory.GetDirectories(dir)`：完整路徑。
pub(crate) fn subdirs(dir: &Path) -> std::io::Result<Vec<String>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type().is_ok_and(|ft| is_dir_type(&ft)) {
            out.push(entry.path().to_string_lossy().into_owned());
        }
    }
    Ok(out)
}

/// `new DirectoryInfo(dir).EnumerateFiles("*.jsonl")`。
pub(crate) fn jsonl_files(dir: &Path) -> std::io::Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let Ok(ft) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().into_owned();
        if !is_file_type(&ft) || !fold(&name).ends_with(".JSONL") {
            continue;
        }
        let meta = entry.metadata()?;
        out.push(FileEntry {
            full_name: entry.path().to_string_lossy().into_owned(),
            name,
            last_write: meta.modified().map(local_time).unwrap_or(NaiveDateTime::MIN),
            creation: meta.created().map(local_time).unwrap_or(NaiveDateTime::MIN),
            length: meta.len(),
        });
    }
    Ok(out)
}

/// `MaxBy(f => f.LastWriteTime)`：一樣大時取第一個。
pub(crate) fn newest(files: Vec<FileEntry>) -> Option<FileEntry> {
    let mut best: Option<FileEntry> = None;
    for f in files {
        if best.as_ref().is_none_or(|b| f.last_write > b.last_write) {
            best = Some(f);
        }
    }
    best
}

/// 讀檔案開頭最多 max 個位元組（共用讀寫，跟 C# `FileShare.ReadWrite | FileShare.Delete` 一樣）。
pub(crate) fn read_head(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.take(max).read_to_end(&mut buf)?;
    Ok(buf)
}

/// `Encoding.UTF8.GetString`（不合法的位元組換成 U+FFFD）。
pub(crate) fn utf8(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// LINQ `MaxBy`：一樣大時取第一個。
pub(crate) fn max_by_first<T, K: PartialOrd>(items: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> Option<T> {
    let mut best: Option<(T, K)> = None;
    for item in items {
        let k = key(&item);
        if best.as_ref().is_none_or(|(_, bk)| k > *bk) {
            best = Some((item, k));
        }
    }
    best.map(|(t, _)| t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 不分大小寫比較() {
        assert!(eq_ic("D:\\Repo", "d:\\REPO"));
        assert!(!eq_ic("D:\\Repo", "D:\\Repo2"));
        assert!(starts_with_ic("D:\\Repo\\x", "d:\\repo\\"));
    }

    #[test]
    fn utf16_長度與截斷() {
        assert_eq!(2, utf16_len("😀"));
        assert_eq!("ab", left_utf16("ab😀", 3));
        assert_eq!("字字", left_utf16("字字字", 2));
    }
}
