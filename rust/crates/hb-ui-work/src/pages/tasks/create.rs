//! 對應 C# `UI/Work/TasksPage.xaml.cs` 的「新增任務」：左邊專案樹（點 ＋ 選上層），右邊填資料。
//! 建立後會留在彈窗裡，可以接著新增下一筆。

use std::rc::Rc;

use hb_api::KeyFinder;
use hb_core::{CiSet, TaskTree};
use hb_ui_common::date_range::from_int;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use serde_json::{Map, Value, json};
use slint::Model;

use super::Page;
use super::vm;
use crate::shared::{self, icon_item, string_model, vec_model};
use crate::{TasksPageData, TkPickRow};

/// 專案的任務不多（150 個以內）時樹全部展開。
const EXPAND_ALL_LIMIT: usize = 150;
const NEW_PRIORITIES: [&str; 5] = ["無", "緊急", "高", "中", "低"];

impl Page {
    pub(super) fn wire_create(self: &Rc<Self>, d: &TasksPageData) {
        use super::on;
        on!(d, self, on_c_project_selected, |p, i| p.project_selected(i as usize));
        on!(d, self, on_c_filter_edited, |p, _t| p.render_pick_tree(false));
        on!(d, self, on_c_hide_done_toggled, |p, v| {
            p.save_config(|c| c.tree_hide_done = v);
            p.render_pick_tree(false);
        });
        on!(d, self, on_c_pick, |p, key| p.pick_parent(if key.is_empty() { None } else { Some(key.to_string()) }));
        on!(d, self, on_c_toggle, |p, i| p.toggle_pick(i as usize));
        on!(d, self, on_c_create, |p| shared::spawn("建立任務", async move { p.create().await }));
        on!(d, self, on_c_cancel, |p| {
            // 取消：從任務的「＋ 子任務」進來的回到那個任務，否則關掉彈窗
            let parent = p.st.borrow().creating_child_of.clone();
            match parent {
                Some(parent) => p.open_key(&parent),
                None => p.close_dialog(),
            }
        });
        on!(d, self, on_c_open_created_clicked, |p| {
            let key = p.st.borrow().last_created.clone();
            if let Some(key) = key {
                p.open_key(&key);
            }
        });
    }

    /// 開新增任務的彈窗；parent 有值時（任務的「＋ 子任務」）先選好那個專案與上層。
    pub(super) fn show_create(self: Rc<Self>, parent: Option<String>) {
        shared::spawn("新增任務", async move { self.show_create_async(parent).await });
    }

    async fn show_create_async(self: Rc<Self>, parent: Option<String>) {
        {
            let mut st = self.st.borrow_mut();
            st.detail_version += 1;
            st.creating_child_of = parent.clone();
            st.picked_parent = parent.clone();
        }
        self.with_data(|d| {
            d.set_dialog(2);
            d.set_c_title(match &parent {
                None => "新增任務".into(),
                Some(p) => format!("新增 {p} 的子任務").into(),
            });
            d.set_c_new_title(Default::default());
            d.set_c_estimate(Default::default());
            d.set_c_filter(Default::default());
            d.set_c_start(0);
            d.set_c_end(0);
            d.set_c_priority_index(0);
            d.set_c_msg(Default::default());
            d.set_c_open_created(Default::default());
            d.set_c_focus_serial(d.get_c_focus_serial() + 1);
        });
        if self.st.borrow().create_projects.is_none() {
            match self.engine().api().list_projects(false).await {
                Ok(projects) => {
                    let labels: Vec<String> = projects.iter().map(|p| p.name.clone()).collect();
                    let icons: Vec<_> = projects.iter().map(|p| icon_item(Some(p), &p.name)).collect();
                    self.with_data(|d| {
                        d.set_c_project_labels(string_model(&labels));
                        d.set_c_project_icons(vec_model(icons));
                    });
                    self.st.borrow_mut().create_projects = Some(projects);
                }
                Err(e) => self.show_create_msg(Some(StatusMessage::error(format!("讀不到專案清單：{}", e.0)))),
            }
        }
        let project_key: Option<String> = match (&parent, self.st.borrow().detail.as_ref()) {
            (Some(p), _) => Some(KeyFinder::project_of(p).to_owned()),
            (None, Some(d)) => Some(KeyFinder::project_of(&d.key).to_owned()),
            (None, None) => self.engine().running_keys().iter().next().map(|k| KeyFinder::project_of(k).to_owned()),
        };
        let target = {
            let st = self.st.borrow();
            st.create_projects.as_ref().and_then(|projects| {
                projects
                    .iter()
                    .position(|p| project_key.as_deref().is_some_and(|k| p.key == k))
                    .or(if projects.is_empty() { None } else { Some(0) })
                    .map(|i| (i, projects[i].key.clone()))
            })
        };
        if let Some((index, key)) = target {
            let current = self.with_data(|d| d.get_c_project_index()).unwrap_or(-1);
            if current == index as i32 {
                self.load_tree(key).await; // 同一個專案不會觸發 SelectionChanged，自己重讀
            } else {
                self.st.borrow_mut().keep_parent_on_project_change = true;
                self.with_data(|d| d.set_c_project_index(index as i32));
                self.project_selected(index);
            }
        }
    }

    fn show_create_msg(&self, msg: Option<StatusMessage>) {
        let (text, tone) = shared::status_parts(msg.as_ref());
        self.with_data(|d| {
            d.set_c_msg(text);
            d.set_c_msg_tone(tone);
        });
    }

    fn project_selected(&self, index: usize) {
        let Some(key) = self.st.borrow().create_projects.as_ref().and_then(|p| p.get(index)).map(|p| p.key.clone())
        else {
            return;
        };
        {
            let mut st = self.st.borrow_mut();
            if !st.keep_parent_on_project_change {
                st.picked_parent = None; // 換專案時上層歸零
            }
            st.keep_parent_on_project_change = false;
        }
        if let Some(p) = self.rc() {
            shared::spawn("讀取專案的任務", async move { p.load_tree(key).await });
        }
    }

    /// 讀專案的全部任務與狀態清單，畫出專案樹。
    async fn load_tree(&self, project_key: String) {
        let version = {
            let mut st = self.st.borrow_mut();
            st.tree_version += 1;
            st.tree_version
        };
        self.with_data(|d| {
            d.set_c_tree_message("讀取中…".into());
            d.set_c_tree(vec_model(Vec::new()));
        });
        let engine = self.engine().clone();
        let result: Result<(Vec<hb_api::ProjectTask>, Rc<hb_api::ProjectDetail>), String> = async {
            let tasks = engine.api().list_project_tasks(&project_key, None).await.map_err(|e| e.0)?;
            let project = engine.get_project(&project_key).await.map_err(|e| e.message().to_owned())?;
            Ok((tasks, project))
        }
        .await;
        if version != self.st.borrow().tree_version {
            return;
        }
        match result {
            Ok((tasks, project)) => {
                let statuses: Vec<String> = project.statuses.iter().map(|s| s.name.clone()).collect();
                let default = project.statuses.iter().position(|s| s.is_default).or(if statuses.is_empty() {
                    None
                } else {
                    Some(0)
                });
                self.with_data(|d| {
                    d.set_c_status_items(string_model(&statuses));
                    d.set_c_status_index(default.map_or(-1, |i| i as i32));
                });
                {
                    let mut st = self.st.borrow_mut();
                    if let Some(p) = &st.picked_parent
                        && !tasks.iter().any(|t| t.key.eq_ignore_ascii_case(p))
                    {
                        st.picked_parent = None;
                    }
                    st.project_tasks = tasks;
                }
                self.render_pick_tree(true);
            }
            Err(message) => {
                log::error!("讀取 {project_key} 的任務樹失敗: {message}");
                self.st.borrow_mut().project_tasks.clear();
                self.with_data(|d| d.set_c_tree_message(format!("讀不到專案的任務：{message}").into()));
            }
        }
        self.update_parent_text();
    }

    /// 專案樹（C# RenderTree）：重畫時展開狀態回到預設（全部展開或只展開第一層與選中的上層）。
    fn render_pick_tree(&self, scroll_to_picked: bool) {
        let Some((filter, hide_done)) = self.with_data(|d| (d.get_c_filter().trim().to_owned(), d.get_c_hide_done()))
        else {
            return;
        };
        {
            let mut st = self.st.borrow_mut();
            st.pick_roots = TaskTree::build(
                st.project_tasks.clone(),
                !hide_done,
                if filter.is_empty() { None } else { Some(&filter) },
            );
            st.pick_toggled.clear();
        }
        let message = {
            let st = self.st.borrow();
            if st.project_tasks.is_empty() {
                "這個專案還沒有任務，新增的會在最上層"
            } else if st.pick_roots.is_empty() {
                "沒有符合的任務"
            } else {
                ""
            }
        };
        self.refresh_pick_rows();
        self.with_data(|d| d.set_c_tree_message(message.into()));
        if scroll_to_picked {
            let picked = self.st.borrow().picked_parent.clone();
            if let Some(p) = picked {
                self.scroll_pick_to(&p);
            }
        }
    }

    fn refresh_pick_rows(&self) {
        let Some(filter) = self.with_data(|d| d.get_c_filter().trim().to_owned()) else { return };
        let rows: Vec<TkPickRow> = {
            let mut st = self.st.borrow_mut();
            let expand_all = !filter.is_empty() || st.project_tasks.len() <= EXPAND_ALL_LIMIT;
            let picked = st.picked_parent.clone();
            let ancestors: CiSet = picked
                .as_deref()
                .map(|p| vm::ancestors_of(&st.project_tasks, p))
                .unwrap_or_default()
                .into_iter()
                .collect();
            st.pick_rows =
                vm::flatten_pick_tree(&st.pick_roots, picked.as_deref(), expand_all, &ancestors, &st.pick_toggled);
            st.pick_rows
                .iter()
                .map(|r| TkPickRow {
                    key: r.task.key.as_str().into(),
                    title: r.task.title.as_str().into(),
                    suffix: r.status_suffix().into(),
                    depth: r.depth,
                    expanded: r.expanded,
                    has_children: r.has_children,
                    picked: r.picked,
                    opacity: r.opacity(),
                    tip: r.tip().into(),
                })
                .collect()
        };
        let root_picked = self.st.borrow().picked_parent.is_none();
        self.with_data(|d| {
            let model = d.get_c_tree();
            // 列數一樣就逐列更新（捲動位置不會跳）
            if model.row_count() == rows.len() && !rows.is_empty() {
                for (i, r) in rows.into_iter().enumerate() {
                    if model.row_data(i).as_ref() != Some(&r) {
                        model.set_row_data(i, r);
                    }
                }
            } else {
                d.set_c_tree(vec_model(rows));
            }
            d.set_c_root_picked(root_picked);
        });
    }

    fn scroll_pick_to(&self, key: &str) {
        let index = self.st.borrow().pick_rows.iter().position(|r| r.task.key.eq_ignore_ascii_case(key));
        if let Some(i) = index {
            self.with_data(|d| {
                d.set_c_scroll_row(i as i32);
                d.set_c_scroll_serial(d.get_c_scroll_serial() + 1);
            });
        }
    }

    fn toggle_pick(&self, index: usize) {
        {
            let mut st = self.st.borrow_mut();
            let Some(row) = st.pick_rows.get(index).cloned() else { return };
            st.pick_toggled.insert(row.task.key.to_uppercase(), !row.expanded);
        }
        self.refresh_pick_rows();
    }

    fn update_parent_text(&self) {
        let text = {
            let st = self.st.borrow();
            vm::parent_text(&st.project_tasks, st.picked_parent.as_deref())
        };
        self.with_data(|d| d.set_c_parent_text(text.into()));
    }

    fn pick_parent(&self, key: Option<String>) {
        self.st.borrow_mut().picked_parent = key;
        self.refresh_pick_rows();
        self.update_parent_text();
    }

    /// 建立任務；成功後留在彈窗、重讀專案樹並標出新任務，可以接著新增下一筆。
    async fn create(self: Rc<Self>) {
        let Some(project_index) = self.with_data(|d| d.get_c_project_index()) else { return };
        let Some(project) = self
            .st
            .borrow()
            .create_projects
            .as_ref()
            .and_then(|p| p.get(project_index.max(0) as usize).cloned())
            .filter(|_| project_index >= 0)
        else {
            self.show_create_msg(Some(StatusMessage::error("請選專案")));
            return;
        };
        let Some((title, status, priority, start, end, estimate, assign_me, start_timer)) = self.with_data(|d| {
            let status = d
                .get_c_status_items()
                .row_data(d.get_c_status_index().max(0) as usize)
                .filter(|_| d.get_c_status_index() >= 0);
            (
                d.get_c_new_title().trim().to_owned(),
                status.map(|s| s.to_string()),
                NEW_PRIORITIES.get(d.get_c_priority_index().max(0) as usize).copied().unwrap_or("無"),
                from_int(d.get_c_start()),
                from_int(d.get_c_end()),
                d.get_c_estimate().trim().to_owned(),
                d.get_c_assign_me(),
                d.get_c_start_timer(),
            )
        }) else {
            return;
        };
        if title.is_empty() {
            self.show_create_msg(Some(StatusMessage::error("請填名稱")));
            self.with_data(|d| d.set_c_focus_serial(d.get_c_focus_serial() + 1));
            return;
        }
        let mut fields = Map::new();
        fields.insert("project".into(), json!(project.key));
        fields.insert("title".into(), json!(title));
        if let Some(parent) = self.st.borrow().picked_parent.clone() {
            fields.insert("parent".into(), json!(parent));
        }
        if let Some(s) = status {
            fields.insert("status".into(), json!(s));
        }
        if priority != "無" {
            fields.insert("priority".into(), json!(priority));
        }
        if let Some(s) = start {
            fields.insert("plannedStart".into(), json!(inputs::date_text(Some(s))));
        }
        if let Some(e) = end {
            fields.insert("plannedEnd".into(), json!(inputs::date_text(Some(e))));
        }
        if !estimate.is_empty() {
            match estimate.parse::<f64>() {
                Ok(h) if h.is_finite() && h >= 0.0 => {
                    fields.insert("estimatedHours".into(), json!(h));
                }
                _ => {
                    self.show_create_msg(Some(StatusMessage::error("預估工時請填小時數")));
                    return;
                }
            }
        }
        if assign_me {
            fields.insert("assignees".into(), Value::Array(vec![json!("me")]));
        }

        self.with_data(|d| d.set_c_open_created(Default::default()));
        let api = self.engine().api();
        let created: Rc<std::cell::RefCell<String>> = Rc::default();
        let c = created.clone();
        let ok = inputs::run(
            |busy| {
                self.with_data(|d| d.set_c_busy(busy));
            },
            |msg| self.show_create_msg(msg),
            move || async move {
                *c.borrow_mut() = api.create_task(fields).await.map_err(ActionError::from)?;
                Ok(())
            },
            None,
        )
        .await;
        if !ok {
            return;
        }
        let key = created.borrow().clone();
        if start_timer && !key.is_empty() {
            self.engine().start_task(&key).await;
        }
        let last = (!key.is_empty()).then(|| key.clone());
        self.st.borrow_mut().last_created = last.clone();
        self.show_create_msg(Some(StatusMessage::ok(if key.is_empty() {
            "已建立".to_owned()
        } else {
            format!("已建立 {key}　{title}")
        })));
        self.with_data(|d| {
            if let Some(k) = &last {
                d.set_c_open_created(format!("開啟 {k} ›").into());
            }
            d.set_c_new_title(Default::default());
            d.set_c_focus_serial(d.get_c_focus_serial() + 1);
        });
        let engine = self.engine().clone();
        engine.clone().spawn(async move { engine.after_external_change().await });
        self.load_tree(project.key.clone()).await;
        if let Some(k) = last {
            self.scroll_pick_to(&k);
        }
    }
}
