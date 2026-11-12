//! SlackBridge 的流程測試：本機假 Slack 伺服器、假的 MCP transport、暫存的 AppPaths（不碰真的 Slack、Hayaku、%APPDATA%）。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::TimeDelta;
use hb_api::{BoxFuture, HayakuApi, McpError, McpTransport};
use hb_backend::auth::OAuthClient;
use hb_backend::config::{AppConfig, AppState};
use hb_backend::infra::AppPaths;
use hb_claude::ClaudeMonitor;
use hb_core::{LocalFuture, TimerEngine};
use serde_json::{Value, json};

use super::*;
use crate::test_support::{MockServer, Recorded, Reply, TempApp, block_on};

#[derive(Default)]
struct FakeTransport {
    calls: Mutex<Vec<(String, Value)>>,
}

impl McpTransport for FakeTransport {
    fn call_tool<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push((name.to_owned(), arguments));
            Ok(Value::Null)
        })
    }

    fn call_tool_raw<'a>(&'a self, name: &'a str, arguments: Value) -> BoxFuture<'a, Result<Value, McpError>> {
        self.call_tool(name, arguments)
    }
}

struct Harness {
    bridge: SlackBridge,
    server: MockServer,
    fake: Arc<FakeTransport>,
    queued: Rc<RefCell<Vec<LocalFuture>>>,
    dismissed: Rc<RefCell<Vec<String>>>,
    _engine: TimerEngine,
    _app: TempApp,
}

fn post(key: &str, kind: &str, ids: &[&str], task: Option<&str>) -> SlackPost {
    SlackPost {
        key: key.to_owned(),
        kind: kind.to_owned(),
        channel: "D1".to_owned(),
        ts: format!("ts-{key}"),
        title: format!("標題 {key}"),
        body: "內容".to_owned(),
        url: Some("https://x".to_owned()),
        notification_ids: ids.iter().map(|s| s.to_string()).collect(),
        task_key: task.map(str::to_owned),
        toast_tag: Some(key.to_owned()),
        posted_at: now() - TimeDelta::hours(1),
        ..Default::default()
    }
}

impl Harness {
    fn new(posts: Vec<SlackPost>) -> Self {
        let app = TempApp::new();
        SlackCredentialStore::save(&SlackCredentials {
            bot_token: "xoxb-1".into(),
            app_token: "xapp-1".into(),
            user_id: "U1".into(),
            team: "公司".into(),
        })
        .unwrap();
        SlackState { posts }.save();
        let server = MockServer::start(|req: &Recorded| match req.path.as_str() {
            "/api/apps.connections.open" => Reply::json(200, r#"{"ok":false,"error":"invalid_auth"}"#),
            _ => Reply::json(200, r#"{"ok":true}"#),
        });
        let fake = Arc::new(FakeTransport::default());
        let api = Arc::new(HayakuApi::new(fake.clone()));
        let auth = OAuthClient::with_store(
            hb_backend::http::app_client(),
            "https://example.invalid",
            &AppPaths::dir().join("auth.bin"),
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
        let dismissed: Rc<RefCell<Vec<String>>> = Rc::default();
        let d = dismissed.clone();
        engine.events().toast_dismiss_requested.subscribe(move |tag| d.borrow_mut().push(tag.clone()));
        let client = SlackClient::with_base_url(format!("{}/api/", server.base_url));
        let bridge = SlackBridge::with_client(engine.clone(), client);
        engine.set_slack(Some(Rc::new(bridge.clone())));
        Self { bridge, server, fake, queued, dismissed, _engine: engine, _app: app }
    }

    fn handle(&self, payload: &str) {
        let v: Value = serde_json::from_str(payload).unwrap();
        block_on(self.bridge.handle(M::parse_interaction(&v))).unwrap();
    }

    fn slack_calls(&self) -> Vec<Recorded> {
        self.server.requests().into_iter().filter(|r| r.path != "/api/apps.connections.open").collect()
    }

    fn mcp_tools(&self) -> Vec<String> {
        self.fake.calls.lock().unwrap().iter().map(|c| c.0.clone()).collect()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.bridge.dispose();
    }
}

fn button(action: &str, value: &str) -> String {
    format!(
        r#"{{"type":"block_actions","user":{{"id":"U1"}},"trigger_id":"t1",
            "container":{{"type":"message","message_ts":"old.1","channel_id":"D9"}},
            "actions":[{{"action_id":"{action}","value":"{value}","type":"button"}}]}}"#
    )
}

fn body_of(r: &Recorded) -> Value {
    serde_json::from_str(&r.body).unwrap()
}

#[test]
fn 沒設定_狀態是空的_重新連線後讀金鑰並連socket_mode() {
    let h = Harness::new(Vec::new());
    assert!(!h.bridge.configured());
    assert_eq!("", h.bridge.status_text());
    assert!(!h.bridge.healthy());

    let raised = Rc::new(Cell::new(0));
    let r = raised.clone();
    let id = h.bridge.subscribe_status_changed(move || r.set(r.get() + 1));
    h.bridge.restart();
    assert_eq!(1, raised.get());
    assert!(h.bridge.configured());
    // 假伺服器回 invalid_auth：連不上，說明照 Describe
    let deadline = Instant::now() + Duration::from_secs(10);
    while h.bridge.status_text() != "連不上：金鑰不對，請重新複製" {
        assert!(Instant::now() < deadline, "{}", h.bridge.status_text());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!h.bridge.healthy());
    let link: &dyn SlackLink = &h.bridge;
    assert_eq!(h.bridge.status_text(), link.status_text());
    assert!(link.as_any().downcast_ref::<SlackBridge>().is_some());
    h.bridge.unsubscribe_status_changed(id);
    h.bridge.restart();
    assert_eq!(1, raised.get());
}

#[test]
fn socket_mode轉回來的狀態_在ui執行緒觸發() {
    let h = Harness::new(Vec::new());
    // 建立時排進來的第一個工作是收訊息的迴圈
    let pump = h.queued.borrow_mut().remove(0);
    let raised = Rc::new(Cell::new(0));
    let r = raised.clone();
    h.bridge.subscribe_status_changed(move || r.set(r.get() + 1));
    h.bridge.0.tx.send(UiMsg::Status).unwrap();
    h.bridge.0.tx.send(UiMsg::Stop).unwrap();
    block_on(pump);
    assert_eq!(1, raised.get());
}

#[test]
fn 不是設定的成員_略過() {
    let h = Harness::new(vec![post("n:1", "notice", &["1"], None)]);
    h.bridge.restart();
    h.handle(&button("read", "n:1").replace("U1", "U2"));
    h.handle(&button("read", "n:1").replace(r#""user":{"id":"U1"},"#, ""));
    assert!(h.slack_calls().is_empty());
    assert!(h.mcp_tools().is_empty());
}

#[test]
fn 找不到訊息_改成超過7天的說明() {
    let h = Harness::new(Vec::new());
    h.bridge.restart();
    h.handle(&button("read", "n:404"));
    let calls = h.slack_calls();
    assert_eq!(1, calls.len());
    assert_eq!("/api/chat.update", calls[0].path);
    assert_eq!(
        json!({ "channel": "D9", "ts": "old.1", "text": "這則訊息超過 7 天，請到 HayakuBar 或網頁處理", "blocks": [] }),
        body_of(&calls[0])
    );
}

#[test]
fn 開啟按鈕_不用處理() {
    let h = Harness::new(vec![post("n:1", "notice", &["1"], None)]);
    h.bridge.restart();
    h.handle(&button("open", "n:1"));
    assert!(h.slack_calls().is_empty());
}

#[test]
fn 標為已讀_呼叫hayaku_關掉本機提示_訊息換成結果() {
    let h = Harness::new(vec![post("n:1", "notice", &["1"], None)]);
    h.bridge.restart();
    h.handle(&button("read", "n:1"));
    assert_eq!(vec!["mark_notifications_read"], h.mcp_tools());
    assert_eq!(json!({ "ids": ["1"] }), h.fake.calls.lock().unwrap()[0].1);
    assert_eq!(vec!["n:1"], *h.dismissed.borrow());
    let calls = h.slack_calls();
    assert_eq!(1, calls.len());
    let body = body_of(&calls[0]);
    assert_eq!(("D1", "ts-n:1"), (body["channel"].as_str().unwrap(), body["ts"].as_str().unwrap()));
    let done = body["blocks"][1]["elements"][0]["text"].as_str().unwrap();
    assert!(done.starts_with("✓ 已標為已讀（") && done.ends_with("，從 Slack）"), "{done}");
    assert!(!calls[0].body.contains("\"actions\""));
    // 存進 slack.json，重開程式也知道處理過了
    let saved = SlackState::load();
    assert_eq!(Some(done), saved.find("n:1").unwrap().done.as_deref());

    // 再按一次：已經處理過，只把結果再送一次，不再呼叫 Hayaku
    h.handle(&button("archive", "n:1"));
    assert_eq!(1, h.mcp_tools().len());
    assert_eq!(2, h.slack_calls().len());
}

#[test]
fn 退回_先跳表單_沒填原因就失敗_按鈕留著() {
    let h = Harness::new(vec![post("n:2", "retest", &["2"], Some("F2P-1"))]);
    h.bridge.restart();
    h.handle(&button("reopen", "n:2"));
    let calls = h.slack_calls();
    assert_eq!("/api/views.open", calls[0].path);
    let body = body_of(&calls[0]);
    assert_eq!("t1", body["trigger_id"]);
    assert_eq!("reopen|n:2", body["view"]["private_metadata"]);
    assert_eq!(false, body["view"]["blocks"][1]["optional"]);

    let submit = |reason: &str| {
        format!(
            r#"{{"type":"view_submission","user":{{"id":"U1"}},
                "view":{{"callback_id":"reason","private_metadata":"reopen|n:2",
                "state":{{"values":{{"reason":{{"text":{{"type":"plain_text_input","value":"{reason}"}}}}}}}}}}}}"#
        )
    };
    h.handle(&submit("   "));
    assert!(h.mcp_tools().is_empty());
    let calls = h.slack_calls();
    assert_eq!("/api/chat.update", calls[1].path);
    assert!(calls[1].body.contains("⚠ 退回要填原因"), "{}", calls[1].body);
    assert!(calls[1].body.contains("\"action_id\":\"reopen\""));
    assert!(SlackState::load().find("n:2").unwrap().done.is_none());
    assert!(h.dismissed.borrow().is_empty());
}

#[test]
fn 審核訊息沒有申請編號_顯示原因() {
    let h = Harness::new(vec![post("cr:x", "review", &[], Some("F2P-1"))]);
    h.bridge.restart();
    h.handle(&button("approve", "cr:x"));
    let calls = h.slack_calls();
    assert_eq!(1, calls.len());
    assert!(calls[0].body.contains("⚠ 這則訊息沒有對應的申請"), "{}", calls[0].body);
}
