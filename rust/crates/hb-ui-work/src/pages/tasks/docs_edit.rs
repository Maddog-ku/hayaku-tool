//! 任務詳細「文件」區塊的寫入（跟上 Hayaku 1.5.12）：每一份文件都能〔＋ 補一段〕（append_to_content_document）、
//! 〔✎ 改一段〕只換某個標題底下那一段（update_content_document 的 section），也能〔＋ 新增文件〕（create_content_document）。
//! 整份取代不做（會蓋掉別人在網頁上的圖片、附件），要整份改請到網頁。都是按了才打 Hayaku。

use std::cell::RefCell;
use std::rc::Rc;

use hb_ui_common::inputs::{self, ActionError, StatusMessage};

use super::Page;
use crate::TasksPageData;
use crate::shared::{self, string_model};

/// 編輯區的模式（TasksPageData.d-doc-edit-mode）。
const APPEND: i32 = 1;
const SECTION: i32 = 2;
const CREATE: i32 = 3;

/// 文件名稱最多幾個字（create_content_document 的 title）。
const TITLE_MAX: usize = 30;

impl Page {
    pub(super) fn wire_docs_edit(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_d_doc_edit_open, |p, mode, index| p.open_doc_edit(mode, index));
        on!(d, self, on_d_doc_edit_section_picked, |p, index| p.pick_section(index));
        on!(d, self, on_d_doc_edit_cancel, |p| p.close_doc_edit());
        on!(d, self, on_d_doc_edit_submit, |p| shared::spawn("寫入文件", async move { p.submit_doc_edit().await }));
    }

    /// 收起編輯區（換任務、取消、寫完）。
    pub(super) fn close_doc_edit(&self) {
        self.st.borrow_mut().doc_sections.clear();
        self.with_data(|d| {
            d.set_d_doc_edit_mode(0);
            d.set_d_doc_edit_index(-1);
            d.set_d_doc_edit_title(Default::default());
            d.set_d_doc_edit_text(Default::default());
            d.set_d_doc_edit_sections(string_model(Vec::<String>::new()));
            d.set_d_doc_edit_section_index(0);
        });
    }

    fn open_doc_edit(&self, mode: i32, index: i32) {
        let sections = if mode == SECTION {
            let st = self.st.borrow();
            match usize::try_from(index).ok().and_then(|i| st.docs.get(i)) {
                Some(doc) => doc.editable_sections(),
                None => return,
            }
        } else {
            Vec::new()
        };
        if mode == SECTION && sections.is_empty() {
            return;
        }
        let first_body = sections.first().map(|s| s.body.clone()).unwrap_or_default();
        let names: Vec<String> = sections.iter().map(|s| s.heading.clone()).collect();
        self.st.borrow_mut().doc_sections = sections;
        self.show_msg(None);
        self.with_data(|d| {
            d.set_d_doc_edit_mode(mode);
            d.set_d_doc_edit_index(if mode == CREATE { -1 } else { index });
            d.set_d_doc_edit_title(Default::default());
            d.set_d_doc_edit_sections(string_model(names));
            d.set_d_doc_edit_section_index(0);
            // 改一段：先帶出那一段目前的內容，改完整段換掉
            d.set_d_doc_edit_text(first_body.into());
        });
    }

    fn pick_section(&self, index: i32) {
        let body = usize::try_from(index).ok().and_then(|i| self.st.borrow().doc_sections.get(i).map(|s| s.body.clone()));
        if let Some(body) = body {
            self.with_data(|d| d.set_d_doc_edit_text(body.into()));
        }
    }

    async fn submit_doc_edit(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        let Some((mode, index, title, text, section_index)) = self.with_data(|d| {
            (
                d.get_d_doc_edit_mode(),
                d.get_d_doc_edit_index(),
                d.get_d_doc_edit_title().trim().to_owned(),
                d.get_d_doc_edit_text().to_string(),
                d.get_d_doc_edit_section_index(),
            )
        }) else {
            return;
        };
        let (doc_id, doc_title) = {
            let st = self.st.borrow();
            match usize::try_from(index).ok().and_then(|i| st.docs.get(i)) {
                Some(doc) => (doc.id.clone(), doc.title.clone()),
                None => (None, String::new()),
            }
        };
        let section =
            usize::try_from(section_index).ok().and_then(|i| self.st.borrow().doc_sections.get(i).map(|s| s.heading.clone()));
        // 先在本機擋掉送了一定會失敗的
        let invalid = match mode {
            APPEND if text.trim().is_empty() => Some("要補的內容是空的".to_owned()),
            APPEND | SECTION if doc_id.is_none() => Some("找不到這份文件，請重新展開文件".to_owned()),
            SECTION if section.is_none() => Some("請先選一個標題".to_owned()),
            CREATE if title.is_empty() => Some("請填文件名稱".to_owned()),
            CREATE if title.chars().count() > TITLE_MAX => Some(format!("文件名稱最多 {TITLE_MAX} 個字")),
            APPEND | SECTION | CREATE => None,
            _ => return,
        };
        if let Some(e) = invalid {
            self.show_msg(Some(StatusMessage::error(e)));
            return;
        }
        let api = self.engine().api();
        let message: Rc<RefCell<Option<String>>> = Rc::default();
        let (m, k) = (message.clone(), key.clone());
        let (id, sec, t) = (doc_id.unwrap_or_default(), section.clone().unwrap_or_default(), title.clone());
        let ok = inputs::run(
            |busy| {
                self.with_data(|d| d.set_d_doc_edit_busy(busy));
            },
            |msg| self.show_msg(msg),
            move || async move {
                let r = match mode {
                    APPEND => api.append_content_document(&k, &id, text.trim()).await,
                    SECTION => api.update_content_section(&k, &id, &sec, text.trim_end()).await,
                    _ => api.create_content_document(&k, &t, text.trim()).await,
                };
                *m.borrow_mut() = r.map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if !ok {
            return;
        }
        let fallback = match mode {
            APPEND => format!("已補在「{doc_title}」最後"),
            SECTION => format!("已換掉「{doc_title}」的「{}」那一段", section.unwrap_or_default()),
            _ => format!("已在 {key} 新增文件「{title}」"),
        };
        let text = message.borrow().clone().filter(|m| !m.is_empty()).unwrap_or(fallback);
        self.close_doc_edit();
        self.show_msg(Some(StatusMessage::ok(text)));
        // 文件內容、份數（標題的「文件（n）」）、備註（第一份文件）都可能變了：重讀一次
        self.refresh_detail_quietly(&key).await;
        self.st.borrow_mut().docs_for = None;
        self.clone().load_docs().await;
    }
}
