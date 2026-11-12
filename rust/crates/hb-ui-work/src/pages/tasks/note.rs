//! 對應 C# `UI/Work/TasksPage.Note.cs`：任務詳細的〔＋ 補一段〕：在任務內容最後補一段 Markdown（append_to_task_note），
//! 原本的圖片、附件、表格都不動。按了才打 Hayaku。

use std::rc::Rc;

use hb_api::TaskDetail;
use hb_ui_common::MarkdownTable;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};

use super::Page;
use crate::TasksPageData;
use crate::shared;

impl Page {
    pub(super) fn wire_note(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_d_note_append_toggle, |p| {
            p.with_data(|d| d.set_d_note_append_open(!d.get_d_note_append_open()));
        });
        on!(d, self, on_d_note_append_submit, |p| shared::spawn("補一段", async move { p.append_note().await }));
        on!(d, self, on_d_note_table, |p, anchor, cursor| p.insert_table(anchor, cursor));
    }

    /// 換任務時收起補一段；同一個任務重新讀取時保留打到一半的內容。
    pub(super) fn reset_note_append(&self, d: &TaskDetail) {
        {
            let mut st = self.st.borrow_mut();
            if st.note_append_for.as_deref() == Some(d.key.as_str()) {
                return;
            }
            st.note_append_for = Some(d.key.clone());
        }
        self.with_data(|x| {
            x.set_d_note_append_open(false);
            x.set_d_note_append_text(Default::default());
        });
    }

    /// 〔▦ 表格〕：在游標的位置插入空白表格（3 欄 2 列）。
    fn insert_table(&self, anchor: i32, cursor: i32) {
        self.with_data(|d| {
            let text = d.get_d_note_append_text().to_string();
            let (a, c) = (anchor.max(0) as usize, cursor.max(0) as usize);
            let (new_text, pos) = MarkdownTable::insert(&text, a.min(c)..a.max(c), &MarkdownTable::template(3, 2));
            d.set_d_note_append_text(new_text.into());
            d.set_d_note_cursor(pos as i32);
        });
    }

    async fn append_note(self: Rc<Self>) {
        let Some(key) = self.st.borrow().detail.as_ref().map(|d| d.key.clone()) else { return };
        let Some(markdown) = self.with_data(|d| d.get_d_note_append_text().trim().to_owned()) else { return };
        if markdown.is_empty() {
            return;
        }
        let api = self.engine().api();
        let message: Rc<std::cell::RefCell<Option<String>>> = Rc::default();
        let (m, k) = (message.clone(), key.clone());
        let ok = inputs::run(
            |busy| {
                self.with_data(|d| d.set_d_note_append_busy(busy));
            },
            |msg| self.show_msg(msg),
            move || async move {
                *m.borrow_mut() = api.append_task_note(&k, &markdown).await.map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if !ok {
            return;
        }
        let text =
            message.borrow().clone().filter(|m| !m.is_empty()).unwrap_or_else(|| format!("已補在 {key} 的內容最後"));
        self.show_msg(Some(StatusMessage::ok(text)));
        self.with_data(|d| {
            d.set_d_note_append_text(Default::default());
            d.set_d_note_append_open(false);
        });
        self.refresh_detail_quietly(&key).await;
    }
}
