//! 工作台「團隊狀況」頁（對應 C# `UI/Work/TeamPage.xaml(.cs)`）。
//! 團隊今天的狀況：誰在線、誰請假、誰正在計時哪個任務。
//! 正在計時用專案的顏色、標上層任務（team_schedule 沒給上層，打開這頁才對看得到的任務補讀 get_task，同一個任務只讀一次）。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use chrono::Local;
use hb_api::{KeyFinder, TeamMember, TeamTiming as ApiTiming, WorkMath};
use hb_core::TimerEngine;
use hb_ui_common::ProjectIcons;
use hb_ui_common::inputs::StatusMessage;
use slint::ComponentHandle;

use super::meetings::w3_common::{model, spawn, status_parts};
use crate::{PageArg, TeamPageData, TeamPerson, TeamTiming, WorkCtx};

struct Page {
    ctx: WorkCtx,
    version: RefCell<u64>,
}

thread_local! {
    static PAGE: RefCell<Option<Rc<Page>>> = const { RefCell::new(None) };
}

pub fn init(ctx: &WorkCtx) {
    let page = Rc::new(Page { ctx: ctx.clone(), version: RefCell::new(0) });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let Some(w) = ctx.window() else { return };
    let g = w.global::<TeamPageData>();
    let weak: Weak<Page> = Rc::downgrade(&page);
    g.on_reload(move || {
        if let Some(page) = weak.upgrade() {
            spawn(async move { page.load().await });
        }
    });
    let weak: Weak<Page> = Rc::downgrade(&page);
    g.on_timing_clicked(move |key| {
        if let Some(page) = weak.upgrade() {
            page.ctx.show_page("tasks", Some(PageArg::Text(key.to_string())));
        }
    });
}

pub async fn activate(_ctx: WorkCtx, _arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    page.load().await;
}

impl Page {
    fn with<R>(&self, f: impl FnOnce(&TeamPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<TeamPageData>();
        Some(f(&g))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_status(text);
            g.set_status_tone(tone);
        });
    }

    async fn load(self: Rc<Self>) {
        let version = {
            let mut v = self.version.borrow_mut();
            *v += 1;
            *v
        };
        self.show(None);
        self.with(|g| g.set_counts("讀取中…".into()));
        let engine = self.ctx.engine.clone();
        let team = match engine.api().team_schedule().await {
            Ok(team) => team,
            Err(e) => {
                log::error!("讀取團隊狀況失敗: {e}");
                self.with(|g| g.set_counts("".into()));
                self.show(Some(StatusMessage::error(format!("讀取失敗：{e}"))));
                return;
            }
        };
        if version != *self.version.borrow() {
            return;
        }
        self.render(&team);
        self.with(|g| g.set_counts(counts_text(&team).into()));

        let mut missing: Vec<String> = Vec::new();
        for t in team.iter().flat_map(|m| m.timing.iter()).filter(|t| t.can_open && !t.task.is_empty()) {
            if !missing.iter().any(|k| k.eq_ignore_ascii_case(&t.task)) && !engine.known_parent_title(&t.task).1 {
                missing.push(t.task.clone());
            }
        }
        if missing.is_empty() {
            return;
        }
        futures_util::future::join_all(missing.iter().map(|key| {
            let engine = engine.clone();
            async move {
                if let Err(e) = engine.load_parent_title(key).await {
                    log::error!("讀取 {key} 的上層任務失敗: {}", e.message());
                }
            }
        }))
        .await;
        if version == *self.version.borrow() {
            self.render(&team);
        }
    }

    fn render(&self, team: &[TeamMember]) {
        let engine = &self.ctx.engine;
        let people = sorted_people(team, engine.user().map(|u| u.username).as_deref());
        let vms: Vec<TeamPerson> = people.into_iter().map(|m| person(m, engine)).collect();
        // 兩欄各自往下排，卡片高度不同也不會留白
        let left: Vec<TeamPerson> = vms.iter().step_by(2).cloned().collect();
        let right: Vec<TeamPerson> = vms.iter().skip(1).step_by(2).cloned().collect();
        self.with(|g| {
            g.set_left(model(left));
            g.set_right(model(right));
        });
    }
}

/// 自己最前面，再來沒請假的、計時中多的、在線的（穩定排序）。
pub(crate) fn sorted_people<'a>(team: &'a [TeamMember], me: Option<&str>) -> Vec<&'a TeamMember> {
    let mut people: Vec<&TeamMember> = team.iter().collect();
    people.sort_by(|a, b| {
        let mine = |m: &TeamMember| if Some(m.user.username.as_str()) == me { 0 } else { 1 };
        mine(a)
            .cmp(&mine(b))
            .then(a.on_leave.cmp(&b.on_leave))
            .then(b.timing.len().cmp(&a.timing.len()))
            .then(b.online.cmp(&a.online))
    });
    people
}

pub(crate) fn counts_text(team: &[TeamMember]) -> String {
    format!(
        "共 {} 人 · 在線 {} · 計時中 {} · 請假 {}（只列跟你有共同專案的人）",
        team.len(),
        team.iter().filter(|m| m.online).count(),
        team.iter().filter(|m| !m.timing.is_empty()).count(),
        team.iter().filter(|m| m.on_leave).count()
    )
}

/// C# `TeamMemberVm`。
fn person(m: &TeamMember, engine: &TimerEngine) -> TeamPerson {
    let (state, state_tone) = member_state(m);
    TeamPerson {
        name: m.user.display_name.as_str().into(),
        initial: m.user.display_name.chars().next().map_or("?".to_owned(), |c| c.to_string()).into(),
        job: m.job_title.clone().unwrap_or_default().into(),
        dot: if m.on_leave {
            0
        } else if m.online {
            1
        } else {
            2
        },
        state: state.into(),
        state_tone,
        timing: model(m.timing.iter().map(|t| timing(t, engine)).collect()),
    }
}

/// 狀態文字與色調（0 警告、1 計時中、2 淡色）。
pub(crate) fn member_state(m: &TeamMember) -> (String, i32) {
    if m.on_leave {
        (format!("請假 {}", m.leaves.join("、")), 0)
    } else if !m.timing.is_empty() {
        (format!("計時中 {}", m.timing.len()), 1)
    } else if m.online {
        ("在線".to_owned(), 2)
    } else {
        ("離線".to_owned(), 2)
    }
}

/// C# `TimingVm`：專案的顏色、「上層任務 › 」、計時多久、提示。
fn timing(t: &ApiTiming, engine: &TimerEngine) -> TeamTiming {
    let project_key = KeyFinder::project_of(&t.task);
    let project = engine.project_of(&t.project).or_else(|| engine.project_of(project_key));
    let color = ProjectIcons::color_of(
        project.as_ref().and_then(|p| p.icon_color.as_deref()),
        project.as_ref().map_or(project_key, |p| p.key.as_str()),
    );
    let parent = engine.known_parent_title(&t.task).0.filter(|p| !p.is_empty());
    let since = t
        .since
        .map(|s| {
            format!(
                "· {}",
                WorkMath::hours_minutes((Local::now().fixed_offset() - s).num_milliseconds() as f64 / 60000.0)
            )
        })
        .unwrap_or_default();
    TeamTiming {
        key: t.task.as_str().into(),
        title: t.title.as_str().into(),
        parent_prefix: parent.as_ref().map(|p| format!("{p} › ")).unwrap_or_default().into(),
        since: since.into(),
        tip: timing_tip(&t.project, parent.as_deref(), t.can_open).into(),
        can_open: t.can_open,
        dot: color.to_slint(),
    }
}

pub(crate) fn timing_tip(project: &str, parent: Option<&str>, can_open: bool) -> String {
    format!(
        "{project}{}{}",
        parent.map(|p| format!("\n上層：{p}")).unwrap_or_default(),
        if can_open { "\n點一下打開" } else { "" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::PersonRef;

    fn member(user: &str, online: bool, on_leave: bool, timing: usize) -> TeamMember {
        TeamMember {
            user: PersonRef::new(user, user.to_uppercase()),
            job_title: None,
            online,
            on_leave,
            leaves: if on_leave { vec!["上午".into(), "特休".into()] } else { vec![] },
            timing: (0..timing)
                .map(|i| ApiTiming {
                    task: format!("F2P-{i}"),
                    title: "t".into(),
                    project: "F2P".into(),
                    since: None,
                    can_open: true,
                })
                .collect(),
        }
    }

    #[test]
    fn sorted_people_自己最前_請假最後() {
        let team = vec![
            member("leave", true, true, 0),
            member("off", false, false, 0),
            member("on", true, false, 0),
            member("busy", true, false, 2),
            member("me", false, false, 0),
        ];
        let order: Vec<_> = sorted_people(&team, Some("me")).iter().map(|m| m.user.username.clone()).collect();
        assert_eq!(vec!["me", "busy", "on", "off", "leave"], order);
        assert_eq!("共 5 人 · 在線 3 · 計時中 1 · 請假 1（只列跟你有共同專案的人）", counts_text(&team));
    }

    #[test]
    fn member_state_文字() {
        assert_eq!(("請假 上午、特休".to_owned(), 0), member_state(&member("a", true, true, 1)));
        assert_eq!(("計時中 2".to_owned(), 1), member_state(&member("a", true, false, 2)));
        assert_eq!(("在線".to_owned(), 2), member_state(&member("a", true, false, 0)));
        assert_eq!(("離線".to_owned(), 2), member_state(&member("a", false, false, 0)));
        assert_eq!("F2P\n上層：報表\n點一下打開", timing_tip("F2P", Some("報表"), true));
        assert_eq!("F2P", timing_tip("F2P", None, false));
    }
}
