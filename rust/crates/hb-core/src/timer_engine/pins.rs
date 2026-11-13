//! 對應 C# `Core/TimerEngine.Pins.cs`：釘選托盤、切換計時、專案清單、待我審核、移動任務與工時。

use std::rc::Rc;

use chrono::TimeDelta;
use hb_api::{ChangeRequest, DateTimeOffset, KeyFinder, PinItem, ProjectDetail, ProjectInfo, TimeEntry};

use super::TimerEngine;
use super::extras::try_load;
use crate::ci_set::eq_ic;
use crate::error::CoreError;
use crate::time;

pub(super) const REVIEWS_INTERVAL: TimeDelta = TimeDelta::minutes(10);

impl TimerEngine {
    /// 托盤最多釘幾個（Hayaku 的上限）。
    pub const MAX_PINS: usize = 20;

    /// 看得到的專案（含封存的、圖示與我的角色），跟我的任務一起每 30 分鐘更新。
    pub fn projects(&self) -> Rc<Vec<ProjectInfo>> {
        self.s().projects.clone()
    }

    /// 釘在托盤上的任務（照順序）；打開面板、工作台時更新。
    pub fn pins(&self) -> Rc<Vec<PinItem>> {
        self.s().pins.clone()
    }

    /// 等我審核的變更申請（不含自己送出的）；只有在某個專案是管理員或編輯者時才會去抓。
    pub fn pending_reviews(&self) -> Rc<Vec<ChangeRequest>> {
        self.s().pending_reviews.clone()
    }

    /// 某個專案是管理員或編輯者（才有東西要審核）。
    pub fn can_review(&self) -> bool {
        self.s().projects.iter().any(|p| p.can_review() && !p.is_archived)
    }

    pub fn is_pinned(&self, key: &str) -> bool {
        self.s().pins.iter().any(|p| eq_ic(&p.key, key))
    }

    /// 專案名稱或代碼 → 專案（圖示用）。
    pub fn project_of(&self, name_or_key: &str) -> Option<ProjectInfo> {
        let s = self.s();
        s.projects
            .iter()
            .find(|p| p.name == name_or_key)
            .or_else(|| s.projects.iter().find(|p| eq_ic(&p.key, name_or_key)))
            .cloned()
    }

    pub(super) async fn refresh_reviews_core(&self) -> Result<(), CoreError> {
        self.s_mut().reviews_fetched = time::now();
        let me = self.s().user.as_ref().map(|u| u.username.clone());
        let pending =
            try_load(self.0.api.list_change_requests("PENDING"), self.pending_reviews(), "待審核的申請").await?;
        let list: Vec<ChangeRequest> =
            pending.iter().filter(|c| c.requester.as_ref().map(|r| &r.username) != me.as_ref()).cloned().collect();
        self.s_mut().pending_reviews = Rc::new(list);
        Ok(())
    }

    /// 審核頁核准、拒絕後呼叫：重抓待審核筆數。
    pub async fn refresh_reviews(&self) {
        self.guarded(self.refresh_reviews_core(), true).await;
    }

    /// 已經讀過的專案狀態清單（不會打 Hayaku）。
    pub fn try_get_cached_project(&self, project: &str) -> Option<Rc<ProjectDetail>> {
        self.s().project_details.get(project).cloned()
    }

    /// 狀態在專案設定裡的順序（專案還沒讀過就是 0，讀過後重排）。
    pub fn status_rank(&self, task_key: &str, status: &str) -> i32 {
        self.s()
            .project_details
            .get(KeyFinder::project_of(task_key))
            .and_then(|p| p.statuses.iter().position(|s| s.name == status))
            .map_or(0, |i| i as i32)
    }

    /// 先讀這些任務所屬專案的狀態清單（排序用）；有讀到新的回傳 true。
    pub async fn load_projects_of<'a>(&self, task_keys: impl IntoIterator<Item = &'a str>) -> bool {
        let mut loaded = false;
        let projects: crate::CiSet = task_keys.into_iter().map(KeyFinder::project_of).map(str::to_owned).collect();
        for project in projects.to_vec() {
            {
                let mut s = self.s_mut();
                if s.project_details.contains_key(&project) || !s.project_load_failed.insert(project.as_str()) {
                    continue; // 讀過、或讀失敗過（沒權限）的不再重讀
                }
            }
            match self.get_project(&project).await {
                Ok(_) => {
                    self.s_mut().project_load_failed.remove(&project);
                    loaded = true;
                }
                Err(err) => log::error!("讀取 {project} 的狀態清單失敗: {err}"),
            }
        }
        loaded
    }

    // ───────────── 釘選、切換 ─────────────

    pub async fn pin_task(&self, key: &str) {
        self.guarded(
            async {
                if self.s().pins.len() >= Self::MAX_PINS && !self.is_pinned(key) {
                    return Err(CoreError::mcp(format!("托盤最多釘 {} 個任務，先取消一個再釘", Self::MAX_PINS)));
                }
                let pins = self.0.api.pin_task(&key.to_uppercase()).await?;
                self.s_mut().pins = Rc::new(pins);
                self.notify(format!("已釘選 {key}"));
                Ok(())
            },
            true,
        )
        .await;
    }

    pub async fn unpin_task(&self, key: &str) {
        self.guarded(
            async {
                let pins = self.0.api.unpin_task(key).await?;
                self.s_mut().pins = Rc::new(pins);
                Ok(())
            },
            true,
        )
        .await;
    }

    /// 切換到這個任務（同網頁托盤的 ▶）：暫停其他所有計時、開始這個；keep_others 是同時計時。
    /// 被暫停的 Claude 計時跟手動暫停一樣，Claude 還在動也先不重開。
    pub async fn switch_task(&self, key: &str, keep_others: bool) {
        self.guarded(
            async {
                let key = key.trim().to_uppercase();
                let before = self.running_keys();
                self.0.api.switch_timer(&key, keep_others).await?;
                if !keep_others {
                    for other in before.iter().filter(|k| !eq_ic(k, &key)) {
                        {
                            let mut s = self.s_mut();
                            s.self_paused.insert(other);
                            if s.claude_desired.contains(other) {
                                s.suppressed.insert(other);
                            }
                        }
                        self.remove_claude_owned(other);
                    }
                }
                self.s_mut().suppressed.remove(&key);
                self.remove_claude_owned(&key);
                self.add_known_project(&key);
                self.refresh_core(false).await?;
                let pins = try_load(self.0.api.list_pins(), self.pins(), "釘選托盤").await?;
                self.s_mut().pins = pins;
                Ok(())
            },
            true,
        )
        .await;
    }

    // ───────────── 移動任務、工時 ─────────────

    /// 換上層（None＝移到最上層），只能在同一個專案裡。
    pub async fn move_task(&self, key: &str, parent: Option<&str>) {
        self.guarded(
            async {
                self.0.api.move_task(key, parent).await?;
                self.notify(match parent {
                    None => format!("{key} 已移到最上層"),
                    Some(p) => format!("{key} 已移到 {p} 底下"),
                });
                self.invalidate_extras();
                self.s_mut().tasks_fetched = time::MIN;
                self.refresh_core(false).await
            },
            true,
        )
        .await;
    }

    /// 把一段已結束的工時移到別的任務：在新任務補登同一段（起訖、備註不變），成功後才刪掉原本那段。
    /// 回傳新那段的 id（找不到就是 None），復原時用。MCP 沒有直接改計時段任務的工具，所以這樣做。
    pub async fn move_time_entry(&self, entry: &TimeEntry, to_task: &str) -> Result<Option<String>, CoreError> {
        let (Some(start), Some(end)) = (entry.started_at, entry.ended_at) else {
            return Err(CoreError::mcp("正在計時的那段不能移，先暫停再移"));
        };
        if entry.running {
            return Err(CoreError::mcp("正在計時的那段不能移，先暫停再移"));
        }
        let to_task = to_task.trim().to_uppercase();
        if eq_ic(&entry.task, &to_task) {
            return Err(CoreError::mcp(format!("這段已經是 {to_task} 的工時")));
        }
        let mut new_id: Option<String> = None;
        let mut error: Option<CoreError> = None;
        self.guarded(
            async {
                let moved: Result<(), CoreError> = async {
                    new_id = self.0.api.add_time_entry_with_id(&to_task, start, end, entry.note.as_deref()).await?;
                    if new_id.is_none() {
                        new_id = self.find_entry_id(&to_task, start, end).await?;
                    }
                    self.0.api.delete_time_entry(&entry.id).await?;
                    let fmt = |t: DateTimeOffset| time::local(&t).format("%H:%M").to_string();
                    log::info!("工時 {} {}–{} 移到 {to_task}", entry.task, fmt(start), fmt(end));
                    Ok(())
                }
                .await;
                if let Err(err) = moved {
                    error = Some(err);
                    // 新的補上了、舊的刪不掉：把新的刪掉，不要變成兩段
                    if let Some(id) = &new_id
                        && let Err(undo) = self.0.api.delete_time_entry(id).await
                    {
                        log::error!("移動工時失敗後刪除新補登的那段也失敗: {undo}");
                    }
                }
                self.refresh_core(false).await
            },
            true,
        )
        .await;
        match error {
            Some(err) => Err(err),
            None => Ok(new_id),
        }
    }

    /// 補登後回應裡沒有 id：從那天的計時段找同一個任務、同樣起訖的。
    async fn find_entry_id(
        &self,
        task: &str,
        start: DateTimeOffset,
        end: DateTimeOffset,
    ) -> Result<Option<String>, CoreError> {
        let day = time::local(&start).date();
        let entries = self.0.api.list_time_entries(day, day).await?;
        let close = |a: DateTimeOffset, b: DateTimeOffset| (a - b).num_milliseconds().abs() < 60_000;
        Ok(entries
            .iter()
            .find(|e| {
                eq_ic(&e.task, task)
                    && e.started_at.is_some_and(|s| close(s, start))
                    && e.ended_at.is_some_and(|en| close(en, end))
            })
            .map(|e| e.id.clone()))
    }
}
