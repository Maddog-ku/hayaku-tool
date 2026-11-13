//! 對應 C# `UI/PollWindow.cs`：聊天裡的投票。題目、各選項票數與誰投了、狀態與截止時間（唯讀；MCP 不能投票，投票請到網頁）。

use chrono::Local;
use hb_api::PollInfo;
use hb_core::TimerEngine;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::dialog;
use crate::ui::{PollOptionRow, PollWindow};
use crate::window_util::{self, Placement};

/// 題目下面那一行（類型・匿名・狀態・截止・發起人・幾人投了）。
pub fn meta(poll: &PollInfo) -> String {
    let voted = poll.voted.map(|v| {
        let pending = match poll.pending {
            Some(p) if p > 0 => format!("、{p} 人還沒投"),
            _ => String::new(),
        };
        format!("{v} 人投了{pending}")
    });
    [
        poll.r#type.clone(),
        poll.anonymous.then(|| "匿名".to_owned()),
        poll.status.clone(),
        poll.closes_at.map(|at| format!("{} 截止", at.with_timezone(&Local).format("%-m/%-d %H:%M"))),
        poll.created_by.as_ref().map(|by| format!("{by} 發起")),
        voted,
    ]
    .into_iter()
    .flatten()
    .filter(|m| !m.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" · ")
}

/// 各選項的列（票數比例對最多票的那個）。
pub fn option_rows(poll: &PollInfo) -> Vec<(String, bool, String, Option<f32>, String)> {
    let max = poll.options.iter().map(|o| o.votes.unwrap_or(0)).max().unwrap_or(0).max(1);
    poll.options
        .iter()
        .map(|o| {
            (
                o.label.clone(),
                o.mine,
                o.votes.map(|n| format!("{n} 票")).unwrap_or_default(),
                o.votes.map(|v| v as f32 / max as f32),
                o.voters.join("、"),
            )
        })
        .collect()
}

/// 開投票（poll_id 為 None 時不開）。owner：置中在哪個視窗上。
pub fn show(engine: &TimerEngine, poll_id: Option<&str>, owner: Option<&slint::Window>) {
    let Some(poll_id) = poll_id else { return };
    let Some(ui) = dialog::create("投票", PollWindow::new) else { return };
    let id = window_util::keep(&ui, None, ());
    {
        let weak = ui.as_weak();
        ui.on_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                window_util::close(&ui, id);
            }
        });
    }
    window_util::present(&ui, id, Placement::Owner(owner), true, (460.0, 300.0), crate::apply_theme);
    let weak = ui.as_weak();
    let engine = engine.clone();
    let poll_id = poll_id.to_owned();
    dialog::spawn(async move {
        let result = engine.api().get_poll(&poll_id).await;
        let Some(ui) = weak.upgrade() else { return };
        match result {
            Err(e) => {
                log::error!("讀取投票 {poll_id} 失敗: {}", e.0);
                ui.set_loading(false);
                ui.set_error(format!("讀不到投票：{}", e.0).into());
                // 沒有內容：題目區收起來
                ui.set_question("".into());
            }
            Ok(poll) => {
                ui.set_question(poll.question.as_str().into());
                ui.set_meta(meta(&poll).into());
                let rows: Vec<PollOptionRow> = option_rows(&poll)
                    .into_iter()
                    .map(|(label, mine, votes, ratio, voters)| PollOptionRow {
                        label: label.into(),
                        mine,
                        votes_text: votes.into(),
                        has_votes: ratio.is_some(),
                        ratio: ratio.unwrap_or(0.0),
                        voters: voters.into(),
                    })
                    .collect();
                ui.set_options(ModelRc::new(VecModel::from(rows)));
                ui.set_hidden_results(poll.options.iter().all(|o| o.votes.is_none()));
                ui.set_loading(false);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::PollOption;

    #[test]
    fn 說明列與票數比例() {
        let poll = PollInfo {
            question: "午餐".into(),
            place: None,
            r#type: Some("單選".into()),
            anonymous: true,
            status: Some("進行中".into()),
            closes_at: None,
            created_by: Some("小明".into()),
            options: vec![
                PollOption { label: "便當".into(), votes: Some(4), voters: vec!["A".into(), "B".into()], mine: true },
                PollOption { label: "麵".into(), votes: Some(2), voters: vec![], mine: false },
            ],
            voted: Some(6),
            pending: Some(2),
        };
        assert_eq!("單選 · 匿名 · 進行中 · 小明 發起 · 6 人投了、2 人還沒投", meta(&poll));
        let rows = option_rows(&poll);
        assert_eq!(
            ("4 票".to_owned(), Some(1.0), "A、B".to_owned()),
            (rows[0].2.clone(), rows[0].3, rows[0].4.clone())
        );
        assert_eq!(Some(0.5), rows[1].3);
    }
}
