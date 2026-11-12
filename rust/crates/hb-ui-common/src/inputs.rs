//! 對應 C# `UI/Work/Inputs.cs`：日期、時間輸入的寬鬆解析與顯示，以及「執行動作、失敗顯示訊息」的共用流程。
//!
//! C# 的 `Show(TextBlock, …)`、`Run(Button, TextBlock, …)` 直接改 WPF 控制項；這裡改成回呼：
//! [`StatusMessage`] 是要顯示的訊息與顏色，[`run`] 用 `set_busy`／`show` 兩個回呼讓 Slint 端更新按鈕與訊息列。

use std::future::Future;

use chrono::{DateTime, Datelike, FixedOffset, Local, Months, NaiveDate, NaiveTime, TimeDelta, TimeZone};
use hb_api::{DateTimeOffset, McpError};
use hb_core::CoreError;

/// 按鈕執行中的文字（C# `button.Content = "處理中…"`）。
pub const BUSY_TEXT: &str = "處理中…";

/// 今天（C# `DateTime.Today`）。
fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// 依格式解析數字：`n` 位數字（min..=max，盡量多拿，跟 .NET ParseExact 一樣貪婪）。
fn take_digits(s: &[u8], pos: &mut usize, min: usize, max: usize) -> Option<u32> {
    let start = *pos;
    let mut end = start;
    while end < s.len() && end - start < max && s[end].is_ascii_digit() {
        end += 1;
    }
    if end - start < min {
        return None;
    }
    *pos = end;
    std::str::from_utf8(&s[start..end]).ok()?.parse().ok()
}

fn take_char(s: &[u8], pos: &mut usize, c: u8) -> Option<()> {
    if s.get(*pos) == Some(&c) {
        *pos += 1;
        Some(())
    } else {
        None
    }
}

/// .NET 格式字串（只用到 yyyy、MM、M、dd、d、HH、H、mm 與字面字元）的精確解析：回傳 (年, 月, 日) 或 (時, 分)。
/// 每個 token 依 .NET 規則：單一字母 1–2 位、其他固定位數。
fn parse_exact(text: &str, format: &str) -> Option<Vec<(char, u32)>> {
    let s = text.as_bytes();
    let f: Vec<char> = format.chars().collect();
    let mut pos = 0;
    let mut i = 0;
    let mut out = Vec::new();
    while i < f.len() {
        let c = f[i];
        if c == '%' {
            i += 1;
            continue;
        }
        if matches!(c, 'y' | 'M' | 'd' | 'H' | 'm') {
            let mut n = 1;
            while i + n < f.len() && f[i + n] == c {
                n += 1;
            }
            let (min, max) = if n == 1 { (1, 2) } else { (n, n) };
            out.push((c, take_digits(s, &mut pos, min, max)?));
            i += n;
        } else {
            take_char(s, &mut pos, c as u8)?;
            i += 1;
        }
    }
    (pos == s.len()).then_some(out)
}

const DATE_FORMATS: [&str; 7] = ["yyyy-MM-dd", "yyyy/M/d", "yyyy-M-d", "M/d", "M-d", "MMdd", "yyyyMMdd"];
const TIME_FORMATS: [&str; 5] = ["H:mm", "HH:mm", "Hmm", "HHmm", "%H"];

fn try_parse_date_exact(t: &str) -> Option<NaiveDate> {
    for format in DATE_FORMATS {
        let Some(parts) = parse_exact(t, format) else { continue };
        let get = |c: char| parts.iter().find(|p| p.0 == c).map(|p| p.1);
        // 格式沒有年份時 .NET 用今年
        let year = get('y').map_or(today().year(), |y| y as i32);
        if year < 1 {
            continue;
        }
        if let (Some(m), Some(d)) = (get('M'), get('d'))
            && let Some(date) = NaiveDate::from_ymd_opt(year, m, d)
        {
            return Some(date);
        }
    }
    None
}

/// 加減天數（超出範圍時 C# 會丟例外，這裡回 None）。
fn add_days(d: NaiveDate, days: i64) -> Option<NaiveDate> {
    d.checked_add_signed(TimeDelta::try_days(days)?)
}

/// 今天、明天、後天、+3、11/5、2026-11-05 …；空白回 None。today 為 None 時用今天。
pub fn parse_date(text: Option<&str>, today_: Option<NaiveDate>) -> Option<NaiveDate> {
    let t = text.unwrap_or("").trim();
    let base_day = today_.unwrap_or_else(today);
    if t.is_empty() {
        return None;
    }
    match t {
        "今天" | "今日" => return Some(base_day),
        "明天" | "明日" => return add_days(base_day, 1),
        "後天" => return add_days(base_day, 2),
        "昨天" => return add_days(base_day, -1),
        _ => {}
    }
    if (t.starts_with('+') || t.starts_with('-'))
        && let Ok(offset) = t.parse::<i32>()
    {
        return add_days(base_day, offset as i64);
    }
    let mut d = try_parse_date_exact(t)?;
    // 只打月日：年份用今年；已經過了很久（超過半年）就當明年
    if !t.contains(&base_day.year().to_string()) && t.encode_utf16().count() <= 5 && d.year() == today().year() {
        d = NaiveDate::from_ymd_opt(base_day.year(), d.month(), d.day())?;
        if let Some(limit) = base_day.checked_sub_months(Months::new(6))
            && d < limit
        {
            d = d.checked_add_months(Months::new(12))?;
        }
    }
    Some(d)
}

pub fn parse_time(text: Option<&str>) -> Option<NaiveTime> {
    let t = text.unwrap_or("").trim();
    for format in TIME_FORMATS {
        let Some(parts) = parse_exact(t, format) else { continue };
        let get = |c: char| parts.iter().find(|p| p.0 == c).map(|p| p.1);
        if let Some(time) = NaiveTime::from_hms_opt(get('H').unwrap_or(0), get('m').unwrap_or(0), 0) {
            return Some(time);
        }
    }
    None
}

/// `yyyy-MM-dd`；None 是空字串。
pub fn date_text(d: Option<NaiveDate>) -> String {
    d.map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
}

/// zh-TW 的星期簡稱（C# `ddd`）。
pub fn weekday_short(d: NaiveDate) -> &'static str {
    ["週一", "週二", "週三", "週四", "週五", "週六", "週日"][d.weekday().num_days_from_monday() as usize]
}

/// 今天、明天、M/d（週X）、yyyy/M/d。
pub fn short_date(d: NaiveDate) -> String {
    short_date_at(d, today())
}

pub fn short_date_at(d: NaiveDate, today: NaiveDate) -> String {
    if d == today {
        "今天".to_owned()
    } else if Some(d) == add_days(today, 1) {
        "明天".to_owned()
    } else if d.year() == today.year() {
        format!("{}/{}（{}）", d.month(), d.day(), weekday_short(d))
    } else {
        format!("{}/{}/{}", d.year(), d.month(), d.day())
    }
}

/// 本地日期＋時間 → 帶時區的時間。
pub fn local(day: NaiveDate, time: NaiveTime) -> DateTimeOffset {
    let dt = day.and_time(time);
    match Local.from_local_datetime(&dt) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) => t.fixed_offset(),
        chrono::LocalResult::None => Local.from_utc_datetime(&dt).fixed_offset(),
    }
}

/// `yyyy-MM-dd'T'HH:mm:sszzz`。
pub fn iso(t: &DateTime<FixedOffset>) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// 剛剛、N 分鐘前、N 小時前、N 天前，超過一週顯示 M/d HH:mm。
pub fn ago(time: &DateTimeOffset) -> String {
    ago_at(time, &Local::now().fixed_offset())
}

pub fn ago_at(time: &DateTimeOffset, now: &DateTimeOffset) -> String {
    let span = *now - *time;
    let minutes = span.num_milliseconds() as f64 / 60000.0;
    if minutes < 1.0 {
        "剛剛".to_owned()
    } else if minutes < 60.0 {
        format!("{} 分鐘前", minutes as i64)
    } else if minutes < 1440.0 {
        format!("{} 小時前", (minutes / 60.0) as i64)
    } else if minutes < 1440.0 * 7.0 {
        format!("{} 天前", (minutes / 1440.0) as i64)
    } else {
        time.with_timezone(&Local).format("%-m/%-d %H:%M").to_string()
    }
}

/// 訊息列的內容（C# `Show(target, message, error)`）：None＝收起來；error 決定紅色（ErrorBrush）或綠色（RunBrush）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StatusMessage {
    pub text: String,
    pub error: bool,
}

impl StatusMessage {
    pub fn error(text: impl Into<String>) -> Self {
        Self { text: text.into(), error: true }
    }

    pub fn ok(text: impl Into<String>) -> Self {
        Self { text: text.into(), error: false }
    }

    pub fn visible(&self) -> bool {
        !self.text.is_empty()
    }
}

/// [`run`] 的動作失敗原因：Hayaku／網路錯誤直接顯示訊息，其他錯誤前面加「發生錯誤：」。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    /// C# 的 McpException／HttpRequestException／TaskCanceledException
    Mcp(String),
    Other(String),
}

impl ActionError {
    pub fn display(&self) -> String {
        match self {
            ActionError::Mcp(m) => m.clone(),
            ActionError::Other(m) => format!("發生錯誤：{m}"),
        }
    }
}

impl From<CoreError> for ActionError {
    fn from(e: CoreError) -> Self {
        ActionError::Mcp(e.message().to_owned())
    }
}

impl From<McpError> for ActionError {
    fn from(e: McpError) -> Self {
        ActionError::Mcp(e.0)
    }
}

impl From<String> for ActionError {
    fn from(e: String) -> Self {
        ActionError::Other(e)
    }
}

/// 執行一個動作；按鈕先變成處理中（`set_busy(true)`，按鈕文字用 [`BUSY_TEXT`]），失敗時把訊息顯示在 status。
/// 成功且有 success 時顯示成功訊息。回傳是否成功。
pub async fn run<F, Fut>(
    set_busy: impl Fn(bool),
    show: impl Fn(Option<StatusMessage>),
    action: F,
    success: Option<&str>,
) -> bool
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<(), ActionError>>,
{
    set_busy(true);
    show(None);
    let ok = match action().await {
        Ok(()) => {
            if let Some(s) = success {
                show(Some(StatusMessage::ok(s)));
            }
            true
        }
        Err(e) => {
            log::error!("工作台動作失敗: {}", e.display());
            show(Some(StatusMessage::error(e.display())));
            false
        }
    };
    set_busy(false);
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    // WorkTests.ParseDate_支援常見寫法
    #[test]
    fn parse_date_支援常見寫法() {
        let today = ymd(2026, 10, 28);
        assert_eq!(Some(today), parse_date(Some("今天"), Some(today)));
        assert_eq!(Some(ymd(2026, 10, 29)), parse_date(Some("明天"), Some(today)));
        assert_eq!(Some(ymd(2026, 10, 31)), parse_date(Some("+3"), Some(today)));
        assert_eq!(Some(ymd(2026, 11, 5)), parse_date(Some("11/5"), Some(today)));
        assert_eq!(Some(ymd(2026, 11, 5)), parse_date(Some("2026-11-05"), Some(today)));
        assert_eq!(None, parse_date(Some("下下週"), Some(today)));
        assert_eq!(None, parse_date(Some(""), Some(today)));
    }

    // WorkTests.ParseTime_支援冒號與四位數
    #[test]
    fn parse_time_支援冒號與四位數() {
        assert_eq!(NaiveTime::from_hms_opt(9, 30, 0), parse_time(Some("9:30")));
        assert_eq!(NaiveTime::from_hms_opt(14, 0, 0), parse_time(Some("1400")));
        assert_eq!(None, parse_time(Some("25:00")));
    }

    #[test]
    fn parse_date_其他格式() {
        let today = ymd(2026, 10, 28);
        assert_eq!(Some(ymd(2026, 11, 5)), parse_date(Some("1105"), Some(today)));
        assert_eq!(Some(ymd(2026, 11, 5)), parse_date(Some("20261105"), Some(today)));
        assert_eq!(Some(ymd(2026, 11, 5)), parse_date(Some("2026/11/5"), Some(today)));
        assert_eq!(Some(ymd(2026, 10, 27)), parse_date(Some("昨天"), Some(today)));
        assert_eq!(Some(ymd(2026, 10, 25)), parse_date(Some("-3"), Some(today)));
        assert_eq!(None, parse_date(Some("13/40"), Some(today)));
    }

    #[test]
    fn short_date_今天明天與星期() {
        let today = ymd(2026, 11, 12);
        assert_eq!("今天", short_date_at(today, today));
        assert_eq!("明天", short_date_at(ymd(2026, 11, 13), today));
        assert_eq!("11/16（週一）", short_date_at(ymd(2026, 11, 16), today));
        assert_eq!("2027/1/5", short_date_at(ymd(2027, 1, 5), today));
    }

    #[test]
    fn ago_各級距() {
        let now = DateTime::parse_from_rfc3339("2026-11-12T10:00:00+08:00").unwrap();
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap();
        assert_eq!("剛剛", ago_at(&at("2026-11-12T09:59:30+08:00"), &now));
        assert_eq!("5 分鐘前", ago_at(&at("2026-11-12T09:55:00+08:00"), &now));
        assert_eq!("3 小時前", ago_at(&at("2026-11-12T07:00:00+08:00"), &now));
        assert_eq!("2 天前", ago_at(&at("2026-11-10T09:00:00+08:00"), &now));
    }

    #[test]
    fn iso_帶時區() {
        let t = DateTime::parse_from_rfc3339("2026-11-12T09:05:00+08:00").unwrap();
        assert_eq!("2026-11-12T09:05:00+08:00", iso(&t));
    }

    #[test]
    fn action_error_顯示() {
        assert_eq!("網路錯誤", ActionError::Mcp("網路錯誤".into()).display());
        assert_eq!("發生錯誤：x", ActionError::Other("x".into()).display());
    }
}
