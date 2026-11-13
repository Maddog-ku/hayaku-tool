//! 對應 C# `Slack/SlackBridge.cs`：把通知傳到 Slack 私訊、在 Slack 上按的按鈕回來處理。
//!
//! 傳什麼：要動手處理的（待我審核、測試結果被退回或進待復驗）一律傳，其他通知只在螢幕鎖定時傳；電腦上的提示照樣跳。
//! 都用同步已經抓到的資料判斷，不會多跟 Hayaku 要資料。兩邊狀態會跟著變：在 Slack 處理了，電腦上那則提示會關掉；
//! 在 HayakuBar 或網頁處理了，Slack 那則訊息會改成「已在 HayakuBar 或網頁處理」、按鈕拿掉。
//!
//! 全部在 UI 執行緒上跑，狀態不用另外上鎖（`Rc`／`RefCell`）。Socket Mode 在 tokio 執行緒收到的互動、狀態變化，
//! 經過 channel 轉回 UI 執行緒（C# `SynchronizationContext.Post`）：建立時用 `engine.spawn`（`slint::spawn_local`）
//! 起一個收訊息的迴圈。C# fire-and-forget 的 `_ = PostAsync(p)` 也走 `engine.spawn`。
//!
//! App 的用法（C# `engine.Slack = new SlackBridge(engine); engine.Slack.Restart();`）：
//! ```ignore
//! let slack = hb_slack::SlackBridge::new(engine.clone());
//! engine.set_slack(Some(Rc::new(slack.clone())));
//! slack.restart();
//! ```

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::{Rc, Weak};
use std::sync::Arc;

use chrono::{Local, NaiveDateTime};
use hb_api::DateTimeOffset;
use hb_core::{CoreError, HandlerId, SlackLink, TimerEngine};
use serde_json::Value;
use tokio::sync::Semaphore;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

use crate::slack_client::{SlackClient, SlackError};
use crate::slack_credentials::{SlackCredentialStore, SlackCredentials};
use crate::slack_messages::{SlackInteraction, SlackMessages as M};
use crate::slack_state::{SlackPost, SlackState};
use crate::socket_mode_client::{OnInteractive, OpenUrl, SocketModeClient};

type StatusHandler = Rc<dyn Fn()>;

const DONE_ELSEWHERE: &str = "✓ 已在 HayakuBar 或網頁處理";

/// 從 Socket Mode（tokio 執行緒）轉回 UI 執行緒的訊息。
enum UiMsg {
    Interactive(Box<SlackInteraction>),
    Status,
    Stop,
}

struct Inner {
    engine: TimerEngine,
    client: SlackClient,
    state: RefCell<SlackState>,
    seen: RefCell<HashSet<String>>,
    busy: RefCell<HashSet<String>>,
    io: Semaphore,
    started_at: DateTimeOffset,
    cred: RefCell<SlackCredentials>,
    socket: RefCell<Option<SocketModeClient>>,
    dm: RefCell<Option<String>>,
    last_sent: Cell<Option<NaiveDateTime>>,
    last_error: RefCell<Option<String>>,
    locked: Cell<bool>,
    status_handlers: RefCell<Vec<(u64, StatusHandler)>>,
    next_handler: Cell<u64>,
    changed_handler: Cell<Option<HandlerId>>,
    tx: UnboundedSender<UiMsg>,
}

/// 把通知傳到 Slack 私訊、處理 Slack 上的按鈕。`clone()` 很便宜（共用同一份狀態）。
///
/// 注意：SlackBridge 拿著 TimerEngine，TimerEngine 又拿著 SlackBridge（`set_slack`），兩邊一起活到程式結束（跟 C# 一樣）。
#[derive(Clone)]
pub struct SlackBridge(Rc<Inner>);

fn now() -> NaiveDateTime {
    Local::now().naive_local()
}

impl SlackBridge {
    pub fn new(engine: TimerEngine) -> Self {
        Self::with_client(engine, SlackClient::new())
    }

    /// 指定 SlackClient（測試用本機假伺服器）。
    pub fn with_client(engine: TimerEngine, client: SlackClient) -> Self {
        let (tx, rx) = unbounded_channel();
        let bridge = SlackBridge(Rc::new(Inner {
            engine: engine.clone(),
            client,
            state: RefCell::new(SlackState::load()),
            seen: RefCell::new(HashSet::new()),
            busy: RefCell::new(HashSet::new()),
            io: Semaphore::new(1),
            started_at: hb_core::time::now_offset(),
            cred: RefCell::new(SlackCredentials::default()),
            socket: RefCell::new(None),
            dm: RefCell::new(None),
            last_sent: Cell::new(None),
            last_error: RefCell::new(None),
            locked: Cell::new(false),
            status_handlers: RefCell::new(Vec::new()),
            next_handler: Cell::new(1),
            changed_handler: Cell::new(None),
            tx,
        }));
        let weak = Rc::downgrade(&bridge.0);
        let id = engine.events().changed.subscribe(move |_| {
            if let Some(b) = Self::upgrade(&weak) {
                b.on_changed();
            }
        });
        bridge.0.changed_handler.set(Some(id));
        engine.spawn(Self::pump(Rc::downgrade(&bridge.0), rx));
        bridge
    }

    fn upgrade(weak: &Weak<Inner>) -> Option<SlackBridge> {
        weak.upgrade().map(SlackBridge)
    }

    /// UI 執行緒上收 Socket Mode 轉過來的訊息（C# `_ui.Post`）。
    async fn pump(weak: Weak<Inner>, mut rx: UnboundedReceiver<UiMsg>) {
        while let Some(msg) = rx.recv().await {
            let Some(bridge) = Self::upgrade(&weak) else {
                return;
            };
            match msg {
                UiMsg::Stop => return,
                UiMsg::Status => bridge.raise_status(),
                UiMsg::Interactive(i) => {
                    let b = bridge.clone();
                    bridge.0.engine.spawn(async move {
                        if let Err(err) = b.handle(*i).await {
                            log::error!("處理 Slack 按鈕失敗: {err}");
                        }
                    });
                }
            }
        }
    }

    /// 螢幕鎖定中（離開座位）：一般通知也傳。
    pub fn locked(&self) -> bool {
        self.0.locked.get()
    }

    pub fn set_locked(&self, locked: bool) {
        self.0.locked.set(locked);
    }

    pub fn configured(&self) -> bool {
        !self.0.cred.borrow().is_empty()
    }

    fn forwarding(&self) -> bool {
        self.configured() && self.0.engine.config().slack_forward
    }

    fn socket_problem(&self) -> Option<String> {
        self.0.socket.borrow().as_ref().and_then(SocketModeClient::problem)
    }

    /// 設定頁顯示的狀態。
    pub fn status_text(&self) -> String {
        if !self.configured() {
            return String::new();
        }
        let mut parts = vec![self.socket_problem().unwrap_or_else(|| "● 已連上，Slack 上的按鈕可以用".to_owned())];
        if let Some(sent) = self.0.last_sent.get() {
            parts.push(format!("上次傳送 {}", sent.format("%-m/%-d %H:%M")));
        }
        if let Some(error) = self.0.last_error.borrow().as_ref() {
            parts.push(format!("上次傳送失敗：{error}"));
        }
        parts.join(" · ")
    }

    pub fn healthy(&self) -> bool {
        self.configured() && self.socket_problem().is_none() && self.0.last_error.borrow().is_none()
    }

    /// 讀金鑰、重新連線（設定頁連線或中斷連線後呼叫）。
    pub fn restart(&self) {
        let old = self.0.socket.borrow_mut().take();
        if let Some(socket) = old {
            socket.dispose();
        }
        *self.0.dm.borrow_mut() = None;
        *self.0.last_error.borrow_mut() = None;
        *self.0.cred.borrow_mut() = SlackCredentialStore::load();
        if self.configured() {
            let client = self.0.client.clone();
            let app_token = self.0.cred.borrow().app_token.clone();
            let open_url: OpenUrl = Arc::new(move || {
                let client = client.clone();
                let token = app_token.clone();
                Box::pin(async move { client.open_socket_url(&token).await })
            });
            let tx = self.0.tx.clone();
            // Socket Mode 的執行緒收到的：轉回 UI 執行緒處理
            let on_interactive: OnInteractive = Arc::new(move |payload: Value| {
                let _ = tx.send(UiMsg::Interactive(Box::new(M::parse_interaction(&payload))));
            });
            let socket = SocketModeClient::new(open_url, on_interactive);
            let tx = self.0.tx.clone();
            socket.on_status_changed(move || {
                let _ = tx.send(UiMsg::Status);
            });
            socket.start();
            *self.0.socket.borrow_mut() = Some(socket);
        }
        self.raise_status();
    }

    /// 狀態變了（C# `StatusChanged += …`）；回傳編號給 [`SlackBridge::unsubscribe_status_changed`]。
    pub fn subscribe_status_changed(&self, f: impl Fn() + 'static) -> u64 {
        let id = self.0.next_handler.get();
        self.0.next_handler.set(id + 1);
        self.0.status_handlers.borrow_mut().push((id, Rc::new(f)));
        id
    }

    pub fn unsubscribe_status_changed(&self, id: u64) {
        self.0.status_handlers.borrow_mut().retain(|(h, _)| *h != id);
    }

    fn raise_status(&self) {
        let handlers: Vec<StatusHandler> = self.0.status_handlers.borrow().iter().map(|(_, h)| h.clone()).collect();
        for h in handlers {
            h();
        }
    }

    // ───────────── 傳出去 ─────────────

    fn on_changed(&self) {
        let engine = &self.0.engine;
        if !engine.logged_in() {
            return;
        }
        // 沒開轉送也照樣記「看過了」，之後打開才不會把之前的一次全傳出去
        let (notices, reviews) = {
            let mut seen = self.0.seen.borrow_mut();
            let notices = M::select_notifications(
                &engine.notifications(),
                &mut seen,
                self.0.started_at,
                self.locked(),
                engine.can_review(),
            );
            let reviews = M::select_reviews(&engine.pending_reviews(), &mut seen, self.0.started_at);
            (notices, reviews)
        };
        if !self.configured() {
            return;
        }
        self.reconcile_handled_elsewhere();
        if !self.forwarding() {
            return;
        }

        let now = now();
        let mut posts: Vec<SlackPost> = reviews
            .iter()
            .map(|c| {
                let url = match &c.task_key {
                    Some(key) => engine.task_url(key),
                    None => engine.web_url("/notifications"),
                };
                M::from_review(c, &url, now)
            })
            .collect();
        posts.extend(
            notices
                .iter()
                .filter(|n| M::is_actionable(n))
                .map(|n| M::from_notification(n, &engine.notification_url(n), now)),
        );
        let plain: Vec<_> = notices.iter().filter(|n| !M::is_actionable(n)).cloned().collect();
        if plain.len() > M::MAX_SINGLE {
            posts.push(M::from_notices(&plain, &engine.web_url("/notifications"), now));
        } else {
            posts.extend(plain.iter().map(|n| M::from_notification(n, &engine.notification_url(n), now)));
        }
        for p in posts {
            let b = self.clone();
            engine.spawn(async move { b.post(p).await });
        }
    }

    async fn post(&self, mut p: SlackPost) {
        let inner = &self.0;
        let permit = inner.io.acquire().await;
        let result: Result<(), SlackError> = async {
            let cred = inner.cred.borrow().clone();
            let cached = inner.dm.borrow().clone();
            let dm = match cached {
                Some(dm) => dm,
                None => {
                    let dm = inner.client.open_dm(&cred.bot_token, &cred.user_id).await?;
                    *inner.dm.borrow_mut() = Some(dm.clone());
                    dm
                }
            };
            p.channel = dm.clone();
            p.ts = inner
                .client
                .post_message(&cred.bot_token, &dm, &M::fallback(&p), Some(Value::Array(M::blocks(&p, None))))
                .await?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                inner.state.borrow_mut().posts.push(p);
                self.save_state();
                inner.last_sent.set(Some(now()));
                *inner.last_error.borrow_mut() = None;
            }
            Err(err) => {
                log::error!("傳到 Slack 失敗（{}）: {err}", p.title);
                *inner.last_error.borrow_mut() = Some(err.message());
            }
        }
        drop(permit);
        self.raise_status();
    }

    /// 在 HayakuBar 或網頁處理掉的（通知已讀、申請不在待審清單了），Slack 那則訊息拿掉按鈕。
    fn reconcile_handled_elsewhere(&self) {
        let engine = &self.0.engine;
        let reviews_fetched = engine.reviews_fetched_at();
        let can_review = engine.can_review();
        let pending = engine.pending_reviews();
        let notifications = engine.notifications();
        let mut updates = Vec::new();
        {
            let busy = self.0.busy.borrow();
            let mut state = self.0.state.borrow_mut();
            for p in state.posts.iter_mut() {
                if p.done.is_some() || p.ts.is_empty() || busy.contains(&p.key) {
                    continue;
                }
                let handled = if p.kind == "review" {
                    can_review
                        && reviews_fetched > p.posted_at
                        && p.change_request_id.as_ref().is_some_and(|id| pending.iter().all(|c| &c.id != id))
                } else {
                    !p.notification_ids.is_empty()
                        && p.notification_ids
                            .iter()
                            .all(|id| notifications.iter().find(|n| &n.id == id).is_some_and(|n| !n.unread))
                };
                if !handled {
                    continue;
                }
                p.done = Some(DONE_ELSEWHERE.to_owned());
                updates.push(p.clone());
            }
        }
        if updates.is_empty() {
            return;
        }
        for p in updates {
            let b = self.clone();
            engine.spawn(async move { b.update(&p, None).await });
        }
        self.save_state();
    }

    async fn update(&self, p: &SlackPost, error: Option<&str>) {
        let inner = &self.0;
        let _permit = inner.io.acquire().await;
        let bot = inner.cred.borrow().bot_token.clone();
        let blocks = Value::Array(M::blocks(p, error));
        if let Err(err) = inner.client.update_message(&bot, &p.channel, &p.ts, &M::fallback(p), Some(blocks)).await {
            log::error!("更新 Slack 訊息失敗（{}）: {err}", p.title);
        }
    }

    fn save_state(&self) {
        let mut state = self.0.state.borrow_mut();
        state.prune(now());
        state.save();
    }

    // ───────────── 按鈕回來 ─────────────

    /// 處理一則互動（已經在 UI 執行緒）；錯誤由呼叫端寫紀錄（C# 的 catch）。
    async fn handle(&self, i: SlackInteraction) -> Result<(), SlackError> {
        let inner = &self.0;
        let cred = inner.cred.borrow().clone();
        if !self.configured() || i.user_id.as_deref() != Some(cred.user_id.as_str()) {
            log::info!("略過 Slack 互動：不是設定的成員（{}）", i.user_id.as_deref().unwrap_or(""));
            return Ok(());
        }
        if i.r#type == "block_actions" {
            let (Some(action), Some(value)) = (i.action_id.as_deref(), i.value.as_deref()) else {
                return Ok(());
            };
            if action == M::OPEN {
                return Ok(());
            }
            let found = inner.state.borrow().find(value).cloned();
            let Some(p) = found else {
                if let (Some(channel), Some(ts)) = (&i.channel, &i.ts) {
                    inner
                        .client
                        .update_message(
                            &cred.bot_token,
                            channel,
                            ts,
                            "這則訊息超過 7 天，請到 HayakuBar 或網頁處理",
                            Some(Value::Array(Vec::new())),
                        )
                        .await?;
                }
                return Ok(());
            };
            if p.done.is_some() {
                self.update(&p, None).await;
                return Ok(());
            }
            if action == M::REJECT || action == M::REOPEN {
                if let Some(trigger) = &i.trigger_id {
                    inner.client.open_view(&cred.bot_token, trigger, M::reason_modal(&p, action)).await?;
                }
                return Ok(());
            }
            self.run(p, action, None).await;
        } else if i.r#type == "view_submission"
            && i.callback_id.as_deref() == Some(M::REASON_CALLBACK)
            && let Some((action, key)) = i.metadata.as_deref().and_then(|m| m.split_once('|'))
        {
            let found = inner.state.borrow().find(key).filter(|p| p.done.is_none()).cloned();
            if let Some(p) = found {
                self.run(p, action, i.reason.as_deref().map(str::trim)).await;
            }
        }
        Ok(())
    }

    async fn run(&self, mut p: SlackPost, action: &str, reason: Option<&str>) {
        let inner = &self.0;
        if !inner.busy.borrow_mut().insert(p.key.clone()) {
            return;
        }
        match self.do_action(&p, action, reason).await {
            Ok(result) => {
                let done = format!("{result}（{}，從 Slack）", now().format("%H:%M"));
                p.done = Some(done.clone());
                if let Some(saved) = inner.state.borrow_mut().find_mut(&p.key) {
                    saved.done = Some(done);
                }
                log::info!("Slack：{} {result}", p.title);
                if let Some(tag) = &p.toast_tag {
                    inner.engine.dismiss_toast(tag);
                }
                self.save_state();
                self.update(&p, None).await;
            }
            Err(err) => {
                log::error!("Slack：{} {action} 失敗: {err}", p.title);
                self.update(&p, Some(err.message())).await;
            }
        }
        inner.busy.borrow_mut().remove(&p.key);
    }

    async fn do_action(&self, p: &SlackPost, action: &str, reason: Option<&str>) -> Result<String, CoreError> {
        let engine = &self.0.engine;
        let key = || p.task_key.clone().ok_or_else(|| CoreError::InvalidOperation("這則訊息沒有任務編號".to_owned()));
        let request = || {
            p.change_request_id.clone().ok_or_else(|| CoreError::InvalidOperation("這則訊息沒有對應的申請".to_owned()))
        };
        let reason = reason.filter(|r| !r.is_empty());
        match action {
            M::READ => {
                engine.mark_read_from_slack(&p.notification_ids).await?;
                Ok("✓ 已標為已讀".to_owned())
            }
            M::ARCHIVE => {
                engine.archive_from_slack(&p.notification_ids).await?;
                Ok("✓ 已封存".to_owned())
            }
            M::APPROVE => {
                engine.review_from_slack(&request()?, true, false, None).await?;
                Ok("✓ 已核准".to_owned())
            }
            M::APPROVE_WATCH => {
                engine.review_from_slack(&request()?, true, true, None).await?;
                Ok("✓ 已核准並關注".to_owned())
            }
            M::REJECT => {
                engine.review_from_slack(&request()?, false, false, reason).await?;
                Ok(match reason {
                    Some(r) => format!("✕ 已駁回：{r}"),
                    None => "✕ 已駁回".to_owned(),
                })
            }
            M::PASS => {
                engine.retest(&key()?, true, None).await?;
                Ok("✓ 復驗通過".to_owned())
            }
            M::REOPEN => {
                let Some(r) = reason else {
                    return Err(CoreError::InvalidOperation("退回要填原因".to_owned()));
                };
                engine.retest(&key()?, false, Some(r)).await?;
                Ok(format!("↩ 已退回：{r}"))
            }
            M::FIX => {
                engine.start_fix(&key()?, None, true).await?;
                Ok("▶ 開始修，已開始計時".to_owned())
            }
            other => Err(CoreError::InvalidOperation(format!("不認得的按鈕：{other}"))),
        }
    }

    /// 程式結束時呼叫：不再接 engine 的變化、關掉 Socket Mode 連線。
    pub fn dispose(&self) {
        if let Some(id) = self.0.changed_handler.take() {
            self.0.engine.events().changed.unsubscribe(id);
        }
        let socket = self.0.socket.borrow_mut().take();
        if let Some(socket) = socket {
            socket.dispose();
        }
        let _ = self.0.tx.send(UiMsg::Stop);
    }
}

impl SlackLink for SlackBridge {
    fn locked(&self) -> bool {
        SlackBridge::locked(self)
    }

    fn set_locked(&self, locked: bool) {
        SlackBridge::set_locked(self, locked);
    }

    fn configured(&self) -> bool {
        SlackBridge::configured(self)
    }

    fn healthy(&self) -> bool {
        SlackBridge::healthy(self)
    }

    fn status_text(&self) -> String {
        SlackBridge::status_text(self)
    }

    fn restart(&self) {
        SlackBridge::restart(self);
    }

    fn dispose(&self) {
        SlackBridge::dispose(self);
    }

    fn subscribe_status_changed(&self, f: Box<dyn Fn()>) -> u64 {
        SlackBridge::subscribe_status_changed(self, f)
    }

    fn unsubscribe_status_changed(&self, id: u64) {
        SlackBridge::unsubscribe_status_changed(self, id);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(test)]
mod tests;
