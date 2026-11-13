//! 對應 C# `Infra/Log.cs`，實作 `log` crate 的 Logger。
//!
//! 紀錄寫在 logs\ 裡，一天一個檔，檔名是那個檔第一筆的時間（20261029093301.log），依日期就能找到。
//! 同一天超過 5MB 會再開一個新檔；舊檔由 Housekeeping 保留 2 個月。
//!
//! 用法：程式啟動時呼叫一次 [`init`]，之後各 crate 用 `log::info!` / `log::error!` 寫紀錄。
//! 只收 HayakuBar 自己的 crate（target 以 `hb_`、`hayakubar` 開頭）的 INFO 以上，
//! 第三方套件（reqwest、hyper…）的紀錄不寫進檔案（C# 版也只有自己的紀錄）。

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::{Local, NaiveDate, NaiveDateTime};

use super::app_paths::AppPaths;

/// 單一紀錄檔的上限。
pub const MAX_BYTES: u64 = 5_000_000;
const NAME_FORMAT: &str = "%Y%m%d%H%M%S";

struct State {
    path: Option<PathBuf>,
    day: Option<NaiveDate>,
}

static GATE: Mutex<State> = Mutex::new(State { path: None, day: None });

fn gate() -> MutexGuard<'static, State> {
    GATE.lock().unwrap_or_else(|e| e.into_inner())
}

struct FileLogger;

static LOGGER: FileLogger = FileLogger;

/// 安裝成 `log` crate 的全域 Logger（重複呼叫沒有作用）。
pub fn init() {
    if ::log::set_logger(&LOGGER).is_ok() {
        ::log::set_max_level(::log::LevelFilter::Info);
    }
}

fn own_target(target: &str) -> bool {
    target.starts_with("hb_") || target.starts_with("hayakubar")
}

impl ::log::Log for FileLogger {
    fn enabled(&self, metadata: &::log::Metadata) -> bool {
        metadata.level() <= ::log::Level::Info && own_target(metadata.target())
    }

    fn log(&self, record: &::log::Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            ::log::Level::Error => "ERROR",
            ::log::Level::Warn => "WARN",
            _ => "INFO",
        };
        write(level, &record.args().to_string());
    }

    fn flush(&self) {}
}

/// 直接寫一筆 INFO（不經過 `log` crate）。
pub fn info(message: &str) {
    write("INFO", message);
}

/// 直接寫一筆 ERROR；有錯誤物件時寫成「訊息: 錯誤」（C# `Log.Error(message, ex)`）。
pub fn error(message: &str, err: Option<&dyn std::fmt::Display>) {
    match err {
        Some(e) => write("ERROR", &format!("{message}: {e}")),
        None => write("ERROR", message),
    }
}

fn write(level: &str, message: &str) {
    // 記錄失敗不能影響主流程
    let _ = try_write(level, message);
}

fn try_write(level: &str, message: &str) -> std::io::Result<()> {
    let mut state = gate();
    let now = Local::now().naive_local();
    let today = now.date();
    let full =
        state.path.as_ref().and_then(|p| std::fs::metadata(p).ok()).is_some_and(|m| m.is_file() && m.len() > MAX_BYTES);
    if state.path.is_none() || state.day != Some(today) || full {
        let reuse = state.path.is_none();
        state.path = Some(pick_file(&AppPaths::logs(), now, reuse)?);
        state.day = Some(today);
    }
    let Some(path) = state.path.as_ref() else {
        return Ok(());
    };
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(format!("{} [{level}] {message}\r\n", now.format("%Y-%m-%d %H:%M:%S")).as_bytes())
}

/// 要寫哪個檔。`reuse_today`（程式剛啟動）：今天已經有、還沒滿的檔就接著寫；否則用現在的時間開新檔。
pub fn pick_file(logs_dir: &Path, now: NaiveDateTime, reuse_today: bool) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(logs_dir)?;
    if reuse_today {
        let prefix = now.format("%Y%m%d").to_string();
        let latest = std::fs::read_dir(logs_dir)?
            .filter_map(Result::ok)
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let meta = e.metadata().ok()?;
                (meta.is_file() && matches_pattern(&name, &prefix, ".log") && meta.len() <= MAX_BYTES).then_some(name)
            })
            .max();
        if let Some(name) = latest {
            return Ok(logs_dir.join(name));
        }
    }
    Ok(free_path(logs_dir, now))
}

/// logs\20261029093301.log；同一秒已經有了就加 -2、-3…。
pub fn free_path(logs_dir: &Path, at: NaiveDateTime) -> PathBuf {
    let stem = at.format(NAME_FORMAT).to_string();
    let mut path = logs_dir.join(format!("{stem}.log"));
    let mut n = 2;
    while path.exists() {
        path = logs_dir.join(format!("{stem}-{n}.log"));
        n += 1;
    }
    path
}

/// 跟寫紀錄互斥地執行（搬移、刪除紀錄檔時用）。
pub fn exclusive<R>(action: impl FnOnce() -> R) -> R {
    let _guard = gate();
    action()
}

/// 紀錄檔第一行的時間（"2026-10-28 09:39:24 [INFO] …"）；沒有檔案或讀不懂回傳 None。
pub fn first_time(path: &Path) -> Option<NaiveDateTime> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; 256];
    let n = file.read(&mut buf).ok()?;
    let bytes = &buf[..n];
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = String::from_utf8_lossy(bytes);
    let line = text.split(['\r', '\n']).next()?;
    let head: String = line.chars().take(19).collect();
    if head.chars().count() < 19 {
        return None;
    }
    NaiveDateTime::parse_from_str(&head, "%Y-%m-%d %H:%M:%S").ok()
}

/// Windows 檔名萬用字元「{prefix}*{suffix}」（不分大小寫）。
pub(crate) fn matches_pattern(name: &str, prefix: &str, suffix: &str) -> bool {
    let lower = name.to_lowercase();
    lower.len() >= prefix.len() + suffix.len()
        && lower.starts_with(&prefix.to_lowercase())
        && lower.ends_with(&suffix.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, s).unwrap()
    }

    #[test]
    fn pick_file_啟動時接著寫今天的檔_換日或滿了開新檔() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 29, 9, 33, 1);
        let today = dir.path().join("20261029080000.log");
        std::fs::write(&today, "x").unwrap();
        std::fs::write(dir.path().join("20261028170000.log"), "x").unwrap();

        assert_eq!(today, pick_file(dir.path(), now, true).unwrap());
        assert_eq!(dir.path().join("20261029093301.log"), pick_file(dir.path(), now, false).unwrap());
    }

    #[test]
    fn pick_file_今天的檔滿了就開新檔() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 29, 9, 33, 1);
        let full = dir.path().join("20261029080000.log");
        std::fs::write(&full, vec![b'x'; (MAX_BYTES + 1) as usize]).unwrap();
        assert_eq!(dir.path().join("20261029093301.log"), pick_file(dir.path(), now, true).unwrap());
    }

    #[test]
    fn free_path_同一秒重複就加編號() {
        let dir = tempfile::tempdir().unwrap();
        let t = at(2026, 10, 29, 9, 33, 1);
        let first = free_path(dir.path(), t);
        assert_eq!(dir.path().join("20261029093301.log"), first);
        std::fs::write(&first, "x").unwrap();
        assert_eq!(dir.path().join("20261029093301-2.log"), free_path(dir.path(), t));
    }

    #[test]
    fn first_time_讀第一行的時間() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("log.txt");
        std::fs::write(&p, "2026-10-20 09:00:00 [INFO] HayakuBar 啟動\r\n2026-10-21 09:00:00 [INFO] x\r\n").unwrap();
        assert_eq!(Some(at(2026, 10, 20, 9, 0, 0)), first_time(&p));
        std::fs::write(&p, "壞掉的內容").unwrap();
        assert_eq!(None, first_time(&p));
        assert_eq!(None, first_time(&dir.path().join("none.txt")));
    }

    #[test]
    fn 寫紀錄_格式跟c_sharp一樣() {
        let _guard = crate::infra::app_paths::tests::lock();
        let dir = tempfile::tempdir().unwrap();
        AppPaths::set_dir(Some(dir.path()));
        gate().path = None;
        info("HayakuBar 啟動");
        error("讀取失敗", Some(&"壞了"));
        let files: Vec<_> = std::fs::read_dir(dir.path().join("logs")).unwrap().filter_map(Result::ok).collect();
        assert_eq!(1, files.len());
        let name = files[0].file_name().to_string_lossy().into_owned();
        assert!(name.len() == 18 && name.ends_with(".log"), "{name}");
        let text = std::fs::read_to_string(files[0].path()).unwrap();
        let lines: Vec<&str> = text.split("\r\n").collect();
        assert_eq!(3, lines.len());
        assert!(NaiveDateTime::parse_from_str(&lines[0][..19], "%Y-%m-%d %H:%M:%S").is_ok());
        assert_eq!(" [INFO] HayakuBar 啟動", &lines[0][19..]);
        assert_eq!(" [ERROR] 讀取失敗: 壞了", &lines[1][19..]);
        assert_eq!("", lines[2]);
        gate().path = None;
        AppPaths::set_dir(None);
    }
}
