//! 對應 C# `UI/HoverCard.xaml.cs`：滑鼠停在工作列的工時上時，顯示今天各任務的明細（不接收滑鼠、不搶焦點）。
//! 出現時從工作列往上長出來並淡入，離開時滑回工作列並淡出（[`crate::slide`]；「顯示動畫」關閉時直接出現、消失）。

use chrono::Local;
use hb_api::{TimeEntry, WorkMath};
use hb_core::TimerEngine;
use hb_ui_common::{DayTimelineData, TaskRowVm};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::popup::{self, PopupKind};
use crate::slide::WindowSlide;
use crate::theme_manager::ThemeManager;
use crate::ui::{DayTimelineModel, HoverCard as HoverCardWindow, HoverRow};

/// 長出來、滑回去的時間（完整走一趟）。
const SHOW_DURATION: std::time::Duration = std::time::Duration::from_millis(220);
const HIDE_DURATION: std::time::Duration = std::time::Duration::from_millis(170);

pub struct HoverCard {
    ui: HoverCardWindow,
    /// 第一次顯示過了（之後用 Win32 顯示／隱藏，才不會搶焦點）。
    created: std::cell::Cell<bool>,
    /// 顯示中（滑回去的途中不算）
    visible: std::cell::Cell<bool>,
    slide: WindowSlide,
}

/// 今天各任務：計時中的在前，再依今天累計多到少，最多 8 個（C# 的 LINQ）。
pub fn summary_rows(engine_rows: Vec<(String, String, bool, f64)>) -> Vec<(String, String, bool, f64)> {
    let mut rows = engine_rows;
    // OrderByDescending(Running).ThenByDescending(Minutes)：穩定排序
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal)));
    rows.truncate(8);
    rows
}

/// 依任務分組（不分大小寫，保留第一次出現的順序）：(編號, 最後一筆的名稱)。
pub fn group_entries(entries: &[TimeEntry]) -> Vec<(String, String)> {
    let mut groups: Vec<(String, String)> = Vec::new();
    for e in entries {
        match groups.iter_mut().find(|(k, _)| k.to_uppercase() == e.task.to_uppercase()) {
            Some(g) => g.1 = e.title.clone(),
            None => groups.push((e.task.clone(), e.title.clone())),
        }
    }
    groups
}

impl HoverCard {
    pub fn new() -> Option<Self> {
        match popup::create(PopupKind::Hover, HoverCardWindow::new) {
            Ok(ui) => {
                crate::track_theme!(ui);
                Some(Self {
                    ui,
                    created: std::cell::Cell::new(false),
                    visible: std::cell::Cell::new(false),
                    slide: WindowSlide::new(),
                })
            }
            Err(e) => {
                log::error!("建立工時摘要失敗: {e}");
                None
            }
        }
    }

    /// C# `ShowFor`：anchor 是工作列上工時的位置（實體像素）。
    pub fn show_for(&self, engine: &TimerEngine, anchor: hb_win::Rect, align_left: bool) {
        let now = Local::now().naive_local();
        let today = now.date();
        let day = today.and_hms_opt(0, 0, 0).unwrap_or(now);
        let entries = engine.entries();
        let schedule = engine.schedule();
        let worked = WorkMath::union_minutes(entries.iter(), day, now);
        let expected = schedule.as_ref().map_or(0, |s| s.expected_minutes) as f64;
        let mut header = format!("今天 {}", WorkMath::hours_minutes(worked));
        if expected > 0.0 {
            header += &format!(" / {}", WorkMath::hours_minutes(expected));
        }
        self.ui.set_header(header.into());
        let phase = WorkMath::phase(schedule.as_ref(), now, engine.state().clocked_out).text;
        let mut parts: Vec<String> = Vec::new();
        if !phase.is_empty() {
            parts.push(phase);
        }
        if !WorkMath::is_filled(worked, expected) {
            parts.push(format!("還差 {}", WorkMath::hours_minutes(expected - worked)));
        }
        let unread = engine.unread_count();
        if unread > 0 {
            parts.push(format!("{unread} 則未讀通知"));
        }
        self.ui.set_sub(parts.join(" · ").into());
        let data = DayTimelineData::new(today, schedule.as_ref().map(|s| s.working.as_slice()), &entries, now, true);
        self.ui.set_timeline(hb_ui_common::day_timeline_model!(DayTimelineModel, data));

        let running = engine.running_keys();
        let rows: Vec<(String, String, bool, f64)> = group_entries(&entries)
            .into_iter()
            .map(|(key, title)| {
                let minutes = WorkMath::task_minutes(entries.iter(), &key, day, now);
                let is_running = running.contains(&key);
                (key, title, is_running, minutes)
            })
            .collect();
        let rows: Vec<HoverRow> = summary_rows(rows)
            .into_iter()
            .map(|(key, title, is_running, minutes)| {
                let vm = TaskRowVm {
                    is_claude: engine.is_claude_owned(&key),
                    parent_title: engine.parent_title_of(&key),
                    key,
                    title,
                    running: is_running,
                    elapsed: WorkMath::clock(minutes),
                    ..Default::default()
                };
                HoverRow {
                    icon: vm.icon().into(),
                    icon_color: vm.icon_color().code(),
                    rest: format!(" {}{}", vm.parent_prefix(), vm.title).into(),
                    key: vm.key.as_str().into(),
                    elapsed: vm.elapsed.as_str().into(),
                }
            })
            .collect();
        self.ui.set_rows(ModelRc::new(VecModel::from(rows)));

        // 先用 Slint 算的大小擺好位置再顯示；顯示後用實際大小再擺一次
        let reduce = ThemeManager::reduce_motion();
        let was_visible = self.visible.replace(true);
        if !was_visible {
            // 從工作列那一側長出來：先擺在藏起來的位置（滑回去的途中被叫回來時，從目前的位置接著播）
            let start = if reduce {
                1.0
            } else if self.slide.is_running() {
                self.slide.progress()
            } else {
                0.0
            };
            if !self.slide.is_running() {
                self.ui.invoke_hide_content();
            }
            self.place(anchor, align_left, Some(start));
            if !self.created.get() {
                if let Err(e) = self.ui.show() {
                    log::error!("顯示工時摘要失敗: {e}");
                    self.visible.set(false);
                    return;
                }
                popup::apply_styles(self.ui.window(), PopupKind::Hover);
                self.created.set(true);
            } else {
                popup::show_no_activate(self.ui.window());
            }
        }
        // 事件迴圈還沒開始時連原生視窗都還沒有，沒辦法播：直接出現在定位
        let ready = popup::hwnd(self.ui.window()).is_some();
        if !ready {
            self.slide.jump(1.0);
        }
        self.place(anchor, align_left, None);
        self.ui.invoke_show_content();
        if reduce {
            self.slide.stop();
        } else if !was_visible && ready {
            self.slide.run(1.0, SHOW_DURATION, || {});
        }
    }

    /// 擺到工時正上方；p 是進度（None 照目前的進度，0 藏在工作列那一側、1 在定位）。
    fn place(&self, anchor: hb_win::Rect, align_left: bool, p: Option<f32>) {
        let scale = popup::scale_at(anchor.left, anchor.top - 1);
        let size = if self.created.get() {
            // 內容變了大小也會跟著變：用 Slint 算的大小（實體像素）
            let w = (self.ui.get_pref_w() * scale).round() as i32;
            let h = (self.ui.get_pref_h() * scale).round() as i32;
            popup::outer_size(self.ui.window())
                .map_or((w, h), |(ow, oh)| if (ow - w).abs() > 2 || (oh - h).abs() > 2 { (w, h) } else { (ow, oh) })
        } else {
            ((self.ui.get_pref_w() * scale).round() as i32, (self.ui.get_pref_h() * scale).round() as i32)
        };
        let (target, clip, edge) = popup::above_anchor(anchor, align_left, size, scale);
        let hwnd = popup::hwnd(self.ui.window());
        match p {
            Some(p) => self.slide.prepare(hwnd, target, clip, edge, popup::corner_radius(scale), p),
            None => self.slide.retarget(hwnd, target, clip, edge),
        }
        if hwnd.is_none() {
            let (x, y) = self.slide.position();
            self.ui.window().set_position(slint::PhysicalPosition::new(x, y));
        }
    }

    /// 滑回工作列並淡出，播完才藏起來（途中又要顯示時由 show_for 接手）。
    pub fn hide(&self) {
        if !self.visible.replace(false) {
            return;
        }
        if ThemeManager::reduce_motion() {
            self.slide.stop();
            popup::hide_native(self.ui.window());
            return;
        }
        self.ui.invoke_fade_content();
        let window = self.ui.as_weak();
        self.slide.run(0.0, HIDE_DURATION, move || {
            if let Some(ui) = window.upgrade() {
                popup::hide_native(ui.window());
            }
        });
    }

    pub fn window(&self) -> &slint::Window {
        self.ui.window()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(task: &str, title: &str) -> TimeEntry {
        TimeEntry::new("1", task, title, None, None, None, false, None)
    }

    #[test]
    fn 依任務分組_不分大小寫_名稱用最後一筆() {
        let groups = group_entries(&[entry("F2P-1", "舊名"), entry("EG-2", "B"), entry("f2p-1", "新名")]);
        assert_eq!(vec![("F2P-1".to_owned(), "新名".to_owned()), ("EG-2".to_owned(), "B".to_owned())], groups);
    }

    #[test]
    fn 計時中在前_再依時間_最多8個() {
        let mut rows: Vec<(String, String, bool, f64)> =
            (0..10).map(|i| (format!("K-{i}"), String::new(), false, i as f64)).collect();
        rows[2].2 = true;
        let out = summary_rows(rows);
        assert_eq!(8, out.len());
        assert_eq!("K-2", out[0].0);
        assert_eq!("K-9", out[1].0);
        assert_eq!("K-3", out[7].0);
    }
}
