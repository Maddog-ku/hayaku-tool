//! 對應 C# `Infra/Housekeeping.cs`：定期清暫存：刪掉舊的截圖與紀錄、整理記憶體（把用不到的記憶體還給 Windows）。

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Local, NaiveDateTime, TimeDelta};

use super::log as hblog;

const SHOTS_KEEP_DAYS: i64 = 3;
const LOG_KEEP_DAYS: i64 = 60;
const TEMP_KEEP_DAYS: i64 = 1;

/// 舊紀錄合計的上限；萬一某段時間紀錄特別多，超過就從最舊的刪。
const LOGS_MAX_BYTES: u64 = 50_000_000;

/// 檔案的最後修改時間（本機時間），跟 C# `FileInfo.LastWriteTime` 一樣。
fn last_write_time(time: SystemTime) -> NaiveDateTime {
    DateTime::<Local>::from(time).naive_local()
}

struct Entry {
    path: PathBuf,
    name: String,
    len: u64,
    modified: NaiveDateTime,
}

fn files(dir: &Path, matches: impl Fn(&str) -> bool) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    read.filter_map(Result::ok)
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !matches(&name) {
                return None;
            }
            Some(Entry { path: e.path(), name, len: meta.len(), modified: last_write_time(meta.modified().ok()?) })
        })
        .collect()
}

/// 刪掉 3 天前的截圖（--shot 產生的）、2 個月前的紀錄（logs\，合計超過 50MB 也從最舊的刪）、
/// 寫設定時中斷留下的 .tmp；舊版的 log.txt、log.txt.1 依第一筆的時間改名搬進 logs\。回傳刪了幾個檔。
pub fn clean_files(dir: &Path, now: NaiveDateTime) -> usize {
    let mut deleted = 0;
    let mut delete = |p: &Path| {
        // 正在用的檔案下次再刪
        if std::fs::remove_file(p).is_ok() {
            deleted += 1;
        }
    };
    for f in files(&dir.join("shots"), |_| true) {
        if now - f.modified > TimeDelta::days(SHOTS_KEEP_DAYS) {
            delete(&f.path);
        }
    }
    for f in files(dir, |n| hblog::matches_pattern(n, "", ".tmp")) {
        if now - f.modified > TimeDelta::days(TEMP_KEEP_DAYS) {
            delete(&f.path);
        }
    }

    let logs_dir = dir.join("logs");
    hblog::exclusive(|| {
        for name in ["log.txt.1", "log.txt"] {
            let legacy = dir.join(name);
            if !legacy.is_file() {
                continue;
            }
            // 失敗的話下次再搬
            let _ = (|| -> std::io::Result<()> {
                std::fs::create_dir_all(&logs_dir)?;
                let at = match hblog::first_time(&legacy) {
                    Some(t) => t,
                    None => last_write_time(std::fs::metadata(&legacy)?.modified()?),
                };
                std::fs::rename(&legacy, hblog::free_path(&logs_dir, at))
            })();
        }
        if !logs_dir.is_dir() {
            return;
        }
        let mut logs = files(&logs_dir, |n| hblog::matches_pattern(n, "", ".log"));
        logs.sort_by(|a, b| b.name.cmp(&a.name));
        let mut total: u64 = 0;
        for f in logs {
            total += f.len;
            if now - f.modified > TimeDelta::days(LOG_KEEP_DAYS) || total > LOGS_MAX_BYTES {
                delete(&f.path);
            }
        }
    });
    deleted
}

/// 把閒置的實體記憶體還給 Windows（之後用到會自動拿回來）。
/// （C# 版先壓縮 GC heap；Rust 沒有 GC，只做 EmptyWorkingSet。）
pub fn trim_memory() {
    use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
    use windows::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
    // SAFETY: GetCurrentProcess 回傳的是虛擬 handle，不用關閉
    unsafe {
        let process = GetCurrentProcess();
        if let Err(err) = EmptyWorkingSet(process) {
            // 舊系統的備用做法：工作集上下限都設 -1 也會把工作集清空
            if SetProcessWorkingSetSize(process, usize::MAX, usize::MAX).is_err() {
                log::error!("釋放記憶體失敗: {err}");
            }
        }
    }
}

/// 目前佔用的記憶體（工作集、私有），給紀錄用。
/// （C# 版最後還有「GC heap N MB」；Rust 沒有 GC heap，略去這段。）
pub fn memory_text() -> String {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut counters = PROCESS_MEMORY_COUNTERS_EX {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        ..Default::default()
    };
    // SAFETY: counters 的大小填在 cb，指標指向本地變數
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    };
    let (working, private) = if ok.is_ok() { (counters.WorkingSetSize, counters.PrivateUsage) } else { (0, 0) };
    format!("工作集 {} MB、私有 {} MB", working / 1_048_576, private / 1_048_576)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, TimeZone};

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, mo, d).unwrap().and_hms_opt(h, mi, s).unwrap()
    }

    fn touch(path: &Path, at: NaiveDateTime, text: &str) -> PathBuf {
        std::fs::write(path, text).unwrap();
        set_mtime(path, at);
        path.to_path_buf()
    }

    fn set_mtime(path: &Path, at: NaiveDateTime) {
        let local = Local.from_local_datetime(&at).single().unwrap();
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::from(local)).unwrap();
    }

    #[test]
    fn clean_files_只刪舊截圖_設定不動() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 28, 15, 0, 0);
        let shots = dir.path().join("shots");
        std::fs::create_dir_all(&shots).unwrap();
        let old_shot = touch(&shots.join("old.png"), now - TimeDelta::days(4), "x");
        let new_shot = touch(&shots.join("new.png"), now - TimeDelta::days(1), "x");
        let config = touch(&dir.path().join("config.json"), now - TimeDelta::days(30), "x");

        assert_eq!(1, clean_files(dir.path(), now));
        assert!(!old_shot.exists());
        assert!(new_shot.exists());
        assert!(config.exists()); // 設定不會動
    }

    #[test]
    fn clean_files_紀錄保留2個月() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 28, 15, 0, 0);
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let expired = touch(&logs.join("20260827090000.log"), now - TimeDelta::days(61), "x");
        let kept = touch(&logs.join("20260901090000.log"), now - TimeDelta::days(57), "x");

        assert_eq!(1, clean_files(dir.path(), now));
        assert!(!expired.exists());
        assert!(kept.exists());
    }

    #[test]
    fn clean_files_紀錄合計超過50mb從最舊的刪() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 28, 15, 0, 0);
        let logs = dir.path().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        let big = vec![b'x'; 26_000_000];
        let older = logs.join("20261020090000.log");
        let newer = logs.join("20261027090000.log");
        std::fs::write(&older, &big).unwrap();
        std::fs::write(&newer, &big).unwrap();
        set_mtime(&older, now - TimeDelta::days(8));
        set_mtime(&newer, now - TimeDelta::days(1));

        assert_eq!(1, clean_files(dir.path(), now));
        assert!(!older.exists());
        assert!(newer.exists());
    }

    #[test]
    fn clean_files_舊版的log_txt依第一筆時間搬進logs() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 28, 15, 0, 0);
        let legacy = touch(
            &dir.path().join("log.txt"),
            now - TimeDelta::hours(1),
            "2026-10-20 09:00:00 [INFO] HayakuBar 啟動\n",
        );
        let legacy_old = touch(
            &dir.path().join("log.txt.1"),
            now - TimeDelta::days(9),
            "2026-10-13 08:30:05 [INFO] HayakuBar 啟動\n",
        );

        clean_files(dir.path(), now);
        assert!(!legacy.exists());
        assert!(!legacy_old.exists());
        assert!(dir.path().join("logs").join("20261020090000.log").exists());
        assert!(dir.path().join("logs").join("20261013083005.log").exists());
    }

    #[test]
    fn clean_files_刪掉寫入中斷留下的舊tmp() {
        let dir = tempfile::tempdir().unwrap();
        let now = at(2026, 10, 28, 15, 0, 0);
        let old_tmp = touch(&dir.path().join("config.json.tmp"), now - TimeDelta::days(2), "{}");
        let new_tmp = touch(&dir.path().join("state.json.tmp"), now - TimeDelta::hours(1), "{}");

        assert_eq!(1, clean_files(dir.path(), now));
        assert!(!old_tmp.exists());
        assert!(new_tmp.exists());
    }

    #[test]
    fn clean_files_沒有截圖資料夾也不會出錯() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(0, clean_files(dir.path(), Local::now().naive_local()));
    }

    #[test]
    fn trim_memory_可以重複呼叫() {
        trim_memory();
        trim_memory();
        assert!(memory_text().contains("工作集"));
    }
}
