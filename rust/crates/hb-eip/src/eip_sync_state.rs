//! 對應 C# `Eip/EipSyncState.cs`：EIP 請假同步的狀態，存在 %APPDATA%\HayakuBar\eip-sync.json。

use std::path::Path;

use chrono::NaiveDateTime;
use hb_backend::infra::{AppPaths, json_file};
use serde::{Deserialize, Serialize};

use crate::net_time;

/// 由同步建立的 Hayaku 請假；只有這些會因為 EIP 駁回、抽單而被取消。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncedLeave {
    /// EIP 單號。
    pub no: String,
    pub r#type: String,
    #[serde(with = "net_time::unspecified")]
    pub start: NaiveDateTime,
    #[serde(with = "net_time::unspecified")]
    pub end: NaiveDateTime,
    pub hayaku_ids: Vec<String>,

    /// 給人看的說明，例「11/3（一）整天 特休」。
    pub summary: String,
}

impl Default for SyncedLeave {
    fn default() -> Self {
        Self {
            no: String::new(),
            r#type: String::new(),
            start: net_time::min_value(),
            end: net_time::min_value(),
            hayaku_ids: Vec::new(),
            summary: String::new(),
        }
    }
}

/// EIP 請假同步的狀態。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EipSyncState {
    /// 上次成功同步的日期（yyyy-MM-dd），一天只自動同步一次。
    pub last_sync_day: String,
    #[serde(with = "net_time::local_opt")]
    pub last_sync_at: Option<NaiveDateTime>,

    /// 上次同步的結果（設定頁顯示）。
    pub last_result: String,

    /// 上班前那次做過的日期（yyyy-MM-dd），一天一次。
    pub morning_day: String,
    #[serde(with = "net_time::local_opt")]
    pub morning_at: Option<NaiveDateTime>,
    pub morning_result: String,

    /// 下班那次（含開機補做）最後一次的時間與結果。
    #[serde(with = "net_time::local_opt")]
    pub after_work_at: Option<NaiveDateTime>,
    pub after_work_result: String,

    pub leaves: Vec<SyncedLeave>,
}

impl EipSyncState {
    pub fn is_synced(&self, hayaku_id: &str) -> bool {
        self.leaves.iter().any(|l| l.hayaku_ids.iter().any(|id| id == hayaku_id))
    }

    pub fn load() -> Self {
        Self::load_from(&AppPaths::eip_sync())
    }

    pub fn save(&self) {
        self.save_to(&AppPaths::eip_sync());
    }

    pub fn load_from(path: &Path) -> Self {
        json_file::load(path)
    }

    pub fn save_to(&self, path: &Path) {
        json_file::save(path, self);
    }
}
