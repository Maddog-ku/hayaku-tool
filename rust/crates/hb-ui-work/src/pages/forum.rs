//! 工作台「論壇」頁（對應 C# `UI/Work/ForumPage.cs`）。
//! 選專案 → 頻道（論壇）→ 左邊貼文清單（置頂在最上面，可只看未解決）→ 右邊貼文內容與回應；可以回覆、發新文。
//! 第一次切到這頁才讀；頻道清單、貼文清單、貼文內容快取到按重新整理或換選擇。
//! 頻道清單分不出聊天頻道和論壇，選到聊天頻道時伺服器回「找不到這個論壇」，就提示到「聊天」頁。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use hb_api::{ForumPost, ForumThread};
use hb_ui_common::ProjectIcons;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use slint::ComponentHandle;

use super::meetings::w3_common::{escape_data, model, spawn, status_parts, strings, wire_md};
use crate::{ForumPageData, ForumReply, PageArg, W3Row, WorkCtx};

/// 標題最多幾個字（C# MaxLength = 120）。
const TITLE_MAX: usize = 120;

struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
}

#[derive(Default)]
struct State {
    /// 下拉選單上的專案（代碼、名稱、圖示、顏色）
    projects: Vec<hb_api::ProjectInfo>,
    project_key: Option<String>,
    channel_list: Vec<String>,
    channel: Option<String>,
    /// 頻道清單、上次選的頻道（專案代碼不分大小寫，key 用小寫）
    channels: HashMap<String, Vec<String>>,
    last_channel: HashMap<String, String>,
    post_cache: HashMap<String, Vec<ForumPost>>,
    threads: HashMap<String, ForumThread>,
    posts: Vec<ForumPost>,
    post: Option<ForumPost>,
    loaded: bool,
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
    let page = Rc::new(Page { ctx: ctx.clone(), st: RefCell::new(State::default()) });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let Some(w) = ctx.window() else { return };
    let g = w.global::<ForumPageData>();
    bind!(g.on_project_selected, page, |i| {
        let key = page.st.borrow().projects.get(i as usize).map(|p| p.key.clone());
        page.st.borrow_mut().project_key = key;
        page.render_project_icon();
        spawn(async move { page.load_channels(false).await });
    });
    bind!(g.on_channel_selected, page, |i| {
        {
            let mut st = page.st.borrow_mut();
            st.channel = st.channel_list.get(i as usize).cloned();
            if let (Some(p), Some(c)) = (st.project_key.clone(), st.channel.clone()) {
                st.last_channel.insert(p.to_lowercase(), c);
            }
        }
        spawn(async move { page.load_posts(false).await });
    });
    bind!(g.on_unsolved_toggled, page, |_c| spawn(async move { page.load_posts(false).await }));
    bind!(g.on_post_selected, page, |i| {
        let post = page.st.borrow().posts.get(i as usize).cloned();
        if let Some(post) = post {
            spawn(async move { page.open(post, false).await });
        }
    });
    bind!(g.on_reply, page, || spawn(async move { page.reply().await }));
    bind!(g.on_create, page, || spawn(async move { page.create().await }));
    bind!(g.on_add, page, || {
        let Some(channel) = page.st.borrow().channel.clone() else { return };
        page.with(|g| {
            g.set_create_label(format!("在「{channel}」發文").into());
            g.set_create_open(true);
            g.set_focus_new_title(g.get_focus_new_title() + 1);
        });
    });
    bind!(g.on_cancel_create, page, || {
        page.with(|g| g.set_create_open(false));
    });
    bind!(g.on_reload, page, || spawn(async move { page.refresh().await }));
    bind!(g.on_web, page, || {
        let key = page.st.borrow().project_key.clone();
        let path = match key {
            Some(k) => format!("/projects/{}", escape_data(&k)),
            None => "/".to_owned(),
        };
        hb_win::open(&page.ctx.engine.web_url(&path));
    });
    bind!(g.on_new_title_edited, page, |text| {
        if text.chars().count() > TITLE_MAX {
            let cut: String = text.chars().take(TITLE_MAX).collect();
            page.with(|g| g.set_new_title(cut.into()));
        }
    });
    wire_md!(&g);
    page.show_thread(None);
}

/// arg：專案代碼（例如從論壇通知點過來）時切到那個專案。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    let wanted = arg.as_ref().and_then(|a| a.as_text()).map(str::to_owned);
    {
        let st = page.st.borrow();
        let same = match (&wanted, &st.project_key) {
            (None, _) => true,
            (Some(w), Some(k)) => w.eq_ignore_ascii_case(k),
            (Some(_), None) => false,
        };
        if st.loaded && same {
            return;
        }
    }
    page.st.borrow_mut().loaded = true;
    page.render_projects(wanted.as_deref());
    page.load_channels(false).await;
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&ForumPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<ForumPageData>();
        Some(f(&g))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_status(text);
            g.set_status_tone(tone);
        });
    }

    fn left(&self, text: &str) {
        self.with(|g| g.set_left_message(text.into()));
    }

    fn next_version(&self) -> u64 {
        let mut st = self.st.borrow_mut();
        st.version += 1;
        st.version
    }

    fn set_posts(&self, posts: Vec<ForumPost>, rows: Vec<W3Row>, current: i32) {
        self.st.borrow_mut().posts = posts;
        self.with(|g| {
            g.set_posts(model(rows));
            g.set_post_current(current);
        });
    }

    fn render_projects(&self, wanted: Option<&str>) {
        let (names, index) = {
            let mut st = self.st.borrow_mut();
            let selected = wanted.map(str::to_owned).or_else(|| st.project_key.clone());
            st.projects = self.ctx.engine.projects().iter().filter(|p| !p.is_archived).cloned().collect();
            let index = st
                .projects
                .iter()
                .position(|p| selected.as_deref().is_some_and(|s| p.key.eq_ignore_ascii_case(s)))
                .or(if st.projects.is_empty() { None } else { Some(0) });
            st.project_key = index.map(|i| st.projects[i].key.clone());
            (st.projects.iter().map(|p| p.name.clone()).collect::<Vec<_>>(), index)
        };
        let empty = names.is_empty();
        self.with(|g| {
            g.set_projects(strings(names));
            g.set_project_index(index.map_or(-1, |i| i as i32));
        });
        self.render_project_icon();
        if empty {
            self.left("沒有看得到的專案");
        }
    }

    /// 選著的專案的圖示（C# 下拉選單每一項是 ProjectIcons.Badge）。
    fn render_project_icon(&self) {
        let icon = {
            let st = self.st.borrow();
            st.projects
                .iter()
                .find(|p| Some(&p.key) == st.project_key.as_ref())
                .map(|p| ProjectIcons::icon(Some(p), &p.name))
        };
        self.with(|g| match icon {
            Some(icon) => {
                g.set_project_glyph(icon.glyph.into());
                g.set_project_tint(icon.color.to_slint());
            }
            None => g.set_project_glyph("".into()),
        });
    }

    /// 重新整理：清掉快取，重抓目前專案的頻道與貼文。
    async fn refresh(self: Rc<Self>) {
        {
            let mut st = self.st.borrow_mut();
            st.channels.clear();
            st.post_cache.clear();
            st.threads.clear();
        }
        self.render_projects(None);
        self.load_channels(true).await;
    }

    async fn load_channels(self: Rc<Self>, refresh: bool) {
        let Some(project) = self.st.borrow().project_key.clone() else { return };
        let cached = if refresh { None } else { self.st.borrow().channels.get(&project.to_lowercase()).cloned() };
        let channels = match cached {
            Some(c) => c,
            None => {
                let version = self.next_version();
                self.show(None);
                self.left("讀取中…");
                self.set_posts(Vec::new(), Vec::new(), -1);
                match self.ctx.engine.api().list_channels(&project).await {
                    Ok(channels) => {
                        if version != self.st.borrow().version
                            || self.st.borrow().project_key.as_ref() != Some(&project)
                        {
                            return;
                        }
                        self.st.borrow_mut().channels.insert(project.to_lowercase(), channels.clone());
                        channels
                    }
                    Err(e) => {
                        if version != self.st.borrow().version {
                            return;
                        }
                        log::error!("讀取 {project} 的頻道失敗: {e}");
                        self.left("");
                        self.show(Some(StatusMessage::error(format!("讀不到頻道：{e}"))));
                        return;
                    }
                }
            }
        };
        // 上次在這個專案選的；沒有就挑名稱像論壇的，再沒有就第一個
        let last = self.st.borrow().last_channel.get(&project.to_lowercase()).cloned();
        let selected = pick_channel(&channels, last.as_deref());
        let index = selected.as_ref().and_then(|s| channels.iter().position(|c| c == s));
        {
            let mut st = self.st.borrow_mut();
            st.channel_list = channels.clone();
            st.channel = selected;
        }
        self.with(|g| {
            g.set_channels(strings(&channels));
            g.set_channel_index(index.map_or(-1, |i| i as i32));
        });
        if channels.is_empty() {
            self.left("這個專案沒有頻道");
            self.set_posts(Vec::new(), Vec::new(), -1);
            self.show_thread(None);
            return;
        }
        self.load_posts(refresh).await;
    }

    async fn load_posts(self: Rc<Self>, refresh: bool) {
        let (Some(project), Some(channel)) = ({
            let st = self.st.borrow();
            (st.project_key.clone(), st.channel.clone())
        }) else {
            return;
        };
        let unsolved = self.with(|g| g.get_unsolved()).unwrap_or(false);
        let cache_key = format!("{project}\n{channel}\n{}", if unsolved { "True" } else { "False" });
        let cached = if refresh { None } else { self.st.borrow().post_cache.get(&cache_key).cloned() };
        if let Some(posts) = cached {
            self.render_posts(posts, unsolved);
            return;
        }
        let version = self.next_version();
        self.show(None);
        self.left("讀取中…");
        self.set_posts(Vec::new(), Vec::new(), -1);
        let result = self.ctx.engine.api().list_forum_posts(&project, &channel, unsolved).await;
        match result {
            Ok(posts) => {
                let same = {
                    let st = self.st.borrow();
                    version == st.version
                        && st.project_key.as_ref() == Some(&project)
                        && st.channel.as_ref() == Some(&channel)
                };
                if !same {
                    return;
                }
                self.st.borrow_mut().post_cache.insert(cache_key, posts.clone());
                self.render_posts(posts, unsolved);
            }
            Err(e) if e.0.contains("NOT_FOUND") || e.0.contains("找不到這個論壇") => {
                if version != self.st.borrow().version {
                    return;
                }
                self.left(&format!("「{channel}」是聊天頻道，不是論壇；聊天請到「聊天」頁"));
                self.show_thread(None);
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取 {project} {channel} 的貼文失敗: {e}");
                self.left("");
                self.show(Some(StatusMessage::error(format!("讀不到貼文：{e}"))));
            }
        }
    }

    fn render_posts(self: &Rc<Self>, posts: Vec<ForumPost>, unsolved: bool) {
        let rows: Vec<W3Row> = posts.iter().map(post_row).collect();
        let current_id = self.st.borrow().post.as_ref().map(|p| p.id.clone());
        let index = posts.iter().position(|p| Some(&p.id) == current_id.as_ref());
        let message = if posts.is_empty() {
            if unsolved {
                "沒有未解決的貼文".to_owned()
            } else {
                "還沒有貼文，按「＋ 發文」".to_owned()
            }
        } else {
            format!("{} 篇", posts.len())
        };
        let selected = index.and_then(|i| posts.get(i).cloned());
        self.set_posts(posts, rows, index.map_or(-1, |i| i as i32));
        self.left(&message);
        match selected {
            // C# 選回上次那篇會觸發 SelectionChanged → OpenAsync
            Some(post) => {
                let page = self.clone();
                spawn(async move { page.open(post, false).await });
            }
            None => self.show_thread(None),
        }
    }

    async fn open(self: Rc<Self>, post: ForumPost, refresh: bool) {
        let (Some(project), Some(channel)) = ({
            let st = self.st.borrow();
            (st.project_key.clone(), st.channel.clone())
        }) else {
            return;
        };
        self.st.borrow_mut().post = Some(post.clone());
        let cached = if refresh { None } else { self.st.borrow().threads.get(&post.id).cloned() };
        if let Some(thread) = cached {
            self.show_thread(Some(&thread));
            return;
        }
        let version = self.next_version();
        self.with(|g| {
            g.set_title(post.title.as_str().into());
            g.set_meta("".into());
            g.set_body_visible(false);
            g.set_replies(model(Vec::new()));
            g.set_thread_visible(false);
            g.set_right_message("讀取中…".into());
            g.set_right_visible(true);
        });
        match self.ctx.engine.api().get_forum_post(&project, &channel, &post.id).await {
            Ok(thread) => {
                self.st.borrow_mut().threads.insert(post.id.clone(), thread.clone());
                if version != self.st.borrow().version
                    || self.st.borrow().post.as_ref().map(|p| &p.id) != Some(&post.id)
                {
                    return;
                }
                self.show_thread(Some(&thread));
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取貼文 {} 失敗: {e}", post.id);
                self.with(|g| g.set_right_message(format!("讀不到這篇：{e}").into()));
            }
        }
    }

    fn show_thread(&self, thread: Option<&ForumThread>) {
        let post = self.st.borrow().post.clone();
        let (Some(thread), Some(post)) = (thread, post) else {
            if thread.is_none() {
                self.st.borrow_mut().post = None;
            }
            self.with(|g| {
                g.set_replies(model(Vec::new()));
                g.set_title("".into());
                g.set_meta("".into());
                g.set_body_visible(false);
                g.set_thread_visible(false);
                g.set_right_message("左邊選一篇貼文".into());
                g.set_right_visible(true);
            });
            return;
        };
        let title = format!(
            "{}{}",
            if thread.solved { "✓ " } else { "" },
            if thread.title.is_empty() { &post.title } else { &thread.title }
        );
        let body = clean(thread.root.as_ref().map_or("", |r| r.body.as_str()));
        let replies: Vec<ForumReply> = thread
            .replies
            .iter()
            .map(|r| ForumReply {
                head: format!("{}・{}", r.author, r.created_at.as_ref().map(inputs::ago).unwrap_or_default()).into(),
                body: if r.deleted { "（回應已刪除）".to_owned() } else { clean(&r.body) }.into(),
            })
            .collect();
        let head = if thread.replies.is_empty() {
            "還沒有回應".to_owned()
        } else {
            format!("回應（{}）", thread.replies.len())
        };
        self.with(|g| {
            g.set_title(title.into());
            g.set_meta(thread_meta(thread).into());
            g.set_body_visible(!body.is_empty());
            g.set_body(body.into());
            g.set_right_visible(false);
            g.set_replies_head(head.into());
            g.set_replies(model(replies));
            g.set_thread_visible(true);
        });
    }

    async fn reply(self: Rc<Self>) {
        let (Some(project), Some(channel), Some(post)) = ({
            let st = self.st.borrow();
            (st.project_key.clone(), st.channel.clone(), st.post.clone())
        }) else {
            return;
        };
        let body = self.with(|g| g.get_reply_text().trim().to_owned()).unwrap_or_default();
        if body.is_empty() {
            self.show(Some(StatusMessage::error("請先填回覆內容")));
            self.with(|g| g.set_focus_reply(g.get_focus_reply() + 1));
            return;
        }
        let api = self.ctx.engine.api();
        let p = self.clone();
        let id = post.id.clone();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_replying(b));
            },
            |m| p.show(m),
            || async move { api.reply_forum_post(&project, &channel, &id, &body).await.map_err(ActionError::from) },
            Some("已回覆"),
        )
        .await;
        if !ok {
            return;
        }
        self.with(|g| g.set_reply_text("".into()));
        self.st.borrow_mut().post_cache.clear(); // 回應數、最新活動變了
        self.open(post, true).await;
    }

    async fn create(self: Rc<Self>) {
        let (Some(project), Some(channel)) = ({
            let st = self.st.borrow();
            (st.project_key.clone(), st.channel.clone())
        }) else {
            return;
        };
        let Some((title, body)) =
            self.with(|g| (g.get_new_title().trim().to_owned(), g.get_new_body().trim().to_owned()))
        else {
            return;
        };
        if title.is_empty() {
            self.show(Some(StatusMessage::error("請填標題")));
            self.with(|g| g.set_focus_new_title(g.get_focus_new_title() + 1));
            return;
        }
        let api = self.ctx.engine.api();
        let p = self.clone();
        let t = title.clone();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_creating(b));
            },
            |m| p.show(m),
            || async move { api.create_forum_post(&project, &channel, &t, &body).await.map_err(ActionError::from) },
            Some(&format!("已發文「{title}」")),
        )
        .await;
        if !ok {
            return;
        }
        self.with(|g| {
            g.set_new_title("".into());
            g.set_new_body("".into());
            g.set_create_open(false);
        });
        let message = self.with(|g| g.get_status().to_string()).unwrap_or_default();
        self.clone().load_posts(true).await;
        self.show(Some(StatusMessage::ok(message)));
    }
}

/// 上次在這個專案選的；沒有就挑名稱像論壇的，再沒有就第一個。
pub(crate) fn pick_channel(channels: &[String], last: Option<&str>) -> Option<String> {
    if let Some(last) = last.filter(|l| channels.iter().any(|c| c == l)) {
        return Some(last.to_owned());
    }
    channels
        .iter()
        .find(|c| c.contains("論壇") || c.to_lowercase().contains("forum"))
        .or_else(|| channels.first())
        .cloned()
}

fn post_row(p: &ForumPost) -> W3Row {
    W3Row {
        title: format!("{}{}{}", if p.pinned { "📌 " } else { "" }, if p.solved { "✓ " } else { "" }, p.title).into(),
        meta: post_meta(p).into(),
        ..Default::default()
    }
}

pub(crate) fn post_meta(p: &ForumPost) -> String {
    let mut parts =
        vec![p.author.clone(), if p.replies > 0 { format!("{} 則回應", p.replies) } else { "還沒有回應".to_owned() }];
    if let Some(at) = p.last_activity_at.as_ref().or(p.created_at.as_ref()) {
        parts.push(inputs::ago(at));
    }
    if !p.tags.is_empty() {
        parts.push(p.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "));
    }
    parts.into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join("・")
}

fn thread_meta(thread: &ForumThread) -> String {
    let mut meta = Vec::new();
    if let Some(root) = &thread.root {
        meta.push(root.author.clone());
        if let Some(at) = &root.created_at {
            meta.push(inputs::ago(at));
        }
    }
    if thread.solved {
        meta.push("已解決".to_owned());
    }
    if !thread.tags.is_empty() {
        meta.push(thread.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" "));
    }
    meta.join("・")
}

/// Markdown 的強制換行（行尾反斜線）照一般換行顯示。
pub(crate) fn clean(markdown: &str) -> String {
    markdown.replace("\\\n", "\n").trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(replies: i32, tags: &[&str]) -> ForumPost {
        ForumPost {
            id: "1".into(),
            title: "問題".into(),
            author: "Amy".into(),
            created_at: None,
            replies,
            last_activity_at: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            pinned: true,
            solved: true,
        }
    }

    #[test]
    fn post_meta_與標題() {
        assert_eq!("Amy・還沒有回應", post_meta(&post(0, &[])));
        assert_eq!("Amy・3 則回應・#bug #ui", post_meta(&post(3, &["bug", "ui"])));
        assert_eq!("📌 ✓ 問題", post_row(&post(0, &[])).title.as_str());
    }

    #[test]
    fn clean_行尾反斜線換行() {
        assert_eq!("第一行\n第二行", clean("  第一行\\\n第二行\n"));
    }

    #[test]
    fn pick_channel_上次選的_論壇_第一個() {
        let channels: Vec<String> = vec!["一般".into(), "產品論壇".into(), "Forum-en".into()];
        assert_eq!(Some("Forum-en".to_owned()), pick_channel(&channels, Some("Forum-en")));
        assert_eq!(Some("產品論壇".to_owned()), pick_channel(&channels, Some("不存在")));
        assert_eq!(Some("x".to_owned()), pick_channel(&["x".to_owned()], None));
        assert_eq!(None, pick_channel(&[], None));
    }
}
