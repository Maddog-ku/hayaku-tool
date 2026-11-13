//! 對應 C# `Core/SyncPolicy.cs`：背景同步什麼時候要打哪些請求（純函式，方便測試）。

use std::collections::HashSet;

use chrono::{NaiveDate, NaiveDateTime, TimeDelta};

pub struct SyncPolicy;

impl SyncPolicy {
    /// 今天的計時列表多久一定重抓一次（網頁上改了已結束的計時也會在這段時間內反映）。
    pub const ENTRIES_INTERVAL: TimeDelta = TimeDelta::minutes(5);

    /// 連續同步失敗時，下一次自動同步要等多久（1 → 2 → 5 分鐘，之後都是 5 分鐘）。
    const BACKOFF: [TimeDelta; 3] = [TimeDelta::minutes(1), TimeDelta::minutes(2), TimeDelta::minutes(5)];

    /// 要不要重抓今天的計時列表：不是背景輪詢（動作後、使用者要求）、正在計時的跟上次不一樣、
    /// 換日、或距離上次超過 5 分鐘。
    #[allow(clippy::too_many_arguments)]
    pub fn need_entries<'a>(
        background: bool,
        running_ids: impl IntoIterator<Item = &'a str>,
        known_running_ids: impl IntoIterator<Item = &'a str>,
        today: NaiveDate,
        fetched_day: NaiveDate,
        now: NaiveDateTime,
        fetched_at: NaiveDateTime,
    ) -> bool {
        !background
            || running_ids.into_iter().collect::<HashSet<_>>() != known_running_ids.into_iter().collect::<HashSet<_>>()
            || fetched_day != today
            || now - fetched_at >= Self::ENTRIES_INTERVAL
    }

    /// 背景輪詢這一輪要不要真的同步：面板、工作台都沒開、Claude 也沒在動時，間隔放寬成兩倍
    /// （畫面上的計時時間是本機在跑，不受影響；只是網頁上的變動晚一點反映）。
    pub fn should_poll(
        ui_visible: bool,
        claude_active: bool,
        now: NaiveDateTime,
        last_polled: NaiveDateTime,
        interval: TimeDelta,
    ) -> bool {
        ui_visible || claude_active || now - last_polled >= interval * 2 - TimeDelta::seconds(1)
    }

    /// 第 failures 次失敗（從 1 開始）後要等多久。
    pub fn backoff_delay(failures: i32) -> TimeDelta {
        Self::BACKOFF[(failures - 1).clamp(0, Self::BACKOFF.len() as i32 - 1) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 對應 SyncPolicyTests

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 29).unwrap()
    }

    fn now() -> NaiveDateTime {
        today().and_hms_opt(10, 0, 0).unwrap()
    }

    fn need(background: bool, running: &[&str], known: &[&str], fetched_day: NaiveDate, minutes_ago: f64) -> bool {
        SyncPolicy::need_entries(
            background,
            running.iter().copied(),
            known.iter().copied(),
            today(),
            fetched_day,
            now(),
            now() - TimeDelta::milliseconds((minutes_ago * 60_000.0) as i64),
        )
    }

    fn need_default() -> bool {
        need(true, &["a"], &["a"], today(), 1.0)
    }

    #[test]
    fn 背景輪詢_計時沒變_不重抓列表() {
        assert!(!need_default());
    }

    #[test]
    fn 動作後或使用者要求_一定重抓() {
        assert!(need(false, &["a"], &["a"], today(), 1.0));
    }

    #[test]
    fn 有計時開始或停止_重抓() {
        assert!(need(true, &["a", "b"], &["a"], today(), 1.0));
        assert!(need(true, &[], &["a"], today(), 1.0));
        assert!(need(true, &["b"], &["a"], today(), 1.0));
    }

    #[test]
    fn 換日_重抓() {
        assert!(need(true, &["a"], &["a"], today() - TimeDelta::days(1), 1.0));
    }

    #[test]
    fn 超過5分鐘_重抓() {
        assert!(need(true, &["a"], &["a"], today(), 5.0));
    }

    #[test]
    fn 失敗退避_1_2_5分鐘() {
        for (failures, minutes) in [(1, 1), (2, 2), (3, 5), (10, 5)] {
            assert_eq!(TimeDelta::minutes(minutes), SyncPolicy::backoff_delay(failures));
        }
    }

    const POLL: TimeDelta = TimeDelta::seconds(30);

    #[test]
    fn 面板開著或claude在動_每次定時都同步() {
        assert!(SyncPolicy::should_poll(true, false, now(), now() - TimeDelta::seconds(30), POLL));
        assert!(SyncPolicy::should_poll(false, true, now(), now() - TimeDelta::seconds(30), POLL));
    }

    #[test]
    fn 都沒有_兩次定時才同步一次() {
        assert!(!SyncPolicy::should_poll(false, false, now(), now() - TimeDelta::seconds(30), POLL));
        assert!(SyncPolicy::should_poll(false, false, now(), now() - TimeDelta::seconds(60), POLL));
        assert!(SyncPolicy::should_poll(false, false, now(), now() - TimeDelta::milliseconds(59_500), POLL)); // 計時器稍微早到也算
    }
}
