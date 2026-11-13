//! 對應 C# `Slack/SlackState.cs`：傳過的訊息存在 %APPDATA%\HayakuBar\slack.json，重開程式後舊訊息的按鈕也能用；超過 7 天的不留。

use std::path::Path;

use chrono::{Local, NaiveDateTime, TimeDelta};
use hb_backend::infra::{AppPaths, json_file};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// 傳到 Slack 的一則訊息：對應哪些通知或哪筆變更申請、按鈕要做什麼，處理了沒。
/// kind：notice（一般通知，可以標為已讀、封存）、notices（一次很多則，合成一則）、review（待我審核）、
/// retest（測試結果待復驗）、fix（測試結果被退回）。
///
/// 欄位順序跟 C# 一樣（slack.json 的屬性順序）；null 照樣寫出來。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackPost {
    /// 這則訊息的代號（n:通知 id、cr:申請 id、ns:時間），按鈕的 value 就是它。
    pub key: String,
    pub kind: String,
    pub channel: String,
    pub ts: String,
    pub title: String,
    pub body: String,
    pub url: Option<String>,
    pub notification_ids: Vec<String>,
    pub change_request_id: Option<String>,

    /// 駁回新增任務的申請會刪掉那個任務，表單上要先講。
    pub reject_warning: Option<String>,

    /// 可以「核准並關注」（刪除任務的申請不行）。
    pub can_watch: bool,

    pub task_key: Option<String>,

    /// 本機提示的標記（Slack 上處理了就把那則提示關掉）。
    pub toast_tag: Option<String>,

    /// 本機時間（C# `DateTime.Now`，寫成帶時區的 `2026-11-03T10:00:00.1234567+08:00`）。
    #[serde(with = "posted_at")]
    pub posted_at: NaiveDateTime,

    /// 處理結果（例「✓ 已核准（10:32，從 Slack）」）；None＝還沒處理，訊息上還有按鈕。
    pub done: Option<String>,
}

impl Default for SlackPost {
    fn default() -> Self {
        Self {
            key: String::new(),
            kind: String::new(),
            channel: String::new(),
            ts: String::new(),
            title: String::new(),
            body: String::new(),
            url: None,
            notification_ids: Vec::new(),
            change_request_id: None,
            reject_warning: None,
            can_watch: false,
            task_key: None,
            toast_tag: None,
            posted_at: hb_eip::net_time::min_value(),
            done: None,
        }
    }
}

/// C# `DateTime` 的寫法：`DateTime.Now`（Kind=Local）帶時區；預設值 `DateTime.MinValue`（Kind=Unspecified）不帶。
/// 讀的時候有時區就換成本機時間。
mod posted_at {
    use super::*;
    use hb_eip::net_time;

    pub fn serialize<S: Serializer>(t: &NaiveDateTime, s: S) -> Result<S::Ok, S::Error> {
        if *t == net_time::min_value() {
            s.serialize_str(&net_time::format_unspecified(t))
        } else {
            s.serialize_str(&net_time::format_local(t))
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveDateTime, D::Error> {
        let text = String::deserialize(d)?;
        net_time::parse(&text).ok_or_else(|| serde::de::Error::custom(format!("無法解析的時間：{text}")))
    }
}

/// slack.json 的內容。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackState {
    pub posts: Vec<SlackPost>,
}

impl SlackState {
    /// 保留 7 天。
    pub const KEEP: TimeDelta = TimeDelta::days(7);

    /// 同一個代號找最後一則（C# `LastOrDefault`）。
    pub fn find(&self, key: &str) -> Option<&SlackPost> {
        self.posts.iter().rev().find(|p| p.key == key)
    }

    pub fn find_mut(&mut self, key: &str) -> Option<&mut SlackPost> {
        self.posts.iter_mut().rev().find(|p| p.key == key)
    }

    pub fn prune(&mut self, now: NaiveDateTime) {
        self.posts.retain(|p| now - p.posted_at <= Self::KEEP);
    }

    /// 讀 slack.json（順便清掉超過 7 天的）。
    pub fn load() -> Self {
        Self::load_from(&AppPaths::slack_state())
    }

    pub fn load_from(path: &Path) -> Self {
        let mut s: SlackState = json_file::load(path);
        s.prune(Local::now().naive_local());
        s
    }

    pub fn save(&self) {
        self.save_to(&AppPaths::slack_state());
    }

    pub fn save_to(&self, path: &Path) {
        json_file::save(path, self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    // SlackMessagesTests.超過7天的訊息_不留
    #[test]
    fn 超過7天的訊息_不留() {
        let now = at(3, 10, 0);
        let mut s = SlackState {
            posts: vec![
                SlackPost { key: "a".into(), posted_at: now - TimeDelta::days(8), ..Default::default() },
                SlackPost { key: "b".into(), posted_at: now - TimeDelta::days(1), ..Default::default() },
            ],
        };
        s.prune(now);
        assert_eq!(vec!["b"], s.posts.iter().map(|p| p.key.as_str()).collect::<Vec<_>>());
    }

    #[test]
    fn 剛好7天_還留著() {
        let now = at(10, 10, 0);
        let mut s =
            SlackState { posts: vec![SlackPost { key: "a".into(), posted_at: at(3, 10, 0), ..Default::default() }] };
        s.prune(now);
        assert_eq!(1, s.posts.len());
    }

    #[test]
    fn 同一個代號_找最後一則() {
        let s = SlackState {
            posts: vec![
                SlackPost { key: "a".into(), title: "舊".into(), ..Default::default() },
                SlackPost { key: "a".into(), title: "新".into(), ..Default::default() },
            ],
        };
        assert_eq!("新", s.find("a").unwrap().title);
        assert!(s.find("b").is_none());
    }

    /// 本機時區的偏移（例 +08:00），C# 寫 `DateTime.Now` 會帶這個。
    fn offset_of(t: NaiveDateTime) -> String {
        use chrono::TimeZone;
        Local.from_local_datetime(&t).earliest().unwrap().format("%:z").to_string()
    }

    /// C# 版（System.Text.Json、JsonFile.Options）寫出的 slack.json：縮排兩格、CRLF、camelCase、null 照寫、中文不跳脫。
    fn cs_json(posted_at: &str) -> String {
        [
            "{",
            "  \"posts\": [",
            "    {",
            "      \"key\": \"cr:a\",",
            "      \"kind\": \"review\",",
            "      \"channel\": \"D1\",",
            "      \"ts\": \"111.2\",",
            "      \"title\": \"待審核・新增任務\",",
            "      \"body\": \"Amy：F2P-2 做登入\\n• 狀態：待處理 → 進行中\",",
            "      \"url\": \"https://hayaku/projects/F2P?task=F2P-2\",",
            "      \"notificationIds\": [],",
            "      \"changeRequestId\": \"a\",",
            "      \"rejectWarning\": \"駁回會刪除 F2P-2 做登入\",",
            "      \"canWatch\": true,",
            "      \"taskKey\": \"F2P-2\",",
            "      \"toastTag\": \"cr:a\",",
            &format!("      \"postedAt\": \"{posted_at}\","),
            "      \"done\": null",
            "    },",
            "    {",
            "      \"key\": \"n:9\",",
            "      \"kind\": \"notice\",",
            "      \"channel\": \"D1\",",
            "      \"ts\": \"111.3\",",
            "      \"title\": \"留言\",",
            "      \"body\": \"小明：<b> & \\\"引號\\\"\",",
            "      \"url\": null,",
            "      \"notificationIds\": [",
            "        \"9\"",
            "      ],",
            "      \"changeRequestId\": null,",
            "      \"rejectWarning\": null,",
            "      \"canWatch\": false,",
            "      \"taskKey\": null,",
            "      \"toastTag\": \"n:9\",",
            &format!("      \"postedAt\": \"{posted_at}\","),
            "      \"done\": \"✓ 已標為已讀（10:32，從 Slack）\"",
            "    }",
            "  ]",
            "}",
        ]
        .join("\r\n")
    }

    #[test]
    fn slack_json_跟c_sharp格式相容_讀了再寫一字不差() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slack.json");
        // 用最近的時間，讀檔時才不會被清掉
        let t = Local::now().naive_local().date().and_hms_nano_opt(9, 5, 7, 123_456_700).unwrap() - TimeDelta::days(1);
        let posted = format!("{}.1234567{}", t.format("%Y-%m-%dT%H:%M:%S"), offset_of(t));
        let text = cs_json(&posted);
        std::fs::write(&path, &text).unwrap();

        let s = SlackState::load_from(&path);
        assert_eq!(2, s.posts.len());
        let review = &s.posts[0];
        assert_eq!(
            ("cr:a", "review", "D1", "111.2"),
            (review.key.as_str(), review.kind.as_str(), review.channel.as_str(), review.ts.as_str())
        );
        assert_eq!(Some("a"), review.change_request_id.as_deref());
        assert_eq!(Some("駁回會刪除 F2P-2 做登入"), review.reject_warning.as_deref());
        assert!(review.can_watch);
        assert_eq!(t, review.posted_at);
        assert_eq!(None, review.done);
        let notice = &s.posts[1];
        assert_eq!(vec!["9"], notice.notification_ids);
        assert_eq!(None, notice.url);
        assert_eq!("小明：<b> & \"引號\"", notice.body);
        assert_eq!(Some("✓ 已標為已讀（10:32，從 Slack）"), notice.done.as_deref());

        // 再存回去：跟 C# 寫的一字不差（C# 版讀得懂）
        s.save_to(&path);
        assert_eq!(text, std::fs::read_to_string(&path).unwrap());
        assert!(!dir.path().join("slack.json.tmp").exists());
    }

    #[test]
    fn slack_json_缺欄位用預設值_沒檔案是空的() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slack.json");
        assert!(SlackState::load_from(&path).posts.is_empty());
        let t = Local::now().naive_local() - TimeDelta::hours(1);
        let posted = format!("{}{}", t.format("%Y-%m-%dT%H:%M:%S%.f"), offset_of(t));
        std::fs::write(&path, format!("{{\"posts\":[{{\"key\":\"n:1\",\"postedAt\":\"{posted}\"}}]}}")).unwrap();
        let s = SlackState::load_from(&path);
        assert_eq!(1, s.posts.len());
        let p = &s.posts[0];
        assert_eq!(("n:1", "", false), (p.key.as_str(), p.ts.as_str(), p.can_watch));
        assert!(p.notification_ids.is_empty() && p.done.is_none() && p.url.is_none());
    }

    #[test]
    fn 讀檔時清掉超過7天的() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slack.json");
        std::fs::write(
            &path,
            "{\"posts\":[{\"key\":\"old\",\"postedAt\":\"2020-01-01T09:00:00+08:00\"},{\"key\":\"min\"}]}",
        )
        .unwrap();
        assert!(SlackState::load_from(&path).posts.is_empty());
    }

    #[test]
    fn 沒有設定時間_寫成c_sharp的min_value() {
        let text = json_file::to_string_compact(&SlackPost::default()).unwrap();
        assert!(text.contains("\"postedAt\":\"0001-01-01T00:00:00\""), "{text}");
    }
}
