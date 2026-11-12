//! 對應 C# `Core/TimerEngine.Eip.cs`：EIP 請假同步。
//!
//! 每天上班前、下班時各讀一次 EIP 的假單（待審＋核准），把跟「今天到下一個上班日」重疊的請假整張登記到 Hayaku
//! （上次之後漏掉的日子也補登，最多 7 天）；由同步建立的請假，EIP 那邊駁回、抽單、改時間時取消（手動登記的不動）。

use std::rc::Rc;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};
use hb_api::{DaySchedule, UserInfo};
use hb_eip::{EipClient, EipCredentialStore, EipCredentials, EipLeave, LeavePiece, LeavePlanner, SyncedLeave};

use super::{TimerEngine, ToastAction, ToastRequest};
use crate::error::{CoreError, is_mcp};
use crate::off_day_cache::OffDayCache;
use crate::time::{self, MIN};

/// EIP 假單加上查詢時的狀態（待審、核准、駁回）。
#[derive(Debug, Clone, PartialEq)]
pub struct EipLeaveRecord {
    pub leave: EipLeave,
    pub state: String,
}

/// 自動同步失敗（連不上 EIP、Hayaku）後多久再試。
const EIP_RETRY_DELAY: TimeDelta = TimeDelta::minutes(30);

/// 找下一個上班日、算整天與否時要看幾天的班表（get_my_schedule 一次最多 31 天）。
const EIP_SCHEDULE_DAYS: i64 = 30;

fn add_days(d: NaiveDate, days: i64) -> NaiveDate {
    d + TimeDelta::days(days)
}

type SyncLists = (Vec<String>, Vec<String>, Vec<String>, Vec<String>);

impl TimerEngine {
    /// EIP 請假同步的狀態（設定頁顯示上次同步的結果）。
    pub fn eip_sync(&self) -> std::cell::Ref<'_, hb_eip::EipSyncState> {
        self.0.eip_state.borrow()
    }

    pub fn eip_syncing(&self) -> bool {
        self.s().eip_syncing
    }

    /// Claude 計時器每次 tick 都會問：有上班的日子下班了、今天還沒做過，就做一次下班後的工作——
    /// 同步 EIP 請假（有設定時），並記下接下來不用上班的日子。有開 EIP 同步時，上班前也同步一次。
    /// 不用上班的日子什麼都不做。
    pub(super) async fn after_work_tick(&self) {
        let now = time::now();
        let schedule = {
            let s = self.s();
            if !s.logged_in || s.eip_syncing || now < s.eip_retry_at {
                return;
            }
            match &s.schedule {
                Some(sc) if sc.date == now.date() => sc.clone(),
                _ => return,
            }
        };
        let clocked_out = self.0.state.borrow().clocked_out;
        if self.eip_sync_enabled() {
            let (morning_day, last_sync_day) = {
                let st = self.0.eip_state.borrow();
                (st.morning_day.clone(), st.last_sync_day.clone())
            };
            if LeavePlanner::is_morning_due(&schedule, clocked_out, now, &morning_day) {
                self.sync_eip_leaves(false, true).await; // 開機補做下班那次的也一起算
            } else if LeavePlanner::is_due(&schedule, clocked_out, now, &last_sync_day) {
                self.sync_eip_leaves(false, false).await; // 會順便更新不用上班的日子
            }
            return;
        }
        let updated = self.0.state.borrow().off_days_updated.clone();
        if LeavePlanner::is_due(&schedule, clocked_out, now, &updated) {
            self.refresh_off_days().await;
        }
    }

    fn eip_sync_enabled(&self) -> bool {
        self.0.config.borrow().eip_leave_sync && hb_backend::infra::AppPaths::eip_auth().exists()
    }

    /// 今天上班前那次 EIP 同步還沒做完（也還沒失敗）：先不自動上班，
    /// 免得早上才在 EIP 請了整天假，還先被自動開始計時。同步失敗就照常上班，30 分鐘後再試。
    pub(super) fn morning_eip_pending(&self, now: NaiveDateTime) -> bool {
        if !self.eip_sync_enabled() {
            return false;
        }
        let s = self.s();
        let Some(schedule) = s.schedule.as_ref().filter(|sc| sc.date == now.date()) else {
            return false;
        };
        s.eip_syncing
            || (now >= s.eip_retry_at
                && LeavePlanner::is_morning_due(
                    schedule,
                    self.0.state.borrow().clocked_out,
                    now,
                    &self.0.eip_state.borrow().morning_day,
                ))
    }

    /// 沒開 EIP 同步時，下班時只讀接下來的班表，記下不用上班的日子。
    async fn refresh_off_days(&self) {
        let today = time::today();
        self.s_mut().eip_syncing = true;
        match self.0.api.get_schedule_range(today, add_days(today, EIP_SCHEDULE_DAYS)).await {
            Ok(days) => self.update_off_days(&days, today),
            Err(err) => {
                log::error!("讀取接下來的班表失敗: {err}");
                self.s_mut().eip_retry_at = time::now() + EIP_RETRY_DELAY;
            }
        }
        self.s_mut().eip_syncing = false;
    }

    fn update_off_days(&self, days: &[DaySchedule], today: NaiveDate) {
        let covered = self.covered_day(time::now());
        let mut state = self.0.state.borrow_mut();
        state.off_days = OffDayCache::from(days, today);
        state.off_days_updated = covered;
        state.save();
        let list = if state.off_days.is_empty() {
            "無".to_owned()
        } else {
            state.off_days.keys().take(12).cloned().collect::<Vec<_>>().join("、")
        };
        log::info!("接下來不用上班的日子：{list}");
    }

    /// 今天是記下的不用上班日、又還沒讀到今天的班表：直接當作今天的班表，不問 Hayaku。回傳有沒有套用。
    pub(super) fn use_cached_off_day(&self, today: NaiveDate) -> bool {
        if !self.0.config.borrow().sync_only_during_work || self.s().schedule.as_ref().is_some_and(|s| s.date == today)
        {
            return false;
        }
        let Some(off) = OffDayCache::today(&self.0.state.borrow().off_days, today) else {
            return false;
        };
        let what = if off.on_leave {
            "請假".to_owned()
        } else if let Some(h) = &off.holiday {
            format!("放假（{h}）")
        } else {
            "不上班".to_owned()
        };
        {
            let mut s = self.s_mut();
            s.schedule = Some(off);
            s.schedule_fetched = MIN; // 按重新整理時會重抓真正的班表
        }
        log::info!("今天{what}，不自動同步");
        true
    }

    /// 今天不用上班（請假、放假、週末）、也沒有計時在跑：不自動送任何請求。
    pub(super) fn off_today(&self, now: NaiveDateTime) -> bool {
        if !self.0.config.borrow().sync_only_during_work {
            return false;
        }
        let s = self.s();
        s.schedule.as_ref().is_some_and(|sc| sc.date == now.date() && sc.working.is_empty())
            && !s.entries.iter().any(|e| e.running)
    }

    /// 這次下班後的工作算哪一天做完的（yyyy-MM-dd）。
    fn covered_day(&self, now: NaiveDateTime) -> String {
        let day = match self.s().schedule.as_ref().filter(|s| s.date == now.date()) {
            Some(s) => LeavePlanner::covered_day(s, self.0.state.borrow().clocked_out, now),
            None => now.date(),
        };
        time::ymd(day)
    }

    pub(super) fn cached_user(&self) -> Option<UserInfo> {
        let state = self.0.state.borrow();
        let name = state.user_name.clone().filter(|n| !n.is_empty())?;
        let display = state.user_display_name.clone().unwrap_or_else(|| name.clone());
        Some(UserInfo { username: name, display_name: display, hayaku_version: None })
    }

    /// 同步一次，回傳結果說明（設定頁的「立即同步」也用這個）。
    /// morning：上班前那次（記在 MorningDay，跟下班那次分開）；也會照下班那次的規則記 LastSyncDay，開機補做的不會再跑一次。
    pub async fn sync_eip_leaves(&self, manual: bool, morning: bool) -> String {
        if self.s().eip_syncing {
            return "同步中…".to_owned();
        }
        let cred = EipCredentialStore::load();
        if cred.is_empty() {
            if morning {
                self.record(manual, morning, time::now(), "失敗：尚未設定 EIP 帳密", true); // 不要讓自動上班一直等
            }
            return "尚未設定 EIP 帳密".to_owned();
        }
        if !self.logged_in() {
            return "還沒登入 Hayaku".to_owned();
        }

        self.s_mut().eip_syncing = true;
        self.raise();
        let now = time::now();
        let result = match self.sync_eip_core(&cred, now).await {
            Ok((added, cancelled, skipped, failed)) => {
                let result = describe(&added, &cancelled, &skipped, &failed);
                self.record(manual, morning, now, &result, true);
                self.s_mut().eip_retry_at = MIN;
                log::info!("EIP 請假同步{}：{result}", if morning { "（上班前）" } else { "" });
                if added.len() + cancelled.len() > 0 {
                    self.s_mut().schedule_fetched = MIN; // 班表的「請假」要跟著更新（今天整天請假就不會自動上班）
                }
                if added.len() + cancelled.len() + failed.len() > 0 {
                    let e = self.clone();
                    self.toast(ToastRequest::new(
                        if failed.is_empty() { "已同步 EIP 請假" } else { "EIP 請假有登記失敗的" },
                        result.clone(),
                        vec![ToastAction::new("查看請假", move || {
                            let e = e.clone();
                            async move { e.request_workspace("leaves", None) }
                        })],
                    ));
                }
                result
            }
            Err(CoreError::EipLogin(message)) => {
                // 帳密錯誤：今天不再自動重試，等使用者到設定更新
                let result = format!("{message}，請到設定更新");
                self.record(manual, morning, now, &format!("失敗：{result}"), true);
                log::info!("EIP 請假同步失敗：{message}");
                if !manual {
                    self.toast(ToastRequest::new("EIP 請假同步失敗", result.clone(), Vec::new()));
                }
                result
            }
            Err(err) => {
                let result =
                    if err.is_http() { format!("連不上 EIP：{}", err.message()) } else { err.message().to_owned() };
                self.record(manual, morning, now, &format!("失敗：{result}"), false);
                self.s_mut().eip_retry_at = now + EIP_RETRY_DELAY;
                log::error!("EIP 請假同步失敗: {err}");
                result
            }
        };
        self.s_mut().eip_syncing = false;
        self.raise();
        result
    }

    /// 記下這次的結果；done：今天這一次算做完了（失敗要重試的不算）。手動同步不算：上班前、下班時照樣再讀。
    fn record(&self, manual: bool, morning: bool, now: NaiveDateTime, result: &str, done: bool) {
        let covered = if done && !manual { Some(self.covered_day(now)) } else { None };
        let mut st = self.0.eip_state.borrow_mut();
        if let Some(covered) = covered {
            st.last_sync_day = covered;
            if morning {
                st.morning_day = time::ymd(now.date());
            }
        }
        if !manual && morning {
            st.morning_at = Some(now);
            st.morning_result = result.to_owned();
        } else if !manual {
            st.after_work_at = Some(now);
            st.after_work_result = result.to_owned();
        }
        st.last_sync_at = Some(now);
        st.last_result = result.to_owned();
        st.save();
    }

    async fn sync_eip_core(&self, cred: &EipCredentials, now: NaiveDateTime) -> Result<SyncLists, CoreError> {
        let today = now.date();
        let mut days = self.0.api.get_schedule_range(today, add_days(today, EIP_SCHEDULE_DAYS)).await?;
        self.update_off_days(&days, today); // EIP 連不上也先記下來
        let next = LeavePlanner::next_workday(&days, today).unwrap_or_else(|| add_days(today, 1));
        // 上次自動同步之後漏掉的日子（例如臨時請整天假、那天電腦沒開）一起補登，要用那幾天的班表
        let back = LeavePlanner::backfill_from(&self.0.eip_state.borrow().last_sync_day, today);
        if back < today {
            let mut earlier = self.0.api.get_schedule_range(back, add_days(today, -1)).await?;
            earlier.append(&mut days);
            days = earlier;
        }
        let window_from = back.and_time(NaiveTime::MIN);
        let window_to = add_days(next, 1).and_time(NaiveTime::MIN);

        // EIP：待審＋核准；年份涵蓋這次的範圍和之前同步過、還沒結束的假單
        let mut years: Vec<i32> =
            self.0.eip_state.borrow().leaves.iter().map(|l| chrono::Datelike::year(&l.start)).collect();
        years.extend([chrono::Datelike::year(&back), chrono::Datelike::year(&today), chrono::Datelike::year(&next)]);
        years.sort();
        years.dedup();
        let mut rows: Vec<EipLeave> = Vec::new();
        {
            let _permit = self.0.eip_gate.acquire().await.map_err(|e| CoreError::Other(e.to_string()))?;
            let eip = self.eip_client();
            for year in years {
                rows.extend(eip.list_leaves(year, EipClient::STATE_PENDING, cred).await?);
                rows.extend(eip.list_leaves(year, EipClient::STATE_APPROVED, cred).await?);
            }
        }
        let leaves = LeavePlanner::effective(&rows);

        let mut added = Vec::new();
        let mut cancelled = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();
        let mut dirty = false;

        // 1. 同步建立的：這次範圍以前就結束的不用再管；已經過去、還在範圍內的留著（免得重複登記），不回頭取消；
        //    EIP 駁回、抽單、改了時間的取消
        let synced: Vec<SyncedLeave> = self.0.eip_state.borrow().leaves.clone();
        for rec in synced {
            if rec.end <= window_from {
                self.remove_synced(&rec.no);
                dirty = true;
                continue;
            }
            if rec.end <= now {
                continue;
            }
            let current = leaves.iter().find(|l| l.no == rec.no);
            if current.is_some_and(|l| l.start == rec.start && l.end == rec.end) {
                continue;
            }
            self.cancel_synced(&rec).await?;
            self.remove_synced(&rec.no);
            dirty = true;
            if current.is_none() {
                cancelled.push(rec.summary.clone()); // 改時間的會在下面重新登記，不用列成取消
            }
        }

        // 2. 跟「補登的日子（通常沒有）＋今天到下一個上班日」重疊、還沒登記的假單：整張登記（已經開始、已經過去的也登記）
        let targets: Vec<EipLeave> = {
            let st = self.0.eip_state.borrow();
            leaves
                .iter()
                .filter(|l| l.end > window_from && l.start < window_to)
                .filter(|l| st.leaves.iter().all(|r| r.no != l.no))
                .cloned()
                .collect()
        };
        if !targets.is_empty() {
            let from = targets.iter().map(|l| l.start).min().unwrap_or(window_from).date();
            let to = targets.iter().map(|l| l.end).max().unwrap_or(window_to).date();
            if to > add_days(today, EIP_SCHEDULE_DAYS) {
                let more = self
                    .0
                    .api
                    .get_schedule_range(
                        add_days(today, EIP_SCHEDULE_DAYS + 1),
                        to.min(add_days(today, EIP_SCHEDULE_DAYS * 2)),
                    )
                    .await?;
                days.extend(more);
            }
            let working = LeavePlanner::working(&days);
            let mine = self.0.api.list_leaves(from, to, Some("me"), Some("LEAVE")).await?; // 出差不算，免得跟出差時段重疊的假被當成登記過

            for leave in targets {
                let pieces = LeavePlanner::plan(&leave, &working);
                if pieces.is_empty() {
                    continue;
                }
                let summary = format!(
                    "{} {}",
                    pieces.iter().map(LeavePiece::describe).collect::<Vec<_>>().join("、"),
                    leave.r#type
                );
                // Hayaku 那段時間已經有請假（手動登記過、或別人幫忙登記）：不重複登記
                if pieces.iter().any(|p| {
                    mine.iter().any(|m| {
                        LeavePlanner::overlaps(p.from(), p.to(), time::local(&m.start_at), time::local(&m.end_at))
                    })
                }) {
                    skipped.push(summary);
                    continue;
                }
                let note = format!(
                    "EIP {} {}{}",
                    leave.no,
                    leave.r#type,
                    if leave.reason.is_empty() { String::new() } else { format!("：{}", leave.reason) }
                );
                // 先記下來再登記：中途失敗時已經登記的也找得到（之後抽單才取消得到）；
                // 找不到 id 也照樣記，避免下次重複登記（只是之後 EIP 抽單時沒辦法自動取消）
                self.0.eip_state.borrow_mut().leaves.push(SyncedLeave {
                    no: leave.no.clone(),
                    r#type: leave.r#type.clone(),
                    start: leave.start,
                    end: leave.end,
                    hayaku_ids: Vec::new(),
                    summary: summary.clone(),
                });
                dirty = true;
                let mut error: Option<CoreError> = None;
                for piece in &pieces {
                    let id = match self.0.api.add_leave(piece.args(&note)).await {
                        Ok(Some(id)) => Some(id),
                        Ok(None) => match self.find_leave_id(piece, &leave.no).await {
                            Ok(id) => id,
                            Err(err) => {
                                error = Some(err);
                                break;
                            }
                        },
                        Err(err) => {
                            error = Some(err.into());
                            break;
                        }
                    };
                    let mut st = self.0.eip_state.borrow_mut();
                    if let Some(id) = id
                        && let Some(rec) = st.leaves.iter_mut().rev().find(|r| r.no == leave.no)
                    {
                        rec.hayaku_ids.push(id);
                    }
                    st.save();
                }
                let ids = self
                    .0
                    .eip_state
                    .borrow()
                    .leaves
                    .iter()
                    .rev()
                    .find(|r| r.no == leave.no)
                    .map_or(0, |r| r.hayaku_ids.len());
                if let Some(err) = error {
                    if ids == 0 {
                        self.remove_synced(&leave.no); // 一筆都沒登記成功：下次再試
                    }
                    self.0.eip_state.borrow().save();
                    if !err.is_mcp() || leave.start >= now {
                        return Err(err);
                    }
                    // 已經開始、已經過去的假單 Hayaku 不收（不確定會不會）：記下來告訴使用者，其他照常同步，不整次重試
                    log::error!("EIP {} 登記到 Hayaku 失敗: {err}", leave.no);
                    failed.push(format!("{summary}（{}）", err.message()));
                    continue;
                }
                added.push(summary);
                if ids < pieces.len() {
                    log::info!("EIP {} 登記後找不到 Hayaku 請假 id，之後無法自動取消", leave.no);
                }
            }
        }
        if dirty {
            self.0.eip_state.borrow().save();
        }
        if added.len() + cancelled.len() > 0 {
            match self.0.api.get_schedule_range(today, add_days(today, EIP_SCHEDULE_DAYS)).await {
                Ok(days) => self.update_off_days(&days, today), // 剛登記的請假也算
                Err(err) => log::error!("重新讀取班表失敗: {err}"),
            }
        }
        Ok((added, cancelled, skipped, failed))
    }

    /// 從 eip-sync.json 拿掉某張假單（同一張只會有一筆）。
    fn remove_synced(&self, no: &str) {
        let mut st = self.0.eip_state.borrow_mut();
        if let Some(i) = st.leaves.iter().position(|r| r.no == no) {
            st.leaves.remove(i);
        }
    }

    /// add_leave 沒回 id 時，從自己的請假裡找時間對得上、還沒記下的那筆。
    async fn find_leave_id(&self, piece: &LeavePiece, no: &str) -> Result<Option<String>, CoreError> {
        let to = piece.to() - TimeDelta::nanoseconds(100);
        let mine = self.0.api.list_leaves(piece.date, to.date(), Some("me"), Some("LEAVE")).await?;
        let st = self.0.eip_state.borrow();
        let rec_ids: Vec<String> =
            st.leaves.iter().rev().find(|r| r.no == no).map(|r| r.hayaku_ids.clone()).unwrap_or_default();
        Ok(mine
            .iter()
            .find(|m| {
                minutes_between(time::local(&m.start_at), piece.from()).abs() < 1.0
                    && minutes_between(time::local(&m.end_at), piece.to()).abs() < 1.0
                    && !rec_ids.contains(&m.id)
                    && !st.is_synced(&m.id)
            })
            .map(|m| m.id.clone()))
    }

    async fn cancel_synced(&self, rec: &SyncedLeave) -> Result<(), CoreError> {
        for id in &rec.hayaku_ids {
            match self.0.api.cancel_leave(id).await {
                Ok(()) => {}
                Err(err) if is_mcp(&err) => {
                    log::error!("取消 Hayaku 請假 {id}（EIP {}）失敗，可能已經手動取消: {err}", rec.no)
                }
                Err(err) => return Err(err.into()),
            }
        }
        Ok(())
    }

    fn eip_client(&self) -> Rc<EipClient> {
        self.0.eip.borrow_mut().get_or_insert_with(|| Rc::new(EipClient::new())).clone()
    }

    /// 請假頁「EIP 假單」分頁用：某一年自己的假單（待審、核准、駁回都列），讀過就用暫存，reload 才重讀。
    /// 是使用者打開頁面才讀，不受「不上班的日子不送請求」限制。
    pub async fn load_eip_year(&self, year: i32, reload: bool) -> Result<Rc<Vec<EipLeaveRecord>>, CoreError> {
        if !reload && let Some(cached) = self.s().eip_years.get(&year) {
            return Ok(cached.clone());
        }
        let cred = EipCredentialStore::load();
        if cred.is_empty() {
            return Err(CoreError::EipLogin("還沒設定 EIP 帳號，請到「設定 › EIP 請假」登入".to_owned()));
        }
        let mut list: Vec<EipLeaveRecord> = Vec::new();
        {
            let _permit = self.0.eip_gate.acquire().await.map_err(|e| CoreError::Other(e.to_string()))?;
            let eip = self.eip_client();
            for (state, name) in [
                (EipClient::STATE_PENDING, "待審"),
                (EipClient::STATE_APPROVED, "核准"),
                (EipClient::STATE_REJECTED, "駁回"),
            ] {
                for leave in eip.list_leaves(year, state, &cred).await? {
                    list.push(EipLeaveRecord { leave, state: name.to_owned() });
                }
            }
        }
        let mut distinct: Vec<EipLeaveRecord> = Vec::new();
        for r in list {
            if !distinct.iter().any(|d| d.leave.no == r.leave.no) {
                distinct.push(r);
            }
        }
        distinct.sort_by_key(|r| std::cmp::Reverse(r.leave.start));
        let list = Rc::new(distinct);
        self.s_mut().eip_years.insert(year, list.clone());
        log::info!("讀取 EIP {year} 年假單：{} 筆", list.len());
        Ok(list)
    }

    /// 這張 EIP 假單目前有沒有由同步登記在 Hayaku。
    pub fn is_eip_synced(&self, no: &str) -> bool {
        self.0.eip_state.borrow().leaves.iter().any(|l| l.no == no)
    }

    /// 換了 EIP 帳密：重新登入。
    pub fn reset_eip(&self) {
        *self.0.eip.borrow_mut() = None;
        let mut s = self.s_mut();
        s.eip_years.clear();
        s.eip_retry_at = MIN;
        drop(s);
        let mut st = self.0.eip_state.borrow_mut();
        if st.last_result.starts_with("失敗") {
            st.last_sync_day = String::new(); // 之前因為帳密錯誤停掉的，馬上再試
            st.save();
        }
    }
}

fn minutes_between(a: NaiveDateTime, b: NaiveDateTime) -> f64 {
    hb_api::work_math::total_minutes(a - b)
}

fn describe(added: &[String], cancelled: &[String], skipped: &[String], failed: &[String]) -> String {
    let mut parts = Vec::new();
    if !added.is_empty() {
        parts.push(format!("登記 {}", added.join("、")));
    }
    if !cancelled.is_empty() {
        parts.push(format!("取消 {}", cancelled.join("、")));
    }
    if !skipped.is_empty() {
        parts.push(format!("Hayaku 已經有請假、略過 {}", skipped.join("、")));
    }
    if !failed.is_empty() {
        parts.push(format!("登記失敗，請到網頁登記 {}", failed.join("、")));
    }
    if parts.is_empty() { "沒有要同步的請假".to_owned() } else { parts.join("；") }
}
