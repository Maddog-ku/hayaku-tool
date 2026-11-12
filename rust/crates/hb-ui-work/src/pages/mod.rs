//! 工作台各頁。每頁一個模組，簽章見各檔開頭的契約。

pub mod activity;
pub mod chat;
pub mod entries;
pub mod forum;
pub mod hours;
pub mod leaves;
pub mod meetings;
pub mod notes;
pub mod projects;
pub mod reminders;
pub mod reviews;
pub mod spaces;
pub mod stats;
pub mod tasks;
pub mod team;
pub mod tests;
pub mod workload;
pub mod worklog;

use crate::{PageArg, WorkCtx};

pub(crate) fn init(index: usize, ctx: &WorkCtx) {
    match index {
        0 => tasks::init(ctx),
        1 => projects::init(ctx),
        2 => tests::init(ctx),
        3 => reviews::init(ctx),
        4 => hours::init(ctx),
        5 => entries::init(ctx),
        6 => worklog::init(ctx),
        7 => reminders::init(ctx),
        8 => meetings::init(ctx),
        9 => chat::init(ctx),
        10 => notes::init(ctx),
        11 => spaces::init(ctx),
        12 => forum::init(ctx),
        13 => team::init(ctx),
        14 => activity::init(ctx),
        15 => workload::init(ctx),
        16 => leaves::init(ctx),
        17 => stats::init(ctx),
        _ => {}
    }
}

pub(crate) async fn activate(index: usize, ctx: WorkCtx, arg: Option<PageArg>) {
    match index {
        0 => tasks::activate(ctx, arg).await,
        1 => projects::activate(ctx, arg).await,
        2 => tests::activate(ctx, arg).await,
        3 => reviews::activate(ctx, arg).await,
        4 => hours::activate(ctx, arg).await,
        5 => entries::activate(ctx, arg).await,
        6 => worklog::activate(ctx, arg).await,
        7 => reminders::activate(ctx, arg).await,
        8 => meetings::activate(ctx, arg).await,
        9 => chat::activate(ctx, arg).await,
        10 => notes::activate(ctx, arg).await,
        11 => spaces::activate(ctx, arg).await,
        12 => forum::activate(ctx, arg).await,
        13 => team::activate(ctx, arg).await,
        14 => activity::activate(ctx, arg).await,
        15 => workload::activate(ctx, arg).await,
        16 => leaves::activate(ctx, arg).await,
        17 => stats::activate(ctx, arg).await,
        _ => {}
    }
}

/// 頁面代號與導覽文字，順序就是導覽順序（也是 Ctrl+1～9、0 的順序）。
pub const PAGES: [(&str, &str); 18] = [
    ("tasks", "我的任務"),
    ("projects", "專案"),
    ("tests", "測試"),
    ("reviews", "待我審核"),
    ("hours", "我的工時"),
    ("entries", "工時紀錄"),
    ("worklog", "工作日誌"),
    ("reminders", "提醒"),
    ("meetings", "會議"),
    ("chat", "聊天"),
    ("notes", "筆記"),
    ("spaces", "團隊空間"),
    ("forum", "論壇"),
    ("team", "團隊狀況"),
    ("activity", "專案動態"),
    ("workload", "工作負荷"),
    ("leaves", "請假"),
    ("stats", "統計"),
];
