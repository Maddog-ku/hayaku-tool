//! 對應 C# `UI/PanelWindow.Pins.cs`：面板的釘選托盤（▶ 一鍵切換、Alt＋數字）、待我審核卡片、今天分頁的測試卡片。
//! （任務分頁的「只顯示沒有子任務的」在 work.rs 的 leaf-only。）

use chrono::{Local, NaiveDateTime, TimeDelta};
use hb_api::{DateTimeOffset, MeetingItem, PinItem, WorkMath};
use hb_core::ci_set::eq_ic;
use hb_ui_common::KeyCopy;
use hb_ui_common::view_models::ColorKey;
use hb_ui_work::PageArg;
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::{PanelInner, now_offset, spawn, with_panel};
use crate::menu::{self, MenuEntry, MenuPlace};
use crate::ui::PinRow;

/// 托盤上的一個釘選任務或會議；正在計時的是深色。會議的 ▶ 是加入會議（開會前 10 分鐘起），
/// 已取消、已結束的會議變灰、不能加入（C# PinRowVm）。
#[derive(Debug, Clone, PartialEq)]
pub struct PinRowVm {
    pub item: PinItem,
    pub number: usize,
    pub running: bool,
    /// 現在的時間（判斷會議能不能加入）。
    pub now: DateTimeOffset,
    pub busy: bool,
    pub elapsed: String,
}

impl PinRowVm {
    pub fn title(&self) -> String {
        if self.item.is_meeting { format!("🗓 {}", self.item.title) } else { self.item.title.clone() }
    }

    pub fn index(&self) -> String {
        if self.number <= 9 { self.number.to_string() } else { String::new() }
    }

    fn gone(&self) -> bool {
        self.item.is_meeting && (self.item.cancelled || self.item.ended(self.now))
    }

    fn meeting_time(&self) -> String {
        let local = |t: &DateTimeOffset| t.with_timezone(&Local);
        match &self.item.start_at {
            Some(s) => {
                let mut text = local(s).format("%-m/%-d %H:%M").to_string();
                if let Some(e) = &self.item.end_at {
                    text += &format!("–{}", local(e).format("%H:%M"));
                }
                text
            }
            None => String::new(),
        }
    }

    pub fn tip(&self) -> String {
        let i = &self.item;
        if i.is_meeting {
            let state = if i.cancelled {
                "\n已取消"
            } else if i.ended(self.now) {
                "\n已結束"
            } else if i.can_join(self.now) {
                "\n▶ 加入會議並計時"
            } else {
                "\n開會前 10 分鐘起可以加入"
            };
            let hotkey = if self.number <= 9 && i.can_join(self.now) {
                format!("\nAlt+{}：加入", self.number)
            } else {
                String::new()
            };
            format!("會議 {} {}\n{}{state}{hotkey}\n點一下看詳細，右鍵更多", i.key, i.title, self.meeting_time())
        } else {
            let no_time = if i.can_time { "" } else { "\n不能計時（分類任務、已完成或只能檢視）" };
            let hotkey =
                if self.number <= 9 { format!("\nAlt+{}：切換到這個任務", self.number) } else { String::new() };
            format!("{} {}\n{} · {}{no_time}{hotkey}\n點一下看詳細，右鍵更多", i.key, i.title, i.project, i.status)
        }
    }

    /// ▶ 能不能按：任務看能不能計時，會議看現在能不能加入。
    pub fn can_start(&self) -> bool {
        if self.item.is_meeting { self.item.can_join(self.now) } else { self.item.can_time }
    }

    pub fn action_color(&self) -> ColorKey {
        if self.running {
            ColorKey::Error
        } else if self.can_start() {
            ColorKey::Run
        } else {
            ColorKey::Muted
        }
    }

    pub fn action_glyph(&self) -> &'static str {
        if self.busy {
            "…"
        } else if self.running {
            "❚❚"
        } else {
            "▶"
        }
    }

    pub fn action_tip(&self) -> &'static str {
        let i = &self.item;
        if i.is_meeting {
            if self.running {
                "離開會議"
            } else if i.cancelled {
                "會議已取消"
            } else if i.ended(self.now) {
                "會議已結束"
            } else if self.can_start() {
                "加入會議並計時"
            } else {
                "開會前 10 分鐘起可以加入"
            }
        } else if self.running {
            "暫停"
        } else {
            "切換到這個任務（暫停其他的）"
        }
    }

    pub fn can_act(&self) -> bool {
        !self.busy && (self.running || self.can_start())
    }

    /// 托盤上的時間：今天的計時列表有這個任務就自己算（計時中會跳秒），沒有就用托盤給的今天累積；
    /// 還沒開始的會議顯示開始時間。
    pub fn elapsed_text(&self, minutes_today: f64, now: NaiveDateTime) -> String {
        let mut minutes = minutes_today;
        if minutes <= 0.0 {
            minutes = self.item.today_minutes as f64;
        }
        if minutes <= 0.0
            && self.item.is_meeting
            && let Some(start) = self.item.start_at
        {
            let local = start.with_timezone(&Local);
            if local.naive_local() > now {
                return if local.date_naive() == now.date() {
                    local.format("%H:%M").to_string()
                } else {
                    local.format("%-m/%-d %H:%M").to_string()
                };
            }
        }
        if minutes <= 0.0 {
            String::new()
        } else if self.running {
            WorkMath::clock(minutes)
        } else {
            WorkMath::hours_text(minutes)
        }
    }

    fn row(&self) -> PinRow {
        PinRow {
            key: self.item.key.as_str().into(),
            index: self.index().into(),
            title: self.title().into(),
            elapsed: self.elapsed.as_str().into(),
            tip: self.tip().into(),
            running: self.running,
            gone: self.gone(),
            can_act: self.can_act(),
            action_glyph: self.action_glyph().into(),
            action_tip: self.action_tip().into(),
            action_color: self.action_color().code(),
        }
    }
}

/// 托盤上的會議 → 加入、離開用的會議資料（今天的會議清單裡有就用那份）。
pub fn pinned_meeting(pin: &PinItem, meetings: &[MeetingItem], now: DateTimeOffset) -> MeetingItem {
    meetings.iter().find(|m| eq_ic(&m.key, &pin.key)).cloned().unwrap_or_else(|| MeetingItem {
        key: pin.key.clone(),
        id: pin.key.clone(),
        title: pin.title.clone(),
        project: pin.project.clone(),
        start_at: pin.start_at.unwrap_or(now),
        end_at: pin.end_at.unwrap_or(now + TimeDelta::hours(1)),
        location: None,
        my_status: None,
    })
}

/// 「待我審核 3 筆（2 個專案）」。
pub fn review_text(count: usize, projects: usize) -> String {
    let mut text = format!("待我審核 {count} 筆");
    if projects > 1 {
        text += &format!("（{projects} 個專案）");
    }
    text
}

/// 測試卡片：我要修的測試結果（🔥 標第 3 輪起的）、要測的測試任務。
pub fn test_card_text(fixes: usize, hot: usize, tests: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    if fixes > 0 {
        parts.push(format!("🐞 要修 {fixes}") + &if hot > 0 { format!("（🔥{hot}）") } else { String::new() });
    }
    if tests > 0 {
        parts.push(format!("🧪 要測 {tests}"));
    }
    parts.join("　")
}

impl PanelInner {
    pub(super) fn render_pins(&self) {
        let running = self.engine.running_keys();
        let now = now_offset();
        let pins: Vec<PinRowVm> = self
            .engine
            .pins()
            .iter()
            .enumerate()
            .map(|(i, p)| PinRowVm {
                item: p.clone(),
                number: i + 1,
                running: running.contains(&p.key),
                now,
                busy: false,
                elapsed: String::new(),
            })
            .collect();
        self.ui.set_pins(ModelRc::new(VecModel::from(pins.iter().map(PinRowVm::row).collect::<Vec<_>>())));
        self.s.borrow_mut().pins = pins;
        self.update_pin_elapsed(super::now());
    }

    pub(super) fn update_pin_elapsed(&self, now: NaiveDateTime) {
        let entries = self.engine.entries();
        let day = now.date().and_hms_opt(0, 0, 0).unwrap_or(now);
        let model = self.ui.get_pins();
        let mut s = self.s.borrow_mut();
        for (i, vm) in s.pins.iter_mut().enumerate() {
            let minutes = WorkMath::task_minutes(entries.iter(), &vm.item.key, day, now);
            let elapsed = vm.elapsed_text(minutes, now);
            if vm.elapsed != elapsed {
                vm.elapsed = elapsed;
                if i < model.row_count() {
                    model.set_row_data(i, vm.row());
                }
            }
        }
    }

    /// 托盤、待我審核、測試卡片的回呼（Panel::new 時接）。
    pub(super) fn wire_pins(&self) {
        let ui = &self.ui;
        ui.on_pin_act(with_panel!(self.me, |p, i| {
            let vm = p.s.borrow().pins.get(i as usize).cloned();
            if let Some(vm) = vm {
                p.run_pin(vm, false);
            }
        }));
        ui.on_pin_open(with_panel!(self.me, |p, i| {
            let vm = p.s.borrow().pins.get(i as usize).cloned();
            if let Some(vm) = vm {
                let page = if vm.item.is_meeting { "meetings" } else { "tasks" };
                p.request_workspace(page, Some(PageArg::Text(vm.item.key)));
            }
        }));
        ui.on_pin_menu(with_panel!(self.me, |p, i, x, y| p.pin_menu(i as usize, x, y)));
        ui.on_review_clicked(with_panel!(self.me, |p| p.request_workspace("reviews", None)));
        ui.on_test_clicked(with_panel!(self.me, |p| {
            let target = p.s.borrow().test_card_target;
            p.request_workspace("tests", Some(PageArg::Text(target.into())));
        }));
    }

    fn set_pin_busy(&self, key: &str, busy: bool) {
        let model = self.ui.get_pins();
        let mut s = self.s.borrow_mut();
        if let Some((i, vm)) = s.pins.iter_mut().enumerate().find(|(_, v)| v.item.key == key) {
            vm.busy = busy;
            if i < model.row_count() {
                model.set_row_data(i, vm.row());
            }
        }
    }

    fn run_pin(&self, vm: PinRowVm, keep_others: bool) {
        self.set_pin_busy(&vm.item.key, true);
        let me = self.weak();
        let engine = self.engine.clone();
        spawn(async move {
            if vm.item.is_meeting {
                let meeting = pinned_meeting(&vm.item, &engine.meetings(), now_offset());
                if vm.running {
                    engine.leave_meeting(&meeting).await;
                } else if vm.item.can_join(now_offset()) {
                    engine.join_meeting(&meeting).await;
                }
            } else if vm.running && !keep_others {
                engine.pause_task(&vm.item.key).await;
            } else {
                engine.switch_task(&vm.item.key, keep_others).await;
            }
            if let Some(p) = me.upgrade() {
                p.set_pin_busy(&vm.item.key, false);
            }
        });
    }

    fn pin_menu(&self, index: usize, x: f32, y: f32) {
        let Some(vm) = self.s.borrow().pins.get(index).cloned() else { return };
        let mut entries: Vec<MenuEntry> = Vec::new();
        // 每一項要做的事（跟 entries 對齊）
        #[derive(Clone, Copy)]
        enum Act {
            Run,
            RunKeep,
            Open,
            Web,
            CopyLink,
            Unpin,
            None,
        }
        let mut acts: Vec<Act> = Vec::new();
        let mut add = |entry: MenuEntry, act: Act| {
            entries.push(entry);
            acts.push(act);
        };
        if vm.item.is_meeting {
            if vm.running {
                add(MenuEntry::item("⏏  離開會議"), Act::Run);
            } else {
                add(MenuEntry::item_if("▶  加入會議並計時", vm.can_start()), Act::Run);
            }
            add(MenuEntry::item("☰  查看詳細"), Act::Open);
            add(MenuEntry::item("↗  在網頁開啟"), Act::Web);
        } else {
            if vm.running {
                add(MenuEntry::item("❚❚  暫停"), Act::Run);
            } else {
                add(MenuEntry::item_if("▶  切換到這個（暫停其他的）", vm.item.can_time), Act::Run);
                add(MenuEntry::item_if("▶  同時計時（不暫停其他的）", vm.item.can_time), Act::RunKeep);
            }
            add(MenuEntry::item("☰  查看詳細"), Act::Open);
            add(MenuEntry::item("🔗  複製連結"), Act::CopyLink);
            add(MenuEntry::item("↗  在網頁開啟"), Act::Web);
        }
        add(MenuEntry::Separator, Act::None);
        add(MenuEntry::item("📌  取消釘選"), Act::Unpin);
        let anchor = menu::to_screen(self.ui.window(), x, y, 1.0, 1.0);
        let me = self.weak();
        menu::show_later(self.ui.as_weak(), entries, MenuPlace::RightOf(anchor), move |i| {
            let Some(p) = me.upgrade() else { return };
            let key = vm.item.key.clone();
            match acts.get(i).copied().unwrap_or(Act::None) {
                Act::Run => p.run_pin(vm, false),
                Act::RunKeep => p.run_pin(vm, true),
                Act::Open => {
                    let page = if vm.item.is_meeting { "meetings" } else { "tasks" };
                    p.request_workspace(page, Some(PageArg::Text(key)));
                }
                Act::Web => {
                    let url = if vm.item.is_meeting {
                        p.engine.meeting_url(&pinned_meeting(&vm.item, &p.engine.meetings(), now_offset()).id)
                    } else {
                        p.engine.task_url(&key)
                    };
                    hb_win::open(&url);
                }
                Act::CopyLink => KeyCopy::copy_link(&p.engine, &key),
                Act::Unpin => {
                    let engine = p.engine.clone();
                    spawn(async move { engine.unpin_task(&key).await });
                }
                Act::None => {}
            }
        });
    }

    /// Alt＋1～9：切換到托盤上第幾個任務（跟網頁一樣）。
    pub(super) fn handle_pin_hotkey(&self, e: &slint::private_unstable_api::re_exports::KeyEvent) -> bool {
        let m = &e.modifiers;
        if !m.alt || m.control || m.shift || m.meta {
            return false;
        }
        let Some(d) = e.text.chars().next().and_then(|c| c.to_digit(10)) else { return false };
        if !(1..=9).contains(&d) {
            return false;
        }
        let index = d as usize - 1;
        let vm = self.s.borrow().pins.get(index).cloned();
        let Some(vm) = vm else { return false };
        if !vm.running && vm.can_start() {
            self.run_pin(vm, false);
        }
        true
    }

    // ───────────── 待我審核、測試卡片 ─────────────

    pub(super) fn render_reviews(&self) {
        let pending = self.engine.pending_reviews();
        let mut projects: Vec<&str> = pending.iter().map(|c| c.project.as_str()).collect();
        projects.sort_unstable();
        projects.dedup();
        self.ui.set_review_text(
            if pending.is_empty() { String::new() } else { review_text(pending.len(), projects.len()) }.into(),
        );
        self.render_test_card();
    }

    fn render_test_card(&self) {
        let fixes = self.engine.my_fixes();
        let tests = self.engine.my_tests();
        let hot = fixes.iter().filter(|t| t.test.hot()).count();
        self.ui.set_test_text(test_card_text(fixes.len(), hot, tests.len()).into());
        self.s.borrow_mut().test_card_target = if fixes.is_empty() { "test" } else { "fix" };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{FixedOffset, NaiveDate, TimeZone};

    fn at(h: u32, m: u32) -> DateTimeOffset {
        FixedOffset::east_opt(8 * 3600).unwrap().with_ymd_and_hms(2026, 11, 12, h, m, 0).unwrap()
    }

    fn task_pin(can_time: bool) -> PinItem {
        PinItem {
            key: "F2P-5".into(),
            title: "列印".into(),
            project: "F2P".into(),
            status: "進行中".into(),
            today_minutes: 90,
            running: false,
            can_time,
            is_meeting: false,
            start_at: None,
            end_at: None,
            cancelled: false,
        }
    }

    fn meeting_pin(start: DateTimeOffset, end: DateTimeOffset) -> PinItem {
        PinItem {
            key: "F2P-M1".into(),
            title: "週會".into(),
            is_meeting: true,
            start_at: Some(start),
            end_at: Some(end),
            can_time: false,
            ..task_pin(false)
        }
    }

    fn vm(item: PinItem, number: usize, running: bool, now: DateTimeOffset) -> PinRowVm {
        PinRowVm { item, number, running, now, busy: false, elapsed: String::new() }
    }

    #[test]
    fn 任務_按鈕與提示() {
        let v = vm(task_pin(true), 2, false, at(10, 0));
        assert_eq!("2", v.index());
        assert_eq!("切換到這個任務（暫停其他的）", v.action_tip());
        assert_eq!(ColorKey::Run, v.action_color());
        assert!(v.can_act());
        assert_eq!("F2P-5 列印\nF2P · 進行中\nAlt+2：切換到這個任務\n點一下看詳細，右鍵更多", v.tip());
        let v = vm(task_pin(false), 10, false, at(10, 0));
        assert_eq!("", v.index());
        assert!(!v.can_act());
        assert_eq!(ColorKey::Muted, v.action_color());
        assert_eq!("F2P-5 列印\nF2P · 進行中\n不能計時（分類任務、已完成或只能檢視）\n點一下看詳細，右鍵更多", v.tip());
        let v = vm(task_pin(true), 1, true, at(10, 0));
        assert_eq!(("❚❚", "暫停", ColorKey::Error), (v.action_glyph(), v.action_tip(), v.action_color()));
    }

    #[test]
    fn 會議_開會前10分鐘起才能加入() {
        let pin = meeting_pin(at(14, 0), at(15, 0));
        let early = vm(pin.clone(), 1, false, at(13, 45));
        assert_eq!("🗓 週會", early.title());
        assert!(!early.can_start());
        assert_eq!("開會前 10 分鐘起可以加入", early.action_tip());
        let ok = vm(pin.clone(), 1, false, at(13, 50));
        assert!(ok.can_start());
        assert_eq!("加入會議並計時", ok.action_tip());
        assert!(ok.tip().ends_with("\n▶ 加入會議並計時\nAlt+1：加入\n點一下看詳細，右鍵更多"), "{}", ok.tip());
        let ended = vm(pin.clone(), 1, false, at(15, 0));
        assert_eq!("會議已結束", ended.action_tip());
        let mut cancelled = pin;
        cancelled.cancelled = true;
        assert_eq!("會議已取消", vm(cancelled, 1, false, at(14, 0)).action_tip());
    }

    #[test]
    fn 托盤時間() {
        let now = NaiveDate::from_ymd_opt(2026, 11, 12).unwrap().and_hms_opt(10, 0, 0).unwrap();
        let v = vm(task_pin(true), 1, false, at(10, 0));
        // 今天的計時列表有就自己算，沒有就用托盤給的今天累積（90 分）
        assert_eq!("0.5h", v.elapsed_text(30.0, now));
        assert_eq!("1.5h", v.elapsed_text(0.0, now));
        let running = vm(task_pin(true), 1, true, at(10, 0));
        assert_eq!("0:30:00", running.elapsed_text(30.0, now));
        // 還沒開始的會議顯示開始時間（今天只顯示時間）
        let local = |h| {
            chrono::Local
                .from_local_datetime(&NaiveDate::from_ymd_opt(2026, 11, 12).unwrap().and_hms_opt(h, 0, 0).unwrap())
                .unwrap()
                .fixed_offset()
        };
        let mut m = meeting_pin(local(14), local(15));
        m.today_minutes = 0;
        assert_eq!("14:00", vm(m.clone(), 1, false, at(10, 0)).elapsed_text(0.0, now));
        let tomorrow = chrono::Local
            .from_local_datetime(&NaiveDate::from_ymd_opt(2026, 11, 13).unwrap().and_hms_opt(9, 30, 0).unwrap())
            .unwrap()
            .fixed_offset();
        m.start_at = Some(tomorrow);
        assert_eq!("11/13 09:30", vm(m, 1, false, at(10, 0)).elapsed_text(0.0, now));
    }

    #[test]
    fn 托盤會議_用今天的會議資料() {
        let pin = meeting_pin(at(14, 0), at(15, 0));
        let fallback = pinned_meeting(&pin, &[], at(10, 0));
        assert_eq!(("F2P-M1", "F2P-M1", at(14, 0)), (fallback.key.as_str(), fallback.id.as_str(), fallback.start_at));
        let today = MeetingItem { key: "f2p-m1".into(), id: "abc".into(), ..fallback.clone() };
        assert_eq!("abc", pinned_meeting(&pin, std::slice::from_ref(&today), at(10, 0)).id);
        // 沒有時間的會議：現在到一小時後
        let mut no_time = pin;
        no_time.start_at = None;
        no_time.end_at = None;
        let m = pinned_meeting(&no_time, &[], at(10, 0));
        assert_eq!((at(10, 0), at(11, 0)), (m.start_at, m.end_at));
    }

    #[test]
    fn 審核與測試卡片文字() {
        assert_eq!("待我審核 3 筆", review_text(3, 1));
        assert_eq!("待我審核 3 筆（2 個專案）", review_text(3, 2));
        assert_eq!("🐞 要修 2（🔥1）　🧪 要測 4", test_card_text(2, 1, 4));
        assert_eq!("🐞 要修 2", test_card_text(2, 0, 0));
        assert_eq!("🧪 要測 1", test_card_text(0, 0, 1));
        assert_eq!("", test_card_text(0, 0, 0));
    }
}
