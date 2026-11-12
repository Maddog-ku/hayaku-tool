//! 對應 C# `UI/Work/TasksPage.Columns.cs`：我的任務清單的欄位：顯示哪些、順序、寬度（設定存在 config）；
//! 標題列拖右邊的線調整寬度。「欄位」對話框勾要顯示的欄、上下調整順序、填寬度，專案欄可以只顯示代碼。

use std::rc::Rc;

use hb_backend::config::TaskColumnConfig;
use hb_core::TaskColumns;

use super::Page;
use crate::shared::vec_model;
use crate::{TasksPageData, TkColDraft, TkColumn};

/// 寬度文字（C# `Width.ToString("0")`）。
pub(super) fn width_text(width: f64) -> String {
    format!("{}", width.round() as i64)
}

/// 寬度欄填的字 → 新的寬度（限制在 40～480）；不是數字就維持原本的。
pub(super) fn parse_width(text: &str, current: f64) -> f64 {
    match text.trim().parse::<f64>() {
        Ok(w) if w.is_finite() => w.clamp(TaskColumns::MIN_WIDTH, TaskColumns::MAX_WIDTH),
        _ => current,
    }
}

/// 顯示中的欄位 → 清單的標題與列用的欄。
pub(super) fn visible_columns(columns: &[TaskColumnConfig]) -> Vec<TkColumn> {
    columns
        .iter()
        .filter(|c| c.visible)
        .map(|c| TkColumn {
            key: c.key.as_str().into(),
            label: TaskColumns::label_of(&c.key).into(),
            width: if c.width <= 0.0 { 0.0 } else { c.width as f32 },
            resizable: c.width > 0.0,
        })
        .collect()
}

impl Page {
    pub(super) fn wire_columns(self: &Rc<Self>, d: &TasksPageData) {
        super::on!(d, self, on_column_resize, |p, i, w| p.column_resize(i as usize, w));
        super::on!(d, self, on_column_resize_done, |p| p.save_columns());
        super::on!(d, self, on_columns_open, |p| p.open_columns());
        super::on!(d, self, on_col_toggle, |p, i, v| {
            if let Some(c) = p.st.borrow_mut().col_draft.get_mut(i as usize).filter(|c| c.key != "title") {
                c.visible = v;
            }
            p.render_draft();
        });
        super::on!(d, self, on_col_move, |p, i, delta| {
            {
                let mut st = p.st.borrow_mut();
                if let Some(key) = st.col_draft.get(i as usize).map(|c| c.key.clone()) {
                    TaskColumns::move_column(&mut st.col_draft, &key, delta);
                }
            }
            p.render_draft();
        });
        super::on!(d, self, on_col_width, |p, i, text| {
            if let Some(c) = p.st.borrow_mut().col_draft.get_mut(i as usize).filter(|c| c.key != "title") {
                c.width = parse_width(&text, c.width);
            }
            p.render_draft();
        });
        super::on!(d, self, on_col_reset, |p| {
            p.st.borrow_mut().col_draft = TaskColumns::normalize([].iter());
            p.with_data(|d| d.set_col_code_only(false));
            p.render_draft();
        });
        super::on!(d, self, on_col_apply, |p| p.apply_draft());
        super::on!(d, self, on_col_cancel, |p| {
            p.with_data(|d| d.set_col_dialog(false));
        });
    }

    /// 依目前的欄位設定重建標題列與列。
    pub(super) fn apply_columns(&self) {
        let (cols, key_column) = {
            let st = self.st.borrow();
            (visible_columns(&st.columns), Self::column_visible(&st, "key"))
        };
        self.with_data(|d| {
            d.set_columns(vec_model(cols));
            d.set_key_column(key_column);
        });
    }

    fn save_columns(&self) {
        let columns: Vec<TaskColumnConfig> = self
            .st
            .borrow()
            .columns
            .iter()
            .map(|c| TaskColumnConfig { key: c.key.clone(), visible: c.visible, width: c.width })
            .collect();
        self.save_config(|c| c.task_columns = columns);
    }

    /// 標題列拖曳：第 index 個顯示中的欄改寬度（放開才存檔）。
    fn column_resize(&self, index: usize, width: f32) {
        {
            let mut st = self.st.borrow_mut();
            let Some(c) = st.columns.iter_mut().filter(|c| c.visible).nth(index) else { return };
            if c.width <= 0.0 {
                return;
            }
            c.width = (width as f64).clamp(TaskColumns::MIN_WIDTH, TaskColumns::MAX_WIDTH);
        }
        self.apply_columns();
    }

    fn open_columns(&self) {
        let code_only = self.engine().config().task_project_code_only;
        {
            let mut st = self.st.borrow_mut();
            st.col_draft = st.columns.clone();
        }
        self.with_data(|d| {
            d.set_col_code_only(code_only);
            d.set_col_dialog(true);
        });
        self.render_draft();
    }

    fn render_draft(&self) {
        let rows: Vec<TkColDraft> = {
            let st = self.st.borrow();
            let n = st.col_draft.len();
            st.col_draft
                .iter()
                .enumerate()
                .map(|(i, c)| TkColDraft {
                    key: c.key.as_str().into(),
                    label: TaskColumns::label_of(&c.key).into(),
                    visible: c.visible,
                    width_text: width_text(c.width).into(),
                    is_title: c.key == "title",
                    can_up: i > 0,
                    can_down: i + 1 < n,
                })
                .collect()
        };
        // 每次給新的模型：輸入框改過的字（例如打錯）會換回整理過的寬度
        self.with_data(|d| d.set_col_draft(vec_model(rows)));
    }

    fn apply_draft(&self) {
        let code_only = self.with_data(|d| d.get_col_code_only()).unwrap_or(false);
        {
            let mut st = self.st.borrow_mut();
            let draft = std::mem::take(&mut st.col_draft);
            st.columns = TaskColumns::normalize(draft.iter());
        }
        self.save_config(|c| c.task_project_code_only = code_only);
        self.save_columns();
        self.apply_columns();
        self.with_data(|d| d.set_col_dialog(false));
        self.render_list(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 寬度輸入限制範圍() {
        assert_eq!(120.0, parse_width("120", 80.0));
        assert_eq!(TaskColumns::MIN_WIDTH, parse_width("3", 80.0));
        assert_eq!(TaskColumns::MAX_WIDTH, parse_width("9999", 80.0));
        assert_eq!(80.0, parse_width("abc", 80.0));
        assert_eq!("97", width_text(96.6));
    }

    #[test]
    fn 顯示中的欄位_名稱填滿() {
        let cols = TaskColumns::normalize([].iter());
        let visible = visible_columns(&cols);
        assert_eq!(
            vec!["key", "title", "project", "status", "end"],
            visible.iter().map(|c| c.key.as_str()).collect::<Vec<_>>()
        );
        let title = visible.iter().find(|c| c.key == "title").unwrap();
        assert_eq!(0.0, title.width);
        assert!(!title.resizable);
        assert!(visible[0].resizable);
        assert_eq!("編號", visible[0].label.as_str());
    }
}
