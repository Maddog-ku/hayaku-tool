//! 對應 C# `Mcp/McpStats.cs`：統計送到 Hayaku /mcp 的請求數（含 initialize 與 token 失效後的重送），
//! 每小時寫一行到紀錄，用來確認同步打了多少請求。

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use chrono::{Local, NaiveDateTime};

const WINDOW: Duration = Duration::from_secs(3600);

struct Stats {
    counts: HashMap<String, i32>,
    since: Option<NaiveDateTime>,
}

static SYNC: LazyLock<Mutex<Stats>> = LazyLock::new(|| Mutex::new(Stats { counts: HashMap::new(), since: None }));

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

fn elapsed(since: NaiveDateTime, now: NaiveDateTime) -> Duration {
    (now - since).to_std().unwrap_or_default()
}

fn with_stats<R>(f: impl FnOnce(&mut Stats, NaiveDateTime) -> R) -> R {
    let mut stats = SYNC.lock().unwrap_or_else(|e| e.into_inner());
    let now = now();
    // C# 是類別初始化時開始算；這裡是第一次用到時
    stats.since.get_or_insert(now);
    f(&mut stats, now)
}

/// 記一次請求（tools/call 用工具名稱，其他用方法名稱）；滿一小時就寫統計並重新計算。
pub fn record(name: &str) {
    let line = with_stats(|stats, now| {
        *stats.counts.entry(name.to_owned()).or_insert(0) += 1;
        let since = stats.since.unwrap_or(now);
        (elapsed(since, now) >= WINDOW).then(|| take_locked(stats, now))
    });
    if let Some(line) = line {
        log::info!("{line}");
    }
}

/// 目前累計（這一小時內）的請求數，給設定頁看；還沒有請求時說明一下。
pub fn current() -> String {
    with_stats(|stats, now| {
        if stats.counts.is_empty() {
            "這一小時還沒有送出請求".to_owned()
        } else {
            format(&stats.counts, elapsed(stats.since.unwrap_or(now), now))
        }
    })
}

/// 把目前累計的寫進紀錄（例如結束程式時）；沒有請求就不寫。
pub fn flush() {
    let line = with_stats(|stats, now| (!stats.counts.is_empty()).then(|| take_locked(stats, now)));
    if let Some(line) = line {
        log::info!("{line}");
    }
}

fn take_locked(stats: &mut Stats, now: NaiveDateTime) -> String {
    let line = format(&stats.counts, elapsed(stats.since.unwrap_or(now), now));
    stats.counts.clear();
    stats.since = Some(now);
    line
}

/// "MCP 請求（過去 60 分鐘）：共 412 次 · list_time_entries 120、get_running_timers 120…"（多的排前面）。
pub fn format(counts: &HashMap<String, i32>, elapsed: Duration) -> String {
    let total: i32 = counts.values().sum();
    let mut pairs: Vec<(&String, &i32)> = counts.iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    let detail: Vec<String> = pairs.iter().map(|(k, v)| format!("{k} {v}")).collect();
    // Math.Round 是四捨六入五成雙
    let minutes = ((elapsed.as_secs_f64() / 60.0).round_ties_even() as i64).max(1);
    format!("MCP 請求（過去 {minutes} 分鐘）：共 {total} 次 · {}", detail.join("、"))
}

/// 程式啟動時呼叫，從現在開始算第一個小時（C# 是類別初始化時；不呼叫的話從第一次用到時開始）。
pub fn start() {
    with_stats(|_, _| ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_多的排前面_同數依名稱() {
        let counts: HashMap<String, i32> =
            [("list_notifications", 30), ("get_running_timers", 120), ("list_time_entries", 120)]
                .into_iter()
                .map(|(k, v)| (k.to_owned(), v))
                .collect();
        assert_eq!(
            "MCP 請求（過去 60 分鐘）：共 270 次 · get_running_timers 120、list_time_entries 120、list_notifications 30",
            format(&counts, Duration::from_secs(3600))
        );
    }

    #[test]
    fn format_不到一分鐘算一分鐘() {
        let counts: HashMap<String, i32> = [("whoami".to_owned(), 1)].into_iter().collect();
        assert!(format(&counts, Duration::from_secs(5)).starts_with("MCP 請求（過去 1 分鐘）：共 1 次"));
    }
}
