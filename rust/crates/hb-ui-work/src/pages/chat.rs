//! 工作台「聊天」頁（對應 C# `UI/Work/ChatPage.xaml(.cs)`；訊息列對應 `TasksPage.ChatRows`、`ShowPoll`）。
//! 專案頻道與任務聊天，讀訊息、送訊息；開著時每 20 秒更新。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::time::Duration;

use hb_api::{ChatMessage, KeyFinder};
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use slint::ComponentHandle;

use super::meetings::w3_common::{model, spawn, status_parts, wire_md};
use crate::{ChatMsg, ChatPageData, PageArg, W3Row, WorkCtx};

/// 一個對話：專案頻道（project＋channel）或任務聊天（task）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Conversation {
    pub title: String,
    pub subtitle: String,
    pub task: Option<String>,
    pub project: Option<String>,
    pub channel: Option<String>,
}

struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
    /// C# `_poll`（DispatcherTimer，20 秒）
    poll: slint::Timer,
}

#[derive(Default)]
struct State {
    current: Option<Conversation>,
    channels: Vec<Conversation>,
    channels_loaded: bool,
    /// 訊息清單有沒有東西（C# MessageList.ItemsSource 不是 null）
    has_items: bool,
    version: u64,
}

thread_local! {
    static PAGE: RefCell<Option<Rc<Page>>> = const { RefCell::new(None) };
}

macro_rules! bind {
    // 沒有參數的回呼：rustfmt 會把 `| |` 排成 `||`，兩種都接受
    ($g:ident . $on:ident, $page:ident, || $body:expr) => {
        bind!($g.$on, $page, | | $body)
    };
    ($g:ident . $on:ident, $page:ident, | $($arg:ident),* | $body:expr) => {{
        let weak: Weak<Page> = Rc::downgrade(&$page);
        $g.$on(move |$($arg),*| {
            if let Some($page) = weak.upgrade() {
                $body
            }
        });
    }};
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page { ctx: ctx.clone(), st: RefCell::new(State::default()), poll: slint::Timer::default() });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let weak = Rc::downgrade(&page);
    page.poll.start(slint::TimerMode::Repeated, Duration::from_secs(20), move || {
        let Some(page) = weak.upgrade() else { return };
        if page.is_visible() && page.st.borrow().current.is_some() {
            spawn(async move { page.load_messages(false).await });
        }
    });
    let Some(w) = ctx.window() else { return };
    let g = w.global::<ChatPageData>();
    bind!(g.on_task_entered, page, || spawn(async move { page.task_entered().await }));
    bind!(g.on_channel_selected, page, |i| {
        let c = page.st.borrow().channels.get(i as usize).cloned();
        if let Some(c) = c {
            spawn(async move { page.open(c).await });
        }
    });
    bind!(g.on_send, page, || spawn(async move { page.send().await }));
    bind!(g.on_reload, page, || spawn(async move { page.load_messages(false).await }));
    bind!(g.on_web, page, || page.web());
    bind!(g.on_poll, page, |id| {
        let w = page.ctx.window();
        hb_ui_common::poll::show(&page.ctx.engine, Some(&id), w.as_ref().map(|w| w.window()));
    });
    wire_md!(&g);
}

pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    page.activate(arg).await;
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&ChatPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<ChatPageData>();
        Some(f(&g))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_status(text);
            g.set_status_tone(tone);
        });
    }

    /// C# `IsVisible`：工作台開著而且正在看這頁。
    fn is_visible(&self) -> bool {
        let Some(w) = self.ctx.window() else { return false };
        let index = crate::pages::PAGES.iter().position(|(k, _)| *k == "chat").map_or(-1, |i| i as i32);
        w.window().is_visible() && w.get_page_index() == index
    }

    /// arg：任務編號 → 直接打開那個任務的聊天。
    async fn activate(self: Rc<Self>, arg: Option<PageArg>) {
        if !self.st.borrow().channels_loaded {
            self.load_channels().await;
        }
        if let Some(key) = arg.as_ref().and_then(|a| a.as_text()).and_then(|t| KeyFinder::parse(Some(t))) {
            let c = self.task_conversation(&key);
            self.open(c).await;
        }
    }

    fn task_conversation(&self, key: &str) -> Conversation {
        Conversation {
            title: format!("{key} {}", self.ctx.engine.title_of(key).unwrap_or_default()).trim().to_owned(),
            subtitle: "任務聊天".to_owned(),
            task: Some(key.to_owned()),
            project: None,
            channel: None,
        }
    }

    async fn load_channels(&self) {
        self.with(|g| g.set_channel_empty("讀取中…".into()));
        let api = self.ctx.engine.api();
        let result: Result<Vec<Conversation>, hb_api::McpError> = async {
            let mut list = Vec::new();
            for project in api.list_projects(false).await? {
                for channel in api.list_channels(&project.key).await? {
                    list.push(Conversation {
                        title: format!("# {channel}"),
                        subtitle: project.name.clone(),
                        task: None,
                        project: Some(project.key.clone()),
                        channel: Some(channel),
                    });
                }
            }
            Ok(list)
        }
        .await;
        match result {
            Ok(list) => {
                let rows: Vec<W3Row> = list
                    .iter()
                    .map(|c| W3Row {
                        title: c.title.as_str().into(),
                        meta: c.subtitle.as_str().into(),
                        ..Default::default()
                    })
                    .collect();
                let empty = list.is_empty();
                {
                    let mut st = self.st.borrow_mut();
                    st.channels = list;
                    st.channels_loaded = true;
                }
                self.with(|g| {
                    g.set_channels(model(rows));
                    g.set_channel_index(-1);
                    g.set_channel_empty(if empty { "沒有可以看的頻道" } else { "" }.into());
                });
            }
            Err(e) => {
                log::error!("讀取頻道失敗: {e}");
                self.with(|g| g.set_channel_empty(format!("讀取失敗：{e}").into()));
            }
        }
    }

    async fn task_entered(self: Rc<Self>) {
        let text = self.with(|g| g.get_task_input().to_string()).unwrap_or_default();
        let Some(key) = KeyFinder::parse(Some(&text)) else {
            self.show(Some(StatusMessage::error("請輸入任務編號，例 F2P-97")));
            return;
        };
        self.with(|g| g.set_channel_index(-1));
        let c = self.task_conversation(&key);
        self.open(c).await;
    }

    async fn open(self: Rc<Self>, c: Conversation) {
        let hint = match &c.task {
            Some(task) => format!("傳訊息到 {task}（可用 @帳號），Enter 送出、Shift+Enter 換行"),
            None => format!("傳到 {}（可用 @帳號），Enter 送出、Shift+Enter 換行", c.title),
        };
        {
            let mut st = self.st.borrow_mut();
            st.current = Some(c.clone());
            st.has_items = false;
        }
        self.with(|g| {
            g.set_conv_title(c.title.as_str().into());
            g.set_conv_subtitle(c.subtitle.as_str().into());
            g.set_has_conv(true);
            g.set_input_hint(hint.into());
            g.set_messages(model(Vec::new()));
        });
        self.clone().load_messages(true).await;
        self.with(|g| g.set_focus_input(g.get_focus_input() + 1));
    }

    async fn load_messages(self: Rc<Self>, scroll_to_end: bool) {
        let Some(c) = self.st.borrow().current.clone() else { return };
        let version = {
            let mut st = self.st.borrow_mut();
            st.version += 1;
            st.version
        };
        if !self.st.borrow().has_items {
            self.with(|g| g.set_message_empty("讀取中…".into()));
        }
        let result =
            self.ctx.engine.api().list_chat(c.task.as_deref(), c.project.as_deref(), c.channel.as_deref()).await;
        match result {
            Ok(messages) => {
                if version != self.st.borrow().version || self.st.borrow().current.as_ref() != Some(&c) {
                    return;
                }
                self.st.borrow_mut().has_items = true;
                let empty = messages.is_empty();
                let rows = chat_rows(&messages);
                self.with(|g| {
                    let at_end = g.get_at_end();
                    g.set_messages(model(rows));
                    g.set_message_empty(if empty { "還沒有訊息，說點什麼吧" } else { "" }.into());
                    if scroll_to_end || at_end {
                        g.set_scroll_end(g.get_scroll_end() + 1);
                    }
                });
            }
            Err(e) => {
                log::error!("讀取聊天失敗: {e}");
                if version == self.st.borrow().version {
                    self.with(|g| g.set_message_empty(format!("讀取失敗：{e}").into()));
                }
            }
        }
    }

    async fn send(self: Rc<Self>) {
        let body = self.with(|g| g.get_message_text().trim().to_owned()).unwrap_or_default();
        let Some(c) = self.st.borrow().current.clone() else { return };
        if body.is_empty() {
            return;
        }
        let api = self.ctx.engine.api();
        let p = self.clone();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_sending(b));
            },
            |m| p.show(m),
            || async move {
                api.send_chat(c.task.as_deref(), c.project.as_deref(), c.channel.as_deref(), &body)
                    .await
                    .map_err(ActionError::from)
            },
            None,
        )
        .await;
        if ok {
            self.with(|g| g.set_message_text("".into()));
            self.load_messages(true).await;
        }
    }

    fn web(&self) {
        let Some(c) = self.st.borrow().current.clone() else { return };
        let engine = &self.ctx.engine;
        hb_win::open(&match &c.task {
            Some(task) => engine.task_url(task),
            None => engine.web_url("/chat"),
        });
    }
}

/// C# `TasksPage.ChatRows`：照時間排、刪掉的訊息與投票換成說明文字。
pub(crate) fn chat_rows(messages: &[ChatMessage]) -> Vec<ChatMsg> {
    let mut sorted: Vec<&ChatMessage> = messages.iter().collect();
    sorted.sort_by_key(|m| m.created_at);
    sorted
        .into_iter()
        .map(|m| ChatMsg {
            author: m.author.display_name.as_str().into(),
            time: inputs::ago(&m.created_at).into(),
            body: if m.deleted {
                "（訊息已刪除）".to_owned()
            } else if let Some(q) = &m.poll_question {
                format!("📊 投票：{q}")
            } else {
                m.body.clone()
            }
            .into(),
            poll_id: if m.deleted { "".into() } else { m.poll_id.clone().unwrap_or_default().into() },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};
    use hb_api::PersonRef;

    fn msg(min: u32, body: &str, deleted: bool, poll: Option<(&str, &str)>) -> ChatMessage {
        ChatMessage {
            id: format!("m{min}"),
            author: PersonRef::new("amy", "Amy"),
            body: body.into(),
            created_at: FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 1, 1, 9, min, 0).unwrap(),
            deleted,
            poll_question: poll.map(|p| p.0.to_owned()),
            poll_id: poll.map(|p| p.1.to_owned()),
        }
    }

    #[test]
    fn chat_rows_照時間排並換掉刪除與投票() {
        let rows = chat_rows(&[
            msg(5, "後", false, None),
            msg(1, "先", false, None),
            msg(3, "x", true, Some(("午餐？", "p1"))),
            msg(4, "", false, Some(("午餐？", "p2"))),
        ]);
        let bodies: Vec<_> = rows.iter().map(|r| r.body.to_string()).collect();
        assert_eq!(vec!["先", "（訊息已刪除）", "📊 投票：午餐？", "後"], bodies);
        assert_eq!("", rows[1].poll_id.as_str());
        assert_eq!("p2", rows[2].poll_id.as_str());
        assert_eq!("Amy", rows[0].author.as_str());
    }
}
