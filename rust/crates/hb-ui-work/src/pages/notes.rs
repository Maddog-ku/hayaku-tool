//! 工作台「筆記」頁（對應 C# `UI/Work/NotesPage.cs`）。
//! 我的筆記本：左邊筆記清單（可搜尋頁名與內容），右邊看選中那頁的 Markdown 與待辦；
//! 可以新增一頁、在選中那頁最後附加內容。第一次切到這頁才讀，之後用快取，按重新整理才重抓。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use hb_api::{NoteContent, NotePage, Notebook};
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use slint::ComponentHandle;

use super::meetings::w3_common::{model, spawn, status_parts, wire_md};
use crate::{NotesPageData, PageArg, W3Row, WorkCtx};

struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
}

#[derive(Default)]
struct State {
    cache: HashMap<String, NoteContent>,
    notebook: Option<Notebook>,
    selected: Option<String>,
    /// 清單上每一列的頁 id（跟 rows 模型同順序）
    row_ids: Vec<String>,
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
    let g = w.global::<NotesPageData>();
    bind!(g.on_search_accepted, page, || spawn(async move { page.search().await }));
    bind!(g.on_search_escape, page, || {
        page.with(|g| g.set_search("".into()));
        page.render_notebook();
    });
    bind!(g.on_row_selected, page, |i| {
        let id = page.st.borrow().row_ids.get(i as usize).cloned();
        if let Some(id) = id {
            spawn(async move { page.open(id).await });
        }
    });
    bind!(g.on_append, page, || spawn(async move { page.append().await }));
    bind!(g.on_create, page, || spawn(async move { page.create().await }));
    bind!(g.on_add, page, || {
        page.with(|g| {
            g.set_create_open(true);
            g.set_focus_new_title(g.get_focus_new_title() + 1);
        });
    });
    bind!(g.on_cancel_create, page, || {
        page.with(|g| g.set_create_open(false));
    });
    bind!(g.on_reload, page, || spawn(async move { page.load().await }));
    bind!(g.on_web, page, || hb_win::open(&page.ctx.engine.web_url("/notebook")));
    wire_md!(&g);
    page.show_note(None);
}

pub async fn activate(_ctx: WorkCtx, _arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    if !page.st.borrow().loaded {
        page.load().await;
    }
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&NotesPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<NotesPageData>();
        Some(f(&g))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_status(text);
            g.set_status_tone(tone);
        });
    }

    fn next_version(&self) -> u64 {
        let mut st = self.st.borrow_mut();
        st.version += 1;
        st.version
    }

    fn set_rows(&self, rows: Vec<(String, W3Row)>, current: i32) {
        let (ids, rows): (Vec<String>, Vec<W3Row>) = rows.into_iter().unzip();
        self.st.borrow_mut().row_ids = ids;
        self.with(|g| {
            g.set_rows(model(rows));
            g.set_current(current);
        });
    }

    async fn load(self: Rc<Self>) {
        let version = self.next_version();
        self.st.borrow_mut().cache.clear();
        self.show(None);
        self.with(|g| g.set_list_message("讀取中…".into()));
        self.set_rows(Vec::new(), -1);
        match self.ctx.engine.api().list_notes().await {
            Ok(notebook) => {
                if version != self.st.borrow().version {
                    return;
                }
                {
                    let mut st = self.st.borrow_mut();
                    st.notebook = Some(notebook);
                    st.loaded = true;
                }
                self.with(|g| g.set_search("".into()));
                // 上次選的那頁還在的話會選回去，選取時才讀內容
                if !self.render_notebook() {
                    self.show_note(None);
                }
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取筆記本失敗: {e}");
                self.with(|g| g.set_list_message("".into()));
                self.show(Some(StatusMessage::error(format!("讀不到筆記本：{e}"))));
            }
        }
    }

    /// 照資料夾分組列出（沒有資料夾的放最前面），新的在前；回傳有沒有選回上次選的那頁（C# SelectionChanged 會打開它）。
    fn render_notebook(self: &Rc<Self>) -> bool {
        let Some(nb) = self.st.borrow().notebook.clone() else { return false };
        let selected = self.st.borrow().selected.clone();
        let rows = notebook_rows(&nb);
        let index = rows.iter().position(|(id, _)| Some(id) == selected.as_ref());
        let counts = if nb.inbox > 0 || nb.todos > 0 {
            format!("隨手記 {}・待辦 {}", nb.inbox, nb.todos)
        } else {
            String::new()
        };
        let message = if rows.is_empty() {
            "還沒有筆記，按「＋ 新增筆記」寫第一頁".to_owned()
        } else {
            format!("{} 頁", rows.len()) + &if counts.is_empty() { String::new() } else { format!("　{counts}") }
        };
        self.set_rows(rows, index.map_or(-1, |i| i as i32));
        self.with(|g| g.set_list_message(message.into()));
        if let (Some(_), Some(id)) = (index, selected) {
            let page = self.clone();
            spawn(async move { page.open(id).await });
            return true;
        }
        false
    }

    async fn search(self: Rc<Self>) {
        let q = self.with(|g| g.get_search().trim().to_owned()).unwrap_or_default();
        if q.is_empty() {
            self.render_notebook();
            return;
        }
        let version = self.next_version();
        self.with(|g| g.set_list_message("搜尋中…".into()));
        self.set_rows(Vec::new(), -1);
        match self.ctx.engine.api().search_notes(&q).await {
            Ok(hits) => {
                if version != self.st.borrow().version {
                    return;
                }
                let n = hits.len();
                let rows = hits
                    .into_iter()
                    .map(|h| {
                        let row = W3Row {
                            title: h.title.as_str().into(),
                            meta: h.snippet.unwrap_or_default().into(),
                            ..Default::default()
                        };
                        (h.id, row)
                    })
                    .collect();
                self.set_rows(rows, -1);
                let message = if n == 0 {
                    format!("找不到「{q}」（Esc 回到全部）")
                } else {
                    format!("「{q}」找到 {n} 頁（Esc 回到全部）")
                };
                self.with(|g| g.set_list_message(message.into()));
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("搜尋筆記失敗: {e}");
                self.with(|g| g.set_list_message("".into()));
                self.show(Some(StatusMessage::error(format!("搜尋失敗：{e}"))));
            }
        }
    }

    async fn open(self: Rc<Self>, id: String) {
        self.st.borrow_mut().selected = Some(id.clone());
        let cached = self.st.borrow().cache.get(&id).cloned();
        if let Some(note) = cached {
            self.show_note(Some(&note));
            return;
        }
        let version = self.next_version();
        let title = self
            .st
            .borrow()
            .notebook
            .as_ref()
            .and_then(|nb| nb.pages.iter().find(|p| p.id == id).map(|p| p.title.clone()))
            .unwrap_or_default();
        self.with(|g| {
            g.set_title(title.into());
            g.set_meta("".into());
            g.set_content_visible(false);
            g.set_content_message("讀取中…".into());
        });
        match self.ctx.engine.api().get_note(&id).await {
            Ok(note) => {
                self.st.borrow_mut().cache.insert(id.clone(), note.clone());
                if version != self.st.borrow().version || self.st.borrow().selected.as_ref() != Some(&id) {
                    return;
                }
                self.show_note(Some(&note));
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取筆記 {id} 失敗: {e}");
                self.with(|g| g.set_content_message(format!("讀不到這頁：{e}").into()));
            }
        }
    }

    fn show_note(&self, note: Option<&NoteContent>) {
        let Some(note) = note else {
            let empty_book = self.st.borrow().notebook.as_ref().is_some_and(|nb| nb.pages.is_empty());
            self.with(|g| {
                g.set_append_visible(false);
                g.set_title("".into());
                g.set_meta("".into());
                g.set_content_visible(false);
                g.set_content_message(if empty_book { "" } else { "左邊選一頁筆記" }.into());
            });
            return;
        };
        let meta = if note.task_codes.is_empty() {
            String::new()
        } else {
            format!("相關任務：{}", note.task_codes.join("、"))
        };
        let text = note_text(note);
        self.with(|g| {
            g.set_append_visible(true);
            g.set_title(note.title.as_str().into());
            g.set_meta(meta.into());
            g.set_content_visible(!text.is_empty());
            g.set_content(text.into());
            g.set_content_message("這頁還沒有內容".into());
        });
    }

    async fn append(self: Rc<Self>) {
        let Some(id) = self.st.borrow().selected.clone() else { return };
        let markdown = self.with(|g| g.get_append_text().trim().to_owned()).unwrap_or_default();
        if markdown.is_empty() {
            self.show(Some(StatusMessage::error("請先填要附加的內容")));
            self.with(|g| g.set_focus_append(g.get_focus_append() + 1));
            return;
        }
        let api = self.ctx.engine.api();
        let p = self.clone();
        let target = id.clone();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_appending(b));
            },
            |m| p.show(m),
            || async move { api.append_note(&target, &markdown).await.map_err(ActionError::from) },
            Some("已附加到這頁"),
        )
        .await;
        if !ok {
            return;
        }
        self.with(|g| g.set_append_text("".into()));
        self.st.borrow_mut().cache.remove(&id);
        self.open(id).await;
    }

    async fn create(self: Rc<Self>) {
        let Some((title, body)) =
            self.with(|g| (g.get_new_title().trim().to_owned(), g.get_new_body().trim().to_owned()))
        else {
            return;
        };
        if title.is_empty() {
            self.show(Some(StatusMessage::error("請填名稱")));
            self.with(|g| g.set_focus_new_title(g.get_focus_new_title() + 1));
            return;
        }
        let api = self.ctx.engine.api();
        let p = self.clone();
        let created: Rc<RefCell<Option<String>>> = Rc::default();
        let c = created.clone();
        let t = title.clone();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_creating(b));
            },
            |m| p.show(m),
            || async move {
                *c.borrow_mut() = api.create_note(&t, &body).await?;
                Ok::<(), ActionError>(())
            },
            Some(&format!("已新增「{title}」")),
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
        self.st.borrow_mut().selected = created.borrow_mut().take();
        let message = self.with(|g| g.get_status().to_string()).unwrap_or_default();
        self.clone().load().await;
        self.show(Some(StatusMessage::ok(message)));
    }
}

/// 筆記本的清單（C# RenderNotebook）：沒有資料夾的放最前面，再照資料夾名稱，同一組新的在前。回傳 (頁 id, 列)。
pub(crate) fn notebook_rows(nb: &Notebook) -> Vec<(String, W3Row)> {
    let folders: HashMap<&str, &str> = nb.folders.iter().map(|f| (f.id.as_str(), f.name.as_str())).collect();
    let folder_name =
        |p: &NotePage| p.folder_id.as_deref().map(|f| folders.get(f).copied().unwrap_or("")).unwrap_or("");
    let mut pages: Vec<&NotePage> = nb.pages.iter().collect();
    pages.sort_by(|a, b| {
        a.folder_id
            .is_some()
            .cmp(&b.folder_id.is_some())
            .then_with(|| folder_name(a).cmp(folder_name(b)))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });
    pages
        .into_iter()
        .map(|p| {
            (
                p.id.clone(),
                W3Row { title: p.title.as_str().into(), meta: note_meta(p, &folders).into(), ..Default::default() },
            )
        })
        .collect()
}

fn note_meta(p: &NotePage, folders: &HashMap<&str, &str>) -> String {
    let mut parts = Vec::new();
    if let Some(folder) = p.folder_id.as_deref().and_then(|f| folders.get(f)) {
        parts.push(format!("📁 {folder}"));
    }
    if let Some(at) = &p.updated_at {
        parts.push(inputs::ago(at));
    }
    if !p.task_codes.is_empty() {
        parts.push(p.task_codes.join("、"));
    }
    parts.join("・")
}

/// 這頁要顯示的文字：Markdown＋待辦（☑／☐）。
pub(crate) fn note_text(note: &NoteContent) -> String {
    let mut text = note.markdown.trim().to_owned();
    if !note.todos.is_empty() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str("待辦\n");
        text.push_str(
            &note
                .todos
                .iter()
                .map(|(t, done)| format!("{} {t}", if *done { "☑" } else { "☐" }))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, TimeZone};
    use hb_api::NoteFolder;

    fn page(id: &str, folder: Option<&str>, day: u32) -> NotePage {
        NotePage {
            id: id.into(),
            title: id.to_uppercase(),
            folder_id: folder.map(str::to_owned),
            updated_at: Some(FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2020, 1, day, 0, 0, 0).unwrap()),
            task_codes: if id == "a" { vec!["F2P-1".into(), "F2P-2".into()] } else { vec![] },
        }
    }

    #[test]
    fn notebook_rows_沒有資料夾的在前_新的在前() {
        let nb = Notebook {
            folders: vec![
                NoteFolder { id: "f2".into(), name: "乙".into() },
                NoteFolder { id: "f1".into(), name: "甲".into() },
            ],
            pages: vec![page("a", Some("f2"), 1), page("b", None, 1), page("c", Some("f1"), 2), page("d", None, 3)],
            inbox: 0,
            todos: 0,
        };
        let ids: Vec<_> = notebook_rows(&nb).into_iter().map(|(id, _)| id).collect();
        // 甲（U+7532）排在乙（U+4E59）後面：照字串比較
        assert_eq!(vec!["d", "b", "a", "c"], ids);
        let rows = notebook_rows(&nb);
        assert!(rows[2].1.meta.starts_with("📁 乙・"));
        assert!(rows[2].1.meta.ends_with("・F2P-1、F2P-2"));
    }

    #[test]
    fn note_text_加上待辦() {
        let note = NoteContent {
            id: "x".into(),
            title: "t".into(),
            markdown: "  內容 \n".into(),
            task_codes: vec![],
            todos: vec![("買牛奶".into(), true), ("寫報告".into(), false)],
        };
        assert_eq!("內容\n\n待辦\n☑ 買牛奶\n☐ 寫報告", note_text(&note));
        let empty = NoteContent { markdown: "".into(), todos: vec![], ..note };
        assert_eq!("", note_text(&empty));
    }
}
