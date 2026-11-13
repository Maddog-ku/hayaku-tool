//! 對應 C# `Infra/JsonFile.cs`：設定檔的讀寫。
//!
//! 格式照 C# `JsonFile.Options`（System.Text.Json）：屬性 camelCase（由各型別的 serde 屬性負責）、
//! 縮排兩格、換行 CRLF（.NET 在 Windows 用 `Environment.NewLine`）、`UnsafeRelaxedJsonEscaping`（中文不跳脫）。
//! 寫入先寫 `.tmp` 再取代。

use std::io::{self, Write};
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::ser::{CharEscape, Formatter};

/// 讀檔；檔案不存在、內容是 `null` 回傳預設值；讀不懂寫紀錄後回傳預設值。
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> T {
    match try_load(path) {
        Ok(Some(v)) => v,
        Ok(None) => T::default(),
        Err(err) => {
            log::error!("讀取 {} 失敗: {err}", path.display());
            T::default()
        }
    }
}

fn try_load<T: DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    from_slice(&bytes)
}

/// 解析 JSON（去掉 UTF-8 BOM；`null` 回傳 None，跟 C# `Deserialize` 回傳 null 一樣）。
pub fn from_slice<T: DeserializeOwned>(bytes: &[u8]) -> anyhow::Result<Option<T>> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let value: serde_json::Value = serde_json::from_slice(bytes)?;
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(serde_json::from_value(value)?))
}

/// 寫檔（先寫 .tmp 再取代）；失敗寫紀錄。
pub fn save<T: Serialize + ?Sized>(path: &Path, value: &T) {
    if let Err(err) = try_save(path, value) {
        log::error!("寫入 {} 失敗: {err}", path.display());
    }
}

/// 寫檔（先寫 .tmp 再取代）；失敗回傳錯誤。
pub fn try_save<T: Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, to_string(value)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 序列化成 C# `JsonFile.Options` 的格式（縮排兩格、CRLF、中文不跳脫）。
pub fn to_string<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<String> {
    serialize(value, true)
}

/// 同 [`to_string`]，回傳 UTF-8 位元組（C# `SerializeToUtf8Bytes`）。
pub fn to_vec<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<Vec<u8>> {
    to_string(value).map(String::into_bytes)
}

/// 不縮排、跳脫規則相同的版本。
pub fn to_string_compact<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<String> {
    serialize(value, false)
}

fn serialize<T: Serialize + ?Sized>(value: &T, indented: bool) -> serde_json::Result<String> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, NetFormatter::new(indented));
    value.serialize(&mut ser)?;
    // 格式器只寫出合法的 UTF-8（字串片段原樣寫出、跳脫都是 ASCII）
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// 模仿 System.Text.Json（`JavaScriptEncoder.UnsafeRelaxedJsonEscaping`、Windows 換行 CRLF）的輸出：
/// 中文照原樣；控制字元、特殊空白、未定義字元寫成大寫的 `\uXXXX`；補充平面字元（emoji）寫成代理對。
pub struct NetFormatter {
    indented: bool,
    indent: usize,
    has_value: bool,
}

impl NetFormatter {
    pub fn new(indented: bool) -> Self {
        Self { indented, indent: 0, has_value: false }
    }

    fn newline<W: ?Sized + Write>(&self, w: &mut W) -> io::Result<()> {
        if self.indented {
            w.write_all(b"\r\n")?;
            for _ in 0..self.indent {
                w.write_all(b"  ")?;
            }
        }
        Ok(())
    }
}

fn escaped_bmp(c: u32) -> bool {
    let table = super::json_escape_table::ESCAPED_BMP;
    let i = table.partition_point(|&(_, end)| u32::from(end) < c);
    i < table.len() && u32::from(table[i].0) <= c
}

fn write_unit<W: ?Sized + Write>(w: &mut W, unit: u32) -> io::Result<()> {
    write!(w, "\\u{unit:04X}")
}

impl Formatter for NetFormatter {
    fn begin_array<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.indent += 1;
        self.has_value = false;
        w.write_all(b"[")
    }

    fn end_array<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.indent -= 1;
        if self.has_value {
            self.newline(w)?;
        }
        w.write_all(b"]")
    }

    fn begin_array_value<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if !first {
            w.write_all(b",")?;
        }
        self.newline(w)
    }

    fn end_array_value<W: ?Sized + Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }

    fn begin_object<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.indent += 1;
        self.has_value = false;
        w.write_all(b"{")
    }

    fn end_object<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        self.indent -= 1;
        if self.has_value {
            self.newline(w)?;
        }
        w.write_all(b"}")
    }

    fn begin_object_key<W: ?Sized + Write>(&mut self, w: &mut W, first: bool) -> io::Result<()> {
        if !first {
            w.write_all(b",")?;
        }
        self.newline(w)
    }

    fn begin_object_value<W: ?Sized + Write>(&mut self, w: &mut W) -> io::Result<()> {
        w.write_all(if self.indented { b": " } else { b":" })
    }

    fn end_object_value<W: ?Sized + Write>(&mut self, _w: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }

    fn write_string_fragment<W: ?Sized + Write>(&mut self, w: &mut W, fragment: &str) -> io::Result<()> {
        let mut start = 0;
        for (i, c) in fragment.char_indices() {
            let code = c as u32;
            if code <= 0xFFFF && !escaped_bmp(code) {
                continue;
            }
            w.write_all(&fragment.as_bytes()[start..i])?;
            let mut units = [0u16; 2];
            for unit in c.encode_utf16(&mut units) {
                write_unit(w, u32::from(*unit))?;
            }
            start = i + c.len_utf8();
        }
        w.write_all(&fragment.as_bytes()[start..])
    }

    fn write_char_escape<W: ?Sized + Write>(&mut self, w: &mut W, escape: CharEscape) -> io::Result<()> {
        match escape {
            CharEscape::Quote => w.write_all(b"\\\""),
            CharEscape::ReverseSolidus => w.write_all(b"\\\\"),
            CharEscape::Solidus => w.write_all(b"/"),
            CharEscape::Backspace => w.write_all(b"\\b"),
            CharEscape::FormFeed => w.write_all(b"\\f"),
            CharEscape::LineFeed => w.write_all(b"\\n"),
            CharEscape::CarriageReturn => w.write_all(b"\\r"),
            CharEscape::Tab => w.write_all(b"\\t"),
            CharEscape::AsciiControl(b) => write_unit(w, u32::from(b)),
        }
    }
}

/// C# 的 `double` 序列化：整數值寫成 `0`、`120`（不是 `0.0`），其他照最短表示。
pub mod double {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &f64, s: S) -> Result<S::Ok, S::Error> {
        if value.is_finite() && value.fract() == 0.0 && value.abs() < 1e15 {
            s.serialize_i64(*value as i64)
        } else {
            s.serialize_f64(*value)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
        f64::deserialize(d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize, serde::Deserialize, Default, Debug, PartialEq)]
    #[serde(rename_all = "camelCase", default)]
    struct Sample {
        display_name: String,
        #[serde(with = "double")]
        width: f64,
        items: Vec<String>,
    }

    #[test]
    fn 格式跟c_sharp一樣_縮排兩格_crlf_中文不跳脫() {
        let s = Sample { display_name: "顧 \"小\"\n明".into(), width: 0.0, items: vec![] };
        assert_eq!(
            "{\r\n  \"displayName\": \"顧 \\\"小\\\"\\n明\",\r\n  \"width\": 0,\r\n  \"items\": []\r\n}",
            to_string(&s).unwrap()
        );
        let nested = serde_json::json!({ "a": [1, { "b": {} }], "c": [] });
        assert_eq!(
            "{\r\n  \"a\": [\r\n    1,\r\n    {\r\n      \"b\": {}\r\n    }\r\n  ],\r\n  \"c\": []\r\n}",
            to_string(&nested).unwrap()
        );
        assert_eq!("{\"a\":[1,{\"b\":{}}],\"c\":[]}", to_string_compact(&nested).unwrap());
    }

    #[test]
    fn 跳脫規則跟c_sharp一樣() {
        // 期望值是 .NET 10 System.Text.Json（UnsafeRelaxedJsonEscaping）實際的輸出
        let s = "\u{0}\u{1}\u{8}\t\n\u{c}\r\u{1f}\"\\\u{7f}\u{80}\u{a0}é\u{378}\u{2028}\u{3000}\u{e000}\u{fffe}中文😀\u{10ffff}/<>&'+";
        let expected = r#""\u0000\u0001\b\t\n\f\r\u001F\"\\\u007F\u0080\u00A0é\u0378\u2028\u3000\uE000\uFFFE中文\uD83D\uDE00\uDBFF\uDFFF/<>&'+""#;
        assert_eq!(expected, to_string(s).unwrap());
        // 讀回來一樣
        assert_eq!(s, serde_json::from_str::<String>(expected).unwrap());
    }

    #[test]
    fn 存檔再讀回_先寫tmp() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        let s = Sample { display_name: "a".into(), width: 12.5, items: vec!["b".into()] };
        save(&path, &s);
        assert!(!dir.path().join("x.json.tmp").exists());
        assert_eq!(s, load::<Sample>(&path));
    }

    #[test]
    fn 讀不到或讀不懂回傳預設值() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.json");
        assert_eq!(Sample::default(), load::<Sample>(&path));
        std::fs::write(&path, "{ bad").unwrap();
        assert_eq!(Sample::default(), load::<Sample>(&path));
        std::fs::write(&path, "null").unwrap();
        assert_eq!(Sample::default(), load::<Sample>(&path));
        std::fs::write(&path, "\u{feff}{\"displayName\":\"b\"}").unwrap();
        assert_eq!("b", load::<Sample>(&path).display_name);
    }
}
