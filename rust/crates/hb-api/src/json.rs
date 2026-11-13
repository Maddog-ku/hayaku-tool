//! 寬鬆的欄位讀取（對應 C# `HayakuApi` 各 partial 裡的私有 helper：Str、Date、Day、Items、FirstStr、FirstKey、Number…）。
//!
//! C# 的 `JsonElement` 對應 `serde_json::Value`；C# 的 `default(JsonElement)`（Undefined）用 [`NULL`]（`Value::Null`）代替。
//! C# 在非物件上呼叫 `TryGetProperty`、在非數字上呼叫 `TryGetInt32` 會丟例外，這裡一律當作「沒有這個欄位」。

use chrono::{DateTime, FixedOffset, Local, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::Value;
use std::sync::LazyLock;

use crate::key_finder::KeyFinder;
use crate::models::{DateTimeOffset, PersonRef};

/// C# 的 `default(JsonElement)`。
pub static NULL: Value = Value::Null;

/// `TryGetProperty`：只有物件才有屬性。
pub fn prop<'a>(e: &'a Value, name: &str) -> Option<&'a Value> {
    e.as_object().and_then(|o| o.get(name))
}

/// `Str`：屬性存在且是字串。
pub fn str_<'a>(e: &'a Value, name: &str) -> Option<&'a str> {
    prop(e, name).and_then(Value::as_str)
}

/// `Str` 的 String 版本。
pub fn string(e: &Value, name: &str) -> Option<String> {
    str_(e, name).map(str::to_owned)
}

/// `TryGetInt32`：整數且在 i32 範圍內。
pub fn as_i32(v: &Value) -> Option<i32> {
    v.as_i64().and_then(|n| i32::try_from(n).ok())
}

/// `TryGetInt64`。
pub fn as_i64(v: &Value) -> Option<i64> {
    v.as_i64()
}

/// `TryGetDouble`：只有數字。
pub fn as_f64(v: &Value) -> Option<f64> {
    if v.is_number() { v.as_f64() } else { None }
}

/// `Bool`：屬性是 true。
pub fn bool_(e: &Value, name: &str) -> bool {
    matches!(prop(e, name), Some(Value::Bool(true)))
}

/// 屬性是 false（例 `canTime: false`）。
pub fn is_false(e: &Value, name: &str) -> bool {
    matches!(prop(e, name), Some(Value::Bool(false)))
}

/// `Date`：字串可解析成 DateTimeOffset。
pub fn date(e: &Value, name: &str) -> Option<DateTimeOffset> {
    str_(e, name).and_then(parse_date_time_offset)
}

/// `Day`：字串前 10 個字可解析成日期。
pub fn day(e: &Value, name: &str) -> Option<NaiveDate> {
    str_(e, name).and_then(|s| parse_date_only(if utf16_len(s) > 10 { truncate_utf16(s, 10) } else { s }))
}

/// `Items`：第一個是陣列的欄位的元素（都沒有是空的）。
pub fn items<'a>(e: &'a Value, names: &[&str]) -> &'a [Value] {
    for name in names {
        if let Some(Value::Array(arr)) = prop(e, name) {
            return arr;
        }
    }
    &[]
}

/// `FirstStr`：第一個不是空白的字串欄位。
pub fn first_str<'a>(e: &'a Value, names: &[&str]) -> Option<&'a str> {
    names.iter().filter_map(|n| str_(e, n)).find(|v| !is_blank(v))
}

/// `FirstStr` 的 String 版本。
pub fn first_string(e: &Value, names: &[&str]) -> Option<String> {
    first_str(e, names).map(str::to_owned)
}

/// `FirstDate`。
pub fn first_date(e: &Value, names: &[&str]) -> Option<DateTimeOffset> {
    names.iter().find_map(|n| date(e, n))
}

/// `FirstKey`：任務編號：字串（「F2P-97 標題」也行）或物件 { key / code }。
pub fn first_key(e: &Value, names: &[&str]) -> Option<String> {
    for name in names {
        let Some(v) = prop(e, name) else { continue };
        if let Some(s) = v.as_str()
            && let Some(key) = KeyFinder::parse(Some(s))
        {
            return Some(key);
        }
        if v.is_object()
            && let Some(obj_key) = str_(v, "key").or_else(|| str_(v, "code"))
        {
            return Some(KeyFinder::parse(Some(obj_key)).unwrap_or_else(|| obj_key.to_owned()));
        }
    }
    None
}

/// `FirstName`：人：字串，或 { displayName / username }。
pub fn first_name(e: &Value, name: &str) -> Option<String> {
    match prop(e, name)? {
        Value::String(s) => Some(s.clone()),
        v @ Value::Object(_) => string(v, "displayName").or_else(|| string(v, "username")),
        _ => None,
    }
}

/// `Number`：第一個是 32 位元整數的欄位。
pub fn number(e: &Value, names: &[&str]) -> Option<i32> {
    names.iter().find_map(|n| prop(e, n).and_then(as_i32))
}

/// `Count`。
pub fn count(e: &Value, name: &str) -> i32 {
    number(e, &[name]).unwrap_or(0)
}

/// `Obj`：屬性是物件就回它，否則回 [`NULL`]。
pub fn obj<'a>(e: &'a Value, name: &str) -> &'a Value {
    match prop(e, name) {
        Some(v @ Value::Object(_)) => v,
        _ => &NULL,
    }
}

/// `Person`：{ username, displayName }，沒有帳號是 None。
pub fn person(e: &Value) -> Option<PersonRef> {
    let u = str_(e, "username")?;
    Some(PersonRef::new(u, str_(e, "displayName").unwrap_or(u)))
}

/// `PersonName`：人可能是字串（顯示名稱）或 { displayName, username }。
pub fn person_name(e: &Value, names: &[&str]) -> Option<String> {
    for name in names {
        let Some(v) = prop(e, name) else { continue };
        if let Some(s) = v.as_str() {
            return Some(s.to_owned());
        }
        let display = person(v).map(|p| p.display_name).or_else(|| first_string(v, &["displayName", "name"]));
        if display.is_some() {
            return display;
        }
    }
    None
}

/// `MinutesOf`：{ minutes, display } 這類欄位的分鐘數；也接受直接是數字（備用的欄位名稱放後面）。
pub fn minutes_of(e: &Value, names: &[&str]) -> i32 {
    for name in names {
        let Some(v) = prop(e, name) else { continue };
        if v.is_object()
            && let Some(m) = prop(v, "minutes").and_then(as_f64)
        {
            return m as i32;
        }
        if let Some(n) = as_f64(v) {
            return n as i32;
        }
    }
    0
}

/// `Names`：人名清單：字串陣列或 { username, displayName } 陣列（取顯示名稱）。
pub fn names(t: &Value, name: &str) -> Vec<String> {
    items(t, &[name])
        .iter()
        .filter_map(|x| match x.as_str() {
            Some(s) => Some(s.to_owned()),
            None => person(x).map(|p| p.display_name).or_else(|| first_string(x, &["name"])),
        })
        .collect()
}

/// `Tags`。
pub fn tags(e: &Value) -> Vec<String> {
    items(e, &["tags"])
        .iter()
        .filter_map(|t| match t.as_str() {
            Some(s) => Some(s),
            None => first_str(t, &["name", "label"]),
        })
        .filter(|t| !is_blank(t))
        .map(str::to_owned)
        .collect()
}

/// `TaskCodes`。
pub fn task_codes(e: &Value) -> Vec<String> {
    items(e, &["taskCodes", "tasks"])
        .iter()
        .filter_map(|t| match t.as_str() {
            Some(s) => Some(s),
            None => first_str(t, &["key", "code", "task"]),
        })
        .filter(|t| !is_blank(t))
        .map(str::to_owned)
        .collect()
}

/// `Message`：伺服器的說明文字。
pub fn message(r: &Value) -> Option<String> {
    match r.as_str() {
        Some(s) => Some(s.to_owned()),
        None => first_string(r, &["message", "note", "changeRequest"]),
    }
}

/// `ProjectName`：專案：字串（名稱或代碼）或物件 { name / key }。
pub fn project_name(t: &Value) -> String {
    match prop(t, "project") {
        Some(Value::String(s)) => s.clone(),
        Some(p) => first_string(p, &["name", "key"]).unwrap_or_default(),
        None => String::new(),
    }
}

/// `IdText`：id 可能是字串或數字。
pub fn id_text(e: &Value, name: &str) -> Option<String> {
    match prop(e, name)? {
        Value::String(s) => Some(s.clone()),
        v @ Value::Number(_) => Some(v.to_string()),
        _ => None,
    }
}

/// `string.IsNullOrWhiteSpace`（不含 null）。
pub fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// `JsonElement.ToString()`：字串是內容，其他是原文。
pub fn element_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => raw_text(other),
    }
}

/// `JsonElement.GetRawText()`（serde_json 重新輸出，數字格式可能跟原文略有不同）。
pub fn raw_text(v: &Value) -> String {
    v.to_string()
}

/// 字串的 UTF-16 長度（C# `string.Length`）。
pub fn utf16_len(s: &str) -> usize {
    s.chars().map(char::len_utf16).sum()
}

/// 取前 n 個 UTF-16 單位（C# `s[..n]`）；不會切開一個字。
pub fn truncate_utf16(s: &str, n: usize) -> &str {
    let mut units = 0;
    for (i, c) in s.char_indices() {
        units += c.len_utf16();
        if units > n {
            return &s[..i];
        }
    }
    s
}

/// 第 i 個 UTF-16 單位（C# `s[i]`）。
pub fn utf16_at(s: &str, i: usize) -> Option<u16> {
    s.encode_utf16().nth(i)
}

static OFFSET_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^(.*?)\s*([+-])(\d{2}):?(\d{2})?$").expect("offset regex"));

const NAIVE_FORMATS: [&str; 6] = [
    "%Y-%m-%dT%H:%M:%S%.f",
    "%Y-%m-%dT%H:%M",
    "%Y-%m-%d %H:%M:%S%.f",
    "%Y-%m-%d %H:%M",
    "%Y/%m/%d %H:%M:%S%.f",
    "%Y/%m/%d %H:%M",
];

/// `DateTimeOffset.TryParse`：ISO 8601（含 Z 或 +08:00）；沒有時區的當本地時間；只有日期是本地午夜。
pub fn parse_date_time_offset(s: &str) -> Option<DateTimeOffset> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d);
    }
    // 拆出時區
    let (body, offset) = if let Some(body) = s.strip_suffix(['Z', 'z']) {
        (body.trim_end(), Some(FixedOffset::east_opt(0)?))
    } else if let Some(c) = OFFSET_RE.captures(s)
        && c.get(1).is_some_and(|b| b.as_str().contains(':'))
    {
        let sign = if &c[2] == "-" { -1 } else { 1 };
        let h: i32 = c[3].parse().ok()?;
        let m: i32 = c.get(4).map_or(Some(0), |m| m.as_str().parse().ok())?;
        if h > 14 || m > 59 {
            return None;
        }
        (c.get(1).map_or("", |b| b.as_str()), Some(FixedOffset::east_opt(sign * (h * 3600 + m * 60))?))
    } else {
        (s, None)
    };
    let naive = parse_naive(body)?;
    match offset {
        Some(off) => off.from_local_datetime(&naive).single(),
        None => Local.from_local_datetime(&naive).earliest().map(|d| d.fixed_offset()),
    }
}

fn parse_naive(s: &str) -> Option<NaiveDateTime> {
    NAIVE_FORMATS
        .iter()
        .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
        .or_else(|| parse_date_only(s).and_then(|d| d.and_hms_opt(0, 0, 0)))
}

/// `DateOnly.TryParse`：yyyy-MM-dd 或 yyyy/MM/dd。
pub fn parse_date_only(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    NaiveDate::parse_from_str(s, "%Y-%m-%d").or_else(|_| NaiveDate::parse_from_str(s, "%Y/%m/%d")).ok()
}

/// `DateOnly.ToString("yyyy-MM-dd")`。
pub fn iso(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// `DateTimeOffset.ToString("yyyy-MM-dd'T'HH:mm:sszzz")`。
pub fn iso_time(t: &DateTimeOffset) -> String {
    t.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

/// `DateTimeOffset.Now`。
pub fn now() -> DateTimeOffset {
    Local::now().fixed_offset()
}

/// `DateTimeOffset.MinValue`。
pub fn min_value() -> DateTimeOffset {
    DateTime::<Utc>::MIN_UTC.fixed_offset()
}

/// `DateTimeOffset.MaxValue`。
pub fn max_value() -> DateTimeOffset {
    DateTime::<Utc>::MAX_UTC.fixed_offset()
}

/// `ToLocalTime().ToString("MM/dd HH:mm")`。
pub fn local_short(t: &DateTimeOffset) -> String {
    t.with_timezone(&Local).format("%m/%d %H:%M").to_string()
}

/// `Math.Round(x, digits)`（預設 ToEven，.NET 的做法：乘上 10^digits 取整再除回去）。
pub fn round_even(x: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    (x * p).round_ties_even() / p
}

/// `Math.Round(x, digits, MidpointRounding.AwayFromZero)`。
pub fn round_away(x: f64, digits: i32) -> f64 {
    let p = 10f64.powi(digits);
    let v = x * p;
    let t = v.trunc();
    let f = v - t;
    let r = if f.abs() >= 0.5 { t + f.signum() } else { t };
    r / p
}

/// `Convert.FromBase64String`：忽略空白，長度要是 4 的倍數，結尾最多兩個 =。
pub fn from_base64(s: &str) -> Option<Vec<u8>> {
    let chars: Vec<u8> = s.bytes().filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n')).collect();
    if !chars.len().is_multiple_of(4) {
        return None;
    }
    let pad = chars.iter().rev().take_while(|&&b| b == b'=').count();
    if pad > 2 {
        return None;
    }
    let body = &chars[..chars.len() - pad];
    fn val(b: u8) -> Option<u32> {
        Some(match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0;
    for &b in body {
        acc = (acc << 6) | val(b)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// `Guid.NewGuid().ToString()`：沒有 id 的計時段用的暫時 id（隨機 v4 格式）。
pub fn new_guid() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut words = [0u64; 2];
    for (i, w) in words.iter_mut().enumerate() {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
        h.write_usize(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
        *w = h.finish();
    }
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&words[0].to_le_bytes());
    b[8..].copy_from_slice(&words[1].to_le_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Timelike;

    #[test]
    fn 日期時間_各種寫法() {
        let z = parse_date_time_offset("2026-10-27T07:44:12.090Z").expect("z");
        assert_eq!(z.offset().local_minus_utc(), 0);
        assert_eq!(z.timestamp_millis() % 1000, 90);
        let off = parse_date_time_offset("2026-10-27T15:44:12+08:00").expect("off");
        assert_eq!(off.with_timezone(&Utc).hour(), 7);
        assert!(parse_date_time_offset("2026-10-27T07:44Z").is_some());
        assert!(parse_date_time_offset("2026-10-27 07:44:00").is_some());
        assert!(parse_date_time_offset("2026-10-27T07:44:00").is_some());
        assert!(parse_date_time_offset("2026-10-27").is_some());
        assert!(parse_date_time_offset("not a date").is_none());
        assert!(parse_date_time_offset("").is_none());
    }

    #[test]
    fn base64() {
        assert_eq!(from_base64("iVBORw0KGgo="), Some(vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]));
        assert_eq!(from_base64("YQ=="), Some(b"a".to_vec()));
        assert_eq!(from_base64("abc"), None);
        assert_eq!(from_base64("a$=="), None);
    }

    #[test]
    fn 四捨五入() {
        assert_eq!(round_even(1.5, 2), 1.5);
        assert_eq!(round_away(2.25, 1), 2.3);
        assert_eq!(round_away(0.04, 1), 0.0);
    }

    #[test]
    fn guid_格式() {
        let g = new_guid();
        assert_eq!(g.len(), 36);
        assert_ne!(g, new_guid());
    }
}
