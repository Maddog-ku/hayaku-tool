//! 對應 C# `Core/TimerEngine.NewWork.cs`：自動建立任務。偵測到沒有任務的新工作時跳提示，建好後指定給那個 session。

use std::time::Duration;

use chrono::{NaiveDateTime, TimeDelta};
use futures_util::FutureExt;
use futures_util::future::{LocalBoxFuture, Shared};
use hb_api::{KeyFinder, McpError, ProjectInfo, ProjectTask};
use hb_backend::config::FolderHint;

use super::{EngineEvents, Event, TimerEngine, ToastAction, ToastRequest};
use crate::error::CoreError;
use crate::project_guess::ProjectGuess;
use crate::time;

/// Claude 在某個資料夾改程式、但對應不到任務：建議建立任務。
/// title 是 session 第一句指令的摘要；project、parent 是這個資料夾上次用的專案與上層（沒有就 None）；
/// session_file 是 session 紀錄檔，請 Claude 估預計工時時讀開頭幾句指令的原文。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewWorkSuggestion {
    pub unit_path: String,
    pub unit_name: String,
    pub session_id: String,
    pub title: Option<String>,
    pub project: Option<String>,
    pub parent: Option<String>,
    pub session_file: Option<String>,
}

/// 專案樹的讀取（C# 的 `Task<List<ProjectTask>>`）：可以同時有多個地方等同一次讀取，讀完的結果留著。
pub type TreeLoad = Shared<LocalBoxFuture<'static, Result<Vec<ProjectTask>, McpError>>>;

/// 多久以內的改檔才算「正在改」；剛開工具時從 session 尾端讀到的舊編輯不算。
const EDIT_FRESHNESS: TimeDelta = TimeDelta::minutes(30);

/// 專案樹多久內可以直接拿來開視窗；超過 TREE_FRESH_TIME 的先顯示、再在背景重讀。
const TREE_CACHE_TIME: TimeDelta = TimeDelta::minutes(10);
const TREE_FRESH_TIME: TimeDelta = TimeDelta::seconds(30);

/// 提示上的按鈕要等提示收起來（淡出約 160ms）再開視窗。
const TOAST_FADE: Duration = Duration::from_millis(180);

impl TimerEngine {
    /// 把任務指定給某個 Claude session（之後這個 session 有動作就為它計時）。
    /// start_now：馬上開始計時（標成 Claude 的），不用等下一次偵測到 Claude 在動。
    pub async fn assign_session_task(&self, session_id: &str, key: &str, start_now: bool) {
        let key = key.trim().to_uppercase();
        {
            let mut state = self.0.state.borrow_mut();
            state.session_tasks.insert(session_id.to_owned(), key.clone());
            state.save();
        }
        self.s_mut().suppressed.remove(&key);
        self.add_known_project(&key);
        log::info!("Claude session {session_id} 指定為 {key}{}", if start_now { "，馬上開始計時" } else { "" });
        if start_now {
            self.guarded(
                async {
                    if self.running_keys().contains(&key) {
                        return Ok(());
                    }
                    self.0.api.start_timer(&key).await?;
                    if !self.is_claude_owned(&key) {
                        self.0.state.borrow_mut().claude_owned.push(key.clone());
                    }
                    self.0.state.borrow().save();
                    self.refresh_core(false).await
                },
                true,
            )
            .await;
        }
        self.claude_tick(true).await;
    }

    /// 記下這個資料夾用的專案與新任務的上層，下次建立任務時直接帶入。
    pub fn remember_folder(&self, unit_path: &str, project: &str, parent: Option<&str>) {
        let mut state = self.0.state.borrow_mut();
        state.folder_hints.insert(
            unit_path.to_owned(),
            FolderHint { project: project.to_owned(), parent: parent.map(str::to_owned) },
        );
        state.save();
    }

    /// 資料夾裡的 session 對應到任務時，順便記下這個資料夾屬於哪個專案（換了專案，上次的上層就不適用）。
    pub(super) fn learn_folder_projects(&self) {
        let claude = self.0.claude.borrow();
        let mut state = self.0.state.borrow_mut();
        let mut changed = false;
        for repo in claude.repos() {
            let Some(task) = &repo.resolved_task else { continue };
            let project = KeyFinder::project_of(task);
            if state.folder_hints.get(&repo.path).is_some_and(|h| crate::ci_set::eq_ic(&h.project, project)) {
                continue;
            }
            state.folder_hints.insert(repo.path.clone(), FolderHint { project: project.to_owned(), parent: None });
            changed = true;
        }
        if changed {
            state.save();
        }
    }

    /// 可以建立任務的專案（沒封存的）；同步時已經讀過就直接用。
    pub async fn creatable_projects(&self) -> Result<Vec<ProjectInfo>, CoreError> {
        let projects = self.projects();
        if !projects.is_empty() {
            return Ok(projects.iter().filter(|p| !p.is_archived).cloned().collect());
        }
        Ok(self.0.api.list_projects(false).await?)
    }

    /// 新工作預設建在哪個專案：這個資料夾上次用的 → 名稱像資料夾的 → 正在計時的；都沒有就第一個。
    pub fn guess_project(&self, projects: &[ProjectInfo], suggestion: &NewWorkSuggestion) -> Option<ProjectInfo> {
        let pairs: Vec<(String, String)> = projects.iter().map(|p| (p.key.clone(), p.name.clone())).collect();
        let running: Vec<String> = self.running_keys().iter().map(|k| KeyFinder::project_of(k).to_owned()).collect();
        let key = ProjectGuess::guess(&pairs, suggestion.project.as_deref(), &suggestion.unit_name, running);
        projects.iter().find(|p| Some(&p.key) == key.as_ref()).or_else(|| projects.first()).cloned()
    }

    /// 專案的全部任務（快取 10 分鐘）。網路在背景執行環境跑，不卡畫面；fresh：超過 30 秒的就重讀。
    pub fn project_tree(&self, project: &str, fresh: bool) -> TreeLoad {
        let now = time::now();
        if let Some((at, load)) = self.0.tree_cache.borrow().get(project) {
            let faulted = matches!(load.peek(), Some(Err(_)));
            if !faulted && now - *at < if fresh { TREE_FRESH_TIME } else { TREE_CACHE_TIME } {
                return load.clone();
            }
        }
        let api = self.0.api.clone();
        let key = project.to_owned();
        let load: TreeLoad = async move { api.list_project_tasks(&key, None).await }.boxed_local().shared();
        // C# 的 Task.Run 會馬上開始：這裡也先開始跑，不用等有人 await
        let driver = load.clone();
        self.spawn(async move {
            let _ = driver.await;
        });
        self.0.tree_cache.borrow_mut().insert(project.to_owned(), (now, load.clone()));
        load
    }

    /// 已經讀好的專案樹（還在讀或沒讀過是 None）；第二個值（Stale）：超過 30 秒，顯示後要在背景重讀。
    pub fn cached_project_tree(&self, project: &str) -> Option<(Vec<ProjectTask>, bool)> {
        let cache = self.0.tree_cache.borrow();
        let (at, load) = cache.get(project)?;
        let age = time::now() - *at;
        match load.peek() {
            Some(Ok(tasks)) if age < TREE_CACHE_TIME => Some((tasks.clone(), age >= TREE_FRESH_TIME)),
            _ => None,
        }
    }

    /// 專案樹有變（例如剛建了任務）：下次重讀。
    pub fn forget_project_tree(&self, project: &str) {
        self.0.tree_cache.borrow_mut().remove(project);
    }

    /// 提示跳出時先讀好預設專案的任務樹與狀態清單，使用者按「建立任務」時視窗一開就有內容。
    pub async fn prefetch_new_work(&self, suggestion: &NewWorkSuggestion) {
        let result: Result<(), CoreError> = async {
            let projects = self.creatable_projects().await?;
            if let Some(project) = self.guess_project(&projects, suggestion) {
                let (tree, detail) =
                    futures_util::future::join(self.project_tree(&project.key, false), self.get_project(&project.key))
                        .await;
                tree?;
                detail?;
            }
            self.0.events.new_work_ready.invoke(suggestion);
            Ok(())
        }
        .await;
        if let Err(err) = result {
            log::error!("預先讀取建立任務用的專案樹失敗: {err}");
        }
    }

    /// 有開「自動建立任務」的資料夾：Claude 最近在裡面改了檔、這個 session 又對應不到任務時，跳提示問一次。
    /// 只是問問題、規劃（沒改檔）不會跳；同一個 session 只問一次。
    pub(super) fn check_new_work(&self, now: NaiveDateTime) {
        if !self.logged_in() || self.0.config.borrow().claude_only_during_work && !self.is_working(now) {
            return;
        }
        let mut suggestions: Vec<NewWorkSuggestion> = Vec::new();
        {
            let claude = self.0.claude.borrow();
            let mut state = self.0.state.borrow_mut();
            for repo in claude.repos() {
                if !repo.auto_create || !repo.active || repo.resolved_task.is_some() {
                    continue;
                }
                let (Some(sid), Some(edited)) = (&repo.session_id, repo.edited_at) else { continue };
                if now - edited > EDIT_FRESHNESS || state.asked_sessions.contains(sid) {
                    continue;
                }
                state.asked_sessions.push(sid.clone());
                state.save();
                let hint = state.folder_hints.get(&repo.path).cloned();
                suggestions.push(NewWorkSuggestion {
                    unit_path: repo.path.clone(),
                    unit_name: repo.name().to_owned(),
                    session_id: sid.clone(),
                    title: repo.first_prompt.clone(),
                    project: hint.as_ref().map(|h| h.project.clone()),
                    parent: hint.and_then(|h| h.parent),
                    session_file: repo.session_file.clone(),
                });
                log::info!("{} 的 Claude session {sid} 在改程式但沒有任務，詢問要不要建立", repo.name());
            }
        }
        // 停留多久自動收起（設定 › 一般；0 是要按掉才收起）。收起後同一個 session 不會再問
        let stay_seconds = self.0.config.borrow().new_work_toast_seconds;
        for suggestion in suggestions {
            let (e1, e2) = (self.clone(), self.clone());
            let (s1, s2) = (suggestion.clone(), suggestion.clone());
            self.toast(
                ToastRequest::new(
                    format!("{} 有新工作，要建立任務嗎？", suggestion.unit_name),
                    suggestion.title.clone().unwrap_or_else(|| "Claude 正在改程式，但對應不到任務".to_owned()),
                    vec![
                        ToastAction::new("建立任務", move || {
                            e1.raise_later(|ev| &ev.create_task_requested, s1.clone());
                            async {}
                        })
                        .primary(),
                        ToastAction::new("選既有任務", move || {
                            e2.raise_later(|ev| &ev.pick_task_requested, s2.clone());
                            async {}
                        }),
                        ToastAction::new("不用", || async {}),
                    ],
                )
                .sticky(stay_seconds <= 0)
                .persistent(stay_seconds <= 0)
                .duration((stay_seconds > 0).then(|| Duration::from_secs(stay_seconds as u64))),
            );
            let e = self.clone();
            self.spawn(async move { e.prefetch_new_work(&suggestion).await });
        }
    }

    /// 提示上的按鈕：等提示收起來（淡出約 160ms）再開視窗。
    /// 開視窗要建整個畫面，跟淡出擠在同一個時間會讓動畫頓一下。
    pub(super) fn raise_later<A: 'static>(&self, pick: fn(&EngineEvents) -> &Event<A>, arg: A) {
        if !pick(&self.0.events).has_handlers() {
            return;
        }
        let weak = self.weak();
        slint::Timer::single_shot(TOAST_FADE, move || {
            if let Some(e) = Self::from_weak(&weak) {
                pick(&e.0.events).invoke(&arg);
            }
        });
    }
}
