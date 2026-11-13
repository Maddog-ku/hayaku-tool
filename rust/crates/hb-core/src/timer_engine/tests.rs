//! TimerEngine 的測試：MeetingPauseTests、TaskExtrasTests／TestingTests 的 NotificationTarget，
//! 以及用假的 MCP transport 跑幾個主要流程（確認沒有 RefCell 重複借用、錯誤訊息跟 C# 一樣）。

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use hb_api::{HayakuApi, NotificationItem};
use hb_backend::auth::OAuthClient;
use hb_backend::config::{AppConfig, AppState};
use hb_backend::infra::AppPaths;
use hb_claude::ClaudeMonitor;

use super::*;
use crate::test_support::{FakeTransport, block_on, j};

fn keys(list: &[&str]) -> CiSet {
    list.iter().copied().collect()
}

// ───────────── MeetingPauseTests ─────────────

#[test]
fn 會議本身在計時_其他加入前在跑的都要暫停() {
    let (meeting, others) =
        TimerEngine::split_for_meeting("REVIVAL-9", &keys(&["F2P-1", "EG-3"]), &keys(&["F2P-1", "EG-3", "REVIVAL-9"]));
    assert_eq!(vec!["REVIVAL-9"], meeting);
    assert_eq!(vec!["EG-3", "F2P-1"], others);
}

#[test]
fn 找不到會議的key_加入後才出現的當成會議() {
    let (meeting, others) =
        TimerEngine::split_for_meeting("REVIVAL-9", &keys(&["F2P-1"]), &keys(&["F2P-1", "REVIVAL-M9"]));
    assert_eq!(vec!["REVIVAL-M9"], meeting);
    assert_eq!(vec!["F2P-1"], others);
}

#[test]
fn 加入前會議就在計時_不會被暫停() {
    let (meeting, others) =
        TimerEngine::split_for_meeting("revival-9", &keys(&["REVIVAL-9", "F2P-1"]), &keys(&["REVIVAL-9", "F2P-1"]));
    assert_eq!(vec!["REVIVAL-9"], meeting);
    assert_eq!(vec!["F2P-1"], others);
}

#[test]
fn 加入前沒有計時_沒有要暫停的() {
    let (_, others) = TimerEngine::split_for_meeting("REVIVAL-9", &keys(&[]), &keys(&["REVIVAL-9"]));
    assert!(others.is_empty());
}

// ───────────── NotificationTarget ─────────────

fn notice(r#type: &str, task: Option<&str>, meeting: Option<&str>) -> NotificationItem {
    NotificationItem::new(
        "n1",
        r#type,
        "",
        "",
        None,
        task.map(str::to_owned),
        Some("F2P".to_owned()),
        meeting.map(str::to_owned),
        crate::time::now_offset(),
        true,
        1,
    )
}

fn text(s: &str) -> Option<WorkspaceArg> {
    Some(WorkspaceArg::text(s))
}

// 對應 TaskExtrasTests.通知_在工作台對應的頁打開
#[test]
fn 通知_在工作台對應的頁打開() {
    assert_eq!(
        (Some("reviews"), None),
        TimerEngine::notification_target(&notice("CHANGE_REQUESTED", Some("F2P-1"), None))
    );
    assert_eq!(
        (Some("tasks"), text("F2P-1")),
        TimerEngine::notification_target(&notice("STATUS_CHANGED", Some("F2P-1"), None))
    );
    assert_eq!(
        (Some("meetings"), text("m1")),
        TimerEngine::notification_target(&notice("MEETING_INVITED", None, Some("m1")))
    );
    assert_eq!((Some("hours"), None), TimerEngine::notification_target(&notice("HOURS_UNFILLED", None, None)));
    assert_eq!((None, None), TimerEngine::notification_target(&notice("SECURITY_ALERT", None, None)));
}

// 對應 TestingTests.ParseNotifications_測試結果與問題回報 的 NotificationTarget 部分（解析在 hb-api 也測過）
#[test]
fn parse_notifications_問題回報對應到工作台() {
    let list = HayakuApi::parse_notifications(&j(r#"
        {"items":[
          {"latest":{"id":"n1","type":"TEST_RESULT_REOPENED","project":{"key":"F2P"},"task":{"number":140,"title":"登入頁跑版"},"readAt":null}},
          {"latest":{"id":"n2","type":"TEST_RESULT_RETEST","project":{"key":"F2P"},"task":{"number":141,"title":"按鈕沒反應"},"readAt":null}},
          {"latest":{"id":"n3","type":"FEEDBACK_NEW","payload":{"number":128,"title":"匯出失敗"},"readAt":null}}
        ]}"#));
    assert!(list[0].is_reopened());
    assert_eq!((Some("F2P-140"), "測試結果被退回"), (list[0].task_key.as_deref(), list[0].label.as_str()));
    assert!(list[1].is_ready_to_retest());
    assert_eq!("測試結果待復驗", list[1].label);
    assert_eq!((Some(128), "匯出失敗"), (list[2].feedback, list[2].text.as_str()));
    assert_eq!((Some("feedback"), Some(WorkspaceArg::Number(128))), TimerEngine::notification_target(&list[2]));
}

// ───────────── 用假的 transport 跑流程 ─────────────

static APP_DIR_LOCK: Mutex<()> = Mutex::new(());

/// 改 AppPaths 到暫存資料夾（不碰真正的 %APPDATA%\HayakuBar）；測試結束時還原。
struct TempApp {
    _lock: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
}

impl TempApp {
    fn new() -> Self {
        let lock = APP_DIR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        AppPaths::set_dir(Some(dir.path()));
        Self { _lock: lock, dir }
    }

    fn path(&self) -> &Path {
        self.dir.path()
    }
}

impl Drop for TempApp {
    fn drop(&mut self) {
        AppPaths::set_dir(None);
    }
}

struct Harness {
    engine: TimerEngine,
    fake: Arc<FakeTransport>,
    queued: Rc<RefCell<Vec<LocalFuture>>>,
    notices: Rc<RefCell<Vec<String>>>,
    toasts: Rc<RefCell<Vec<ToastRequest>>>,
    _app: TempApp,
}

impl Harness {
    fn new() -> Self {
        let app = TempApp::new();
        let fake = Arc::new(FakeTransport::default());
        let api = Arc::new(HayakuApi::new(fake.clone()));
        let auth = OAuthClient::with_store(
            hb_backend::http::app_client(),
            "https://example.invalid",
            &app.path().join("auth.bin"),
        );
        let config = AppConfig { auto_schedule: false, claude_enabled: false, ..AppConfig::default() };
        let engine = TimerEngine::new(
            Rc::new(RefCell::new(config)),
            Rc::new(RefCell::new(AppState::default())),
            auth,
            api,
            ClaudeMonitor::new(),
        );
        let queued: Rc<RefCell<Vec<LocalFuture>>> = Rc::default();
        let q = queued.clone();
        engine.set_spawner(Rc::new(move |f| q.borrow_mut().push(f)));
        let notices: Rc<RefCell<Vec<String>>> = Rc::default();
        let n = notices.clone();
        engine.events().notify.subscribe(move |m| n.borrow_mut().push(m.clone()));
        let toasts: Rc<RefCell<Vec<ToastRequest>>> = Rc::default();
        let t = toasts.clone();
        engine.events().toast.subscribe(move |r| t.borrow_mut().push(r.clone()));
        Self { engine, fake, queued, notices, toasts, _app: app }
    }

    /// 把 fire-and-forget 排進來的工作跑完。
    fn drain(&self) {
        loop {
            let next = self.queued.borrow_mut().pop();
            match next {
                Some(f) => block_on(f),
                None => break,
            }
        }
    }
}

fn running_entry(id: &str, task: &str) -> String {
    format!(
        r#"{{"id":"{id}","task":"{task}","title":"{task} 名稱","startedAt":"2026-11-12T01:00:00Z","endedAt":null,"running":true}}"#
    )
}

#[test]
fn 同步一輪_讀到使用者計時與清單() {
    let h = Harness::new();
    h.fake.on("whoami", j(r#"{"username":"harry.gu","displayName":"古信汯"}"#));
    h.fake.on("get_running_timers", j(&format!(r#"{{"running":[{}]}}"#, running_entry("e1", "F2P-5"))));
    h.fake.on("list_time_entries", j(r#"{"entries":[]}"#));
    h.fake.on("list_projects", j(r#"{"projects":[{"key":"F2P","name":"列印平台","role":"ADMIN"}]}"#));
    block_on(h.engine.refresh(true));
    h.drain();
    assert_eq!(None, h.engine.error());
    assert!(h.engine.logged_in());
    assert_eq!("古信汯", h.engine.user().unwrap().display_name);
    assert_eq!(vec!["F2P-5".to_owned()], h.engine.running_keys().to_vec());
    assert!(h.engine.can_review());
    assert!(h.engine.last_synced().is_some());
    assert!(!h.engine.syncing());
    // 帳號記在 state，不上班的日子不問 Hayaku 也能顯示
    assert_eq!(Some("harry.gu"), h.engine.state().user_name.as_deref());
    assert!(h.fake.tools().contains(&"get_running_timers".to_owned()));
}

#[test]
fn 需要登入時_顯示未登入與原因() {
    let h = Harness::new();
    h.fake.fail("whoami", "需要登入 Hayaku");
    block_on(h.engine.refresh(true));
    assert!(!h.engine.logged_in());
    assert_eq!(Some("需要登入 Hayaku".to_owned()), h.engine.error());
}

#[test]
fn 托盤滿了_不送請求並顯示原因() {
    let h = Harness::new();
    let pins: Vec<String> = (1..=20).map(|i| format!(r#"{{"task":"F2P-{i}","title":"t"}}"#)).collect();
    h.fake.on("list_pins", j(&format!(r#"{{"pins":[{}]}}"#, pins.join(","))));
    h.fake.on("whoami", j(r#"{"username":"a","displayName":"A"}"#));
    block_on(h.engine.refresh(true));
    assert_eq!(TimerEngine::MAX_PINS, h.engine.pins().len());
    let before = h.fake.calls().len();
    block_on(h.engine.pin_task("F2P-99"));
    assert_eq!(Some("托盤最多釘 20 個任務，先取消一個再釘".to_owned()), h.engine.error());
    assert_eq!(before, h.fake.calls().len());
}

#[test]
fn 下班_暫停所有計時並跳出可以復原的提示() {
    let h = Harness::new();
    h.fake.on("whoami", j(r#"{"username":"a","displayName":"A"}"#));
    h.fake.on(
        "get_running_timers",
        j(&format!(r#"{{"running":[{},{}]}}"#, running_entry("e1", "F2P-5"), running_entry("e2", "EG-3"))),
    );
    block_on(h.engine.clock_out());
    assert_eq!(None, h.engine.error());
    assert!(h.engine.state().clocked_out);
    assert_eq!(vec!["EG-3".to_owned(), "F2P-5".to_owned()], h.engine.state().resume_set);
    let calls = h.fake.calls();
    assert!(calls.iter().any(|(name, args)| name == "pause_timer" && *args == serde_json::json!({})));
    let toasts = h.toasts.borrow();
    let toast = toasts.iter().find(|t| t.title == "已下班").expect("下班提示");
    assert!(toast.message.starts_with("停止 2 個計時。今天 "));
    assert_eq!("復原", toast.actions[0].label);
    assert!(toast.actions[0].primary);
}

#[test]
fn 移動工時_正在計時的不能移() {
    let h = Harness::new();
    let entry = hb_api::TimeEntry::new("e1", "F2P-1", "x", Some(crate::time::now_offset()), None, None, true, None);
    let err = block_on(h.engine.move_time_entry(&entry, "F2P-2")).unwrap_err();
    assert_eq!("正在計時的那段不能移，先暫停再移", err.message());
    let mut done = entry.clone();
    done.running = false;
    done.ended_at = Some(crate::time::now_offset());
    let err = block_on(h.engine.move_time_entry(&done, " f2p-1 ")).unwrap_err();
    assert_eq!("這段已經是 F2P-1 的工時", err.message());
    assert!(h.fake.calls().is_empty());
}

#[test]
fn 加入會議_暫停其他計時_離開後恢復() {
    let h = Harness::new();
    h.engine.config_mut().meeting_pauses_others = true;
    h.fake.on("whoami", j(r#"{"username":"a","displayName":"A"}"#));
    h.fake.on("get_running_timers", j(&format!(r#"{{"running":[{}]}}"#, running_entry("e1", "F2P-5"))));
    block_on(h.engine.refresh(true));
    // 加入後：會議與原本的都在跑
    h.fake.on(
        "get_running_timers",
        j(&format!(r#"{{"running":[{},{}]}}"#, running_entry("e1", "F2P-5"), running_entry("e2", "REVIVAL-9"))),
    );
    let meeting = hb_api::MeetingItem {
        key: "REVIVAL-9".into(),
        id: "m1".into(),
        title: "週會".into(),
        project: "REVIVAL".into(),
        start_at: crate::time::now_offset(),
        end_at: crate::time::now_offset(),
        location: None,
        my_status: None,
    };
    block_on(h.engine.join_meeting(&meeting));
    assert_eq!(None, h.engine.error());
    let pause = h.engine.state().meeting_pause.clone().expect("記下暫停的");
    assert_eq!(vec!["REVIVAL-9".to_owned()], pause.meeting_tasks);
    assert_eq!(vec!["F2P-5".to_owned()], pause.tasks);
    assert!(h.notices.borrow().iter().any(|n| n == "已加入會議並開始計時：週會，暫停 F2P-5"));

    // 會議的計時停了（網頁上離開）：下一輪同步時恢復
    h.fake.on("get_running_timers", j(r#"{"running":[]}"#));
    block_on(h.engine.refresh(true));
    assert!(h.engine.state().meeting_pause.is_none());
    assert!(h.notices.borrow().iter().any(|n| n == "會議的計時停了，恢復計時：F2P-5"));
}

#[test]
fn 審核提示_文字與按鈕() {
    let h = Harness::new();
    let list = HayakuApi::parse_change_requests(&j(r#"{"items":[{"id":"cr1","type":"UPDATE","status":"PENDING",
        "task":{"key":"F2P-7","title":"報表"},"requester":{"username":"b","displayName":"小美"},"createdAt":"2026-11-12T01:00:00Z",
        "reason":"做完了"}]}"#));
    let c = list.first().expect("解析變更申請");
    let t = h.engine.review_toast(c, true);
    assert_eq!(format!("待審核・{}", c.type_text()), t.title);
    assert_eq!("小美：F2P-7 報表\n理由：做完了", t.message);
    assert_eq!(vec!["核准", "駁回", "查看"], t.actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>());
    assert!(t.actions[0].primary);
    assert_eq!(Some("cr:cr1".to_owned()), t.tag);
    assert!(t.sticky && t.from_bottom);
    // demo：按了只跳說明，不送出
    block_on((t.actions[0].run)());
    assert_eq!(vec!["（測試）這是模擬的申請，沒有真的送出".to_owned()], *h.notices.borrow());
    assert!(h.fake.calls().is_empty());
}

#[test]
fn current_是最後建立還活著的engine() {
    // 同一個測試只能建一個 Harness（TempApp 會鎖住 APP_DIR_LOCK）
    let h = Harness::new();
    assert!(TimerEngine::current().is_some_and(|e| Rc::ptr_eq(&e.0, &h.engine.0)));
    drop(h);
    // 只存弱參照：engine 不在了就是 None，不會讓它多活
    assert!(TimerEngine::current().is_none());
}

#[test]
fn 按了暫停或開始_還在等_hayaku_時先照按下的結果顯示() {
    use hb_api::TimeEntry;
    let now = crate::time::now_offset();
    let mut list = vec![
        TimeEntry::new("e1", "F2P-1", "一", Some(now), None, None, true, None),
        TimeEntry::new("e2", "F2P-2", "二", Some(now), Some(now), Some(10), false, None),
    ];
    let mut pause = CiSet::new();
    pause.insert("f2p-1");
    let mut start = CiSet::new();
    start.insert("F2P-2");
    start.insert("F2P-3");
    apply_pending(&pause, &start, &mut list, now);
    // 暫停的那筆先標成停了
    assert!(!list[0].running);
    assert_eq!(Some(now), list[0].ended_at);
    // 開始的：沒有計時中的紀錄就先放一筆暫時的，標題沿用今天的紀錄
    let added: Vec<_> = list.iter().filter(|e| e.id.starts_with("pending:")).collect();
    assert_eq!(2, added.len());
    assert!(added.iter().all(|e| e.running));
    assert_eq!("二", added.iter().find(|e| e.task == "F2P-2").unwrap().title);
    assert_eq!("", added.iter().find(|e| e.task == "F2P-3").unwrap().title);

    // 同步讀回真的計時中紀錄後再套一次：不會重複放暫時的
    let mut list = vec![TimeEntry::new("e3", "F2P-3", "三", Some(now), None, None, true, None)];
    apply_pending(&CiSet::new(), &start, &mut list, now);
    assert_eq!(1, list.iter().filter(|e| e.task == "F2P-3").count());
}
