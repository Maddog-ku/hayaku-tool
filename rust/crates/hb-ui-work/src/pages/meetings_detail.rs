//! 會議詳細（對應 C# `UI/Work/MeetingsPage.Detail.cs`）：右欄的時間、地點、主辦、重複規則、參加者與各自記到的工時；
//! 可以修改（名稱、時間、地點、參加者、會議紀錄）。會議紀錄、檔案、白板放在可展開的區塊，展開才讀，平常不多打請求。

use std::rc::{Rc, Weak};

use chrono::Local;
use hb_api::{MeetingDetail, WorkMath};
use hb_ui_common::file_preview;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use serde_json::{Map, Value, json};
use slint::{ComponentHandle, Model};

use super::w3_common::{model, spawn, status_parts};
use super::{Page, attendee_changes, field_text, file_size, set_check};
use crate::{MeetAttendee, MeetBlock, MeetInfo, MeetPreview, MeetSection, MeetingsPageData, W3Check};

/// 可展開區塊的標題（順序就是畫面上的順序）。
const SECTIONS: [&str; 4] = ["會議紀錄", "紀錄更新歷程", "檔案", "白板"];

macro_rules! bind {
    // 沒有參數的回呼：rustfmt 會把 `| |` 排成 `||`，兩種都接受
    ($g:ident . $on:ident, $page:ident, || $body:expr) => {
        bind!($g.$on, $page, | | $body)
    };
    ($g:ident . $on:ident, $page:ident, | $($arg:ident),* | $body:expr) => {{
        let weak: Weak<Page> = Rc::downgrade($page);
        $g.$on(move |$($arg),*| {
            if let Some($page) = weak.upgrade() {
                $body
            }
        });
    }};
}

pub(super) fn bind(page: &Rc<Page>, g: &MeetingsPageData<'_>) {
    bind!(g.on_card_clicked, page, |i| {
        let key = page.st.borrow().items.get(i as usize).map(|m| m.key.clone());
        if let Some(key) = key {
            spawn(async move { page.open_detail(key).await });
        }
    });
    bind!(g.on_close_detail, page, || page.close_detail());
    bind!(g.on_detail_web, page, || {
        let id = page.st.borrow().detail.as_ref().map(|d| d.id.clone());
        if let Some(id) = id {
            hb_win::open(&page.ctx.engine.meeting_url(&id));
        }
    });
    bind!(g.on_pin_detail, page, || page.pin_current());
    bind!(g.on_key_p, page, || page.pin_current());
    bind!(g.on_edit_clicked, page, || spawn(async move { page.build_edit_form().await }));
    bind!(g.on_edit_cancel, page, || {
        page.with(|g| g.set_edit_open(false));
    });
    bind!(g.on_edit_person_toggled, page, |i, c| {
        page.with(|g| set_check(&g.get_edit_people(), i, c));
    });
    bind!(g.on_edit_save, page, || spawn(async move { page.save_edit().await }));
    bind!(g.on_section_toggled, page, |i, open| page.section_toggled(i as usize, open));
    bind!(g.on_block_clicked, page, |s, b| page.block_clicked(s as usize, b as usize));
    bind!(g.on_preview, page, |id, name| {
        let w = page.ctx.window();
        file_preview::show(&page.ctx.engine, &id, &name, w.as_ref().map(|w| w.window()), None);
    });
}

impl Page {
    fn show_detail_status(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_detail_status(text);
            g.set_detail_tone(tone);
        });
    }

    pub(super) async fn open_detail(self: Rc<Self>, key: String) {
        let version = {
            let mut st = self.st.borrow_mut();
            st.detail_version += 1;
            st.detail_key = Some(key.clone());
            st.detail_version
        };
        self.refresh_rows();
        self.with(|g| {
            g.set_detail_open(true);
            g.set_detail_loaded(false);
            g.set_detail_title("".into());
            g.set_detail_message("讀取中…".into());
            g.set_detail_error(false);
            g.set_edit_open(false);
        });
        let result = self.ctx.engine.api().get_meeting(&key).await;
        if version != self.st.borrow().detail_version {
            return;
        }
        match result {
            Ok(d) => {
                self.render_detail(&d);
                self.st.borrow_mut().detail = Some(d);
            }
            Err(e) => {
                log::error!("讀取會議 {key} 失敗: {e}");
                self.with(|g| {
                    g.set_detail_title(key.as_str().into());
                    g.set_detail_message(format!("讀不到這場會議：{e}").into());
                    g.set_detail_error(true);
                });
            }
        }
    }

    fn close_detail(&self) {
        {
            let mut st = self.st.borrow_mut();
            st.detail_version += 1;
            st.detail_key = None;
            st.detail = None;
        }
        self.with(|g| {
            g.set_detail_open(false);
            g.set_edit_open(false);
        });
        self.refresh_rows();
    }

    fn render_detail(&self, d: &MeetingDetail) {
        let engine = &self.ctx.engine;
        let start = d.start_at.with_timezone(&Local);
        let end = d.end_at.with_timezone(&Local);
        let mut lines = vec![
            (
                "時間",
                format!(
                    "{}　{}–{}",
                    inputs::short_date(start.date_naive()),
                    start.format("%H:%M"),
                    end.format("%H:%M")
                ),
            ),
            ("專案", engine.project_of(&d.project).map(|p| p.name).unwrap_or_else(|| d.project.clone())),
        ];
        if let Some(l) = d.location.as_ref().filter(|l| !l.trim().is_empty()) {
            lines.push(("地點", l.clone()));
        }
        if let Some(o) = &d.organizer {
            lines.push(("主辦", o.clone()));
        }
        if let Some(r) = &d.recurrence {
            lines.push(("重複", r.clone()));
        }
        if let Some(me) = &d.my_state {
            lines.push(("我", me.clone()));
        }
        let info: Vec<MeetInfo> =
            lines.into_iter().map(|(l, v)| MeetInfo { label: l.into(), value: v.into() }).collect();
        let people: Vec<MeetAttendee> = d
            .attendees
            .iter()
            .map(|a| MeetAttendee {
                name: a.name.as_str().into(),
                state: a.state.as_str().into(),
                state_accent: a.state == "會議中",
                logged: if a.logged_minutes > 0 {
                    WorkMath::hours_minutes(a.logged_minutes as f64).into()
                } else {
                    "".into()
                },
            })
            .collect();
        let sections: Vec<MeetSection> = SECTIONS
            .iter()
            .map(|t| MeetSection { title: (*t).into(), open: false, blocks: model(Vec::new()) })
            .collect();
        self.st.borrow_mut().section_loaded = [false; 4];
        self.with(|g| {
            g.set_scroll_top(g.get_scroll_top() + 1);
            g.set_detail_title(d.title.as_str().into());
            g.set_detail_message("".into());
            g.set_detail_error(false);
            g.set_info(model(info));
            g.set_can_edit(d.can_edit);
            g.set_people(model(people));
            g.set_sections(model(sections));
            g.set_edit_open(false);
            g.set_detail_loaded(true);
        });
        self.show_detail_status(None);
        self.render_pin(&d.key);
    }

    fn render_pin(&self, key: &str) {
        let pinned = self.ctx.engine.is_pinned(key);
        let text = if pinned { "📌 取消釘選" } else { "📌 釘到托盤" };
        let tip = format!(
            "{}（P）",
            if pinned {
                "從面板的托盤拿掉"
            } else {
                "釘到面板的托盤，開會前 10 分鐘起按 ▶ 就加入"
            }
        );
        self.with(|g| {
            g.set_pin_text(text.into());
            g.set_pin_tip(tip.into());
        });
    }

    /// 目前詳細的會議釘選／取消（〔📌〕按鈕與 P 鍵）。
    fn pin_current(self: &Rc<Self>) {
        let key = self.st.borrow().detail.as_ref().map(|d| d.key.clone());
        if let Some(key) = key {
            let page = self.clone();
            spawn(async move { page.toggle_pin(key, true).await });
        }
    }

    /// 會議釘到托盤或取消（托盤上 ▶ 是加入會議）；detail：訊息顯示在詳細，不然顯示在清單上方。
    pub(super) async fn toggle_pin(self: Rc<Self>, key: String, detail: bool) {
        let engine = &self.ctx.engine;
        let pinned = engine.is_pinned(&key);
        if pinned {
            engine.unpin_task(&key).await;
        } else {
            engine.pin_task(&key).await;
        }
        let message = match engine.error() {
            Some(e) => StatusMessage::error(e),
            None => StatusMessage::ok(if pinned {
                "已取消釘選"
            } else {
                "已釘到托盤（面板的計時分頁最上面，開會前 10 分鐘起按 ▶ 加入）"
            }),
        };
        if detail {
            self.show_detail_status(Some(message));
            if self.st.borrow().detail.as_ref().is_some_and(|d| d.key == key) {
                self.render_pin(&key);
            }
        } else {
            self.show_list(Some(message));
        }
        self.refresh_rows();
    }

    // ───────────── 修改 ─────────────

    async fn build_edit_form(self: Rc<Self>) {
        if self.with(|g| g.get_edit_open()).unwrap_or(true) {
            return;
        }
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let version = self.st.borrow().detail_version;
        let start = d.start_at.with_timezone(&Local);
        let end = d.end_at.with_timezone(&Local);
        self.st.borrow_mut().edit_users.clear();
        self.with(|g| {
            g.set_edit_title(d.title.as_str().into());
            g.set_edit_date(start.format("%-m/%-d").to_string().into());
            g.set_edit_from(start.format("%H:%M").to_string().into());
            g.set_edit_to(end.format("%H:%M").to_string().into());
            g.set_edit_location(d.location.clone().unwrap_or_default().into());
            g.set_edit_note(d.note.clone().unwrap_or_default().into());
            g.set_edit_people(model(Vec::new()));
            g.set_edit_people_message("讀取專案成員…".into());
            g.set_edit_people_error(false);
            g.set_edit_recurring(d.recurring);
            g.set_edit_following(false);
            g.set_edit_open(true);
        });
        if !d.can_edit {
            return;
        }
        match self.ctx.engine.get_project(&d.project).await {
            Ok(project) => {
                if version != self.st.borrow().detail_version {
                    return;
                }
                let current: Vec<String> = d.attendees.iter().map(|a| a.username.to_lowercase()).collect();
                // 專案成員＋目前的參加者（可能已經不在專案裡）
                let mut people: Vec<(String, String)> = Vec::new();
                let candidates = project.members.iter().map(|m| (m.username.clone(), m.display_name.clone())).chain(
                    d.attendees.iter().filter(|a| !a.username.is_empty()).map(|a| (a.username.clone(), a.name.clone())),
                );
                for (u, n) in candidates {
                    if !people.iter().any(|(p, _)| p.to_lowercase() == u.to_lowercase()) {
                        people.push((u, n));
                    }
                }
                let checks: Vec<W3Check> = people
                    .iter()
                    .map(|(u, n)| W3Check {
                        id: u.as_str().into(),
                        text: n.as_str().into(),
                        checked: current.contains(&u.to_lowercase()),
                    })
                    .collect();
                self.st.borrow_mut().edit_users = people.into_iter().map(|(u, _)| u).collect();
                self.with(|g| {
                    g.set_edit_people(model(checks));
                    g.set_edit_people_message("".into());
                });
            }
            Err(e) => {
                log::error!("讀取 {} 的成員失敗: {}", d.project, e.message());
                if version != self.st.borrow().detail_version {
                    return;
                }
                self.with(|g| {
                    g.set_edit_people_message(format!("讀不到專案成員（參加者先不改）：{}", e.message()).into());
                    g.set_edit_people_error(true);
                });
            }
        }
    }

    async fn save_edit(self: Rc<Self>) {
        let Some(d) = self.st.borrow().detail.clone() else { return };
        let Some((title, date, from, to, location, note, checked, following)) = self.with(|g| {
            (
                g.get_edit_title().trim().to_owned(),
                g.get_edit_date().to_string(),
                g.get_edit_from().to_string(),
                g.get_edit_to().to_string(),
                g.get_edit_location().trim().to_owned(),
                g.get_edit_note().to_string(),
                g.get_edit_people().iter().filter(|c| c.checked).map(|c| c.id.to_string()).collect::<Vec<_>>(),
                g.get_edit_following(),
            )
        }) else {
            return;
        };
        let loaded = !self.st.borrow().edit_users.is_empty();
        let mut fields = Map::new();
        if d.can_edit {
            if !title.is_empty() && title != d.title {
                fields.insert("title".into(), json!(title));
            } else if title.is_empty() {
                self.show_detail_status(Some(StatusMessage::error("名稱不能空白")));
                return;
            }
            let Some(day) = inputs::parse_date(Some(&date), None) else {
                self.show_detail_status(Some(StatusMessage::error("日期看不懂，請用 今天、明天、11/5 這類寫法")));
                return;
            };
            let (Some(s), Some(e)) = (inputs::parse_time(Some(&from)), inputs::parse_time(Some(&to))) else {
                self.show_detail_status(Some(StatusMessage::error("開始、結束請用 HH:mm，例 14:00")));
                return;
            };
            if e <= s {
                self.show_detail_status(Some(StatusMessage::error("結束要晚於開始（會議要在同一天）")));
                return;
            }
            let new_start = inputs::local(day, s);
            let new_end = inputs::local(day, e);
            if new_start != d.start_at || new_end != d.end_at {
                fields.insert("startAt".into(), json!(inputs::iso(&new_start)));
                fields.insert("endAt".into(), json!(inputs::iso(&new_end)));
            }
            if location != d.location.clone().unwrap_or_default() {
                fields.insert("location".into(), if location.is_empty() { Value::Null } else { json!(location) });
            }
            let (add, remove) = attendee_changes(
                d.attendees.iter().map(|a| a.username.as_str()),
                checked.iter().map(String::as_str),
                loaded,
            );
            if !add.is_empty() {
                fields.insert("addAttendees".into(), json!(add));
            }
            if !remove.is_empty() {
                fields.insert("removeAttendees".into(), json!(remove));
            }
        }
        if note != d.note.clone().unwrap_or_default() {
            fields.insert("note".into(), if note.is_empty() { Value::Null } else { json!(note) });
        }
        if fields.is_empty() {
            self.show_detail_status(Some(StatusMessage::error("沒有修改")));
            return;
        }
        if d.recurring && d.can_edit {
            fields.insert("scope".into(), json!(if following { "following" } else { "this" }));
        }
        let new_title = field_text(&fields, "title").unwrap_or_else(|| d.title.clone());
        let api = self.ctx.engine.api();
        let key = d.key.clone();
        let p = self.clone();
        let ok = inputs::run(
            |b| {
                p.with(|g| g.set_saving(b));
            },
            |m| p.show_detail_status(m),
            || async move { api.update_meeting(&key, fields).await.map_err(ActionError::from) },
            None,
        )
        .await;
        if !ok {
            return;
        }
        self.show_list(Some(StatusMessage::ok(format!("已修改會議「{new_title}」"))));
        self.ctx.engine.after_external_change().await;
        self.clone().load().await;
        self.open_detail(d.key).await;
    }

    // ───────────── 會議紀錄、檔案、白板（展開才讀） ─────────────

    fn set_section(&self, i: usize, open: Option<bool>, blocks: Option<Vec<MeetBlock>>) {
        self.with(|g| {
            let sections = g.get_sections();
            if let Some(mut s) = sections.row_data(i) {
                if let Some(open) = open {
                    s.open = open;
                }
                if let Some(blocks) = blocks {
                    s.blocks = model(blocks);
                }
                sections.set_row_data(i, s);
            }
        });
    }

    /// 可展開的區塊：第一次展開才讀，讀失敗下次展開再試。
    fn section_toggled(self: &Rc<Self>, i: usize, open: bool) {
        if i >= SECTIONS.len() {
            return;
        }
        self.set_section(i, Some(open), None);
        let (key, version) = {
            let mut st = self.st.borrow_mut();
            let Some(key) = st.detail.as_ref().map(|d| d.key.clone()) else { return };
            if !open || st.section_loaded[i] {
                return;
            }
            st.section_loaded[i] = true;
            (key, st.detail_version)
        };
        self.set_section(i, None, Some(vec![block(0, "讀取中…")]));
        let page = self.clone();
        spawn(async move {
            let result = page.load_section(i, &key).await;
            if version != page.st.borrow().detail_version {
                return;
            }
            match result {
                Ok(blocks) => page.set_section(i, None, Some(blocks)),
                Err(e) => {
                    log::error!("讀取會議 {key} 的{}失敗: {e}", SECTIONS[i]);
                    page.st.borrow_mut().section_loaded[i] = false;
                    page.set_section(i, None, Some(vec![block(7, &format!("讀取失敗：{e}"))]));
                }
            }
        });
    }

    async fn load_section(&self, i: usize, key: &str) -> Result<Vec<MeetBlock>, hb_api::McpError> {
        let api = self.ctx.engine.api();
        Ok(match i {
            0 => notes_blocks(&api.meeting_notes(key).await?),
            1 => history_blocks(&api.meeting_note_history(key, 10).await?),
            2 => files_blocks(&api.meeting_files(key).await?),
            _ => whiteboard_blocks(&api.meeting_whiteboards(key).await?),
        })
    }

    /// 檔案：能讀（有 id）就直接看內容，不然在網頁開啟這場會議。
    fn block_clicked(&self, s: usize, b: usize) {
        let Some(blk) = self.with(|g| g.get_sections().row_data(s).and_then(|sec| sec.blocks.row_data(b))).flatten()
        else {
            return;
        };
        if blk.kind != 3 {
            return;
        }
        let engine = &self.ctx.engine;
        let id = {
            let st = self.st.borrow();
            st.detail.as_ref().map(|d| d.id.clone()).or_else(|| st.detail_key.clone()).unwrap_or_default()
        };
        let url = engine.meeting_url(&id);
        if blk.id.is_empty() {
            hb_win::open(&url);
        } else {
            let w = self.ctx.window();
            file_preview::show(engine, &blk.id, &blk.text, w.as_ref().map(|w| w.window()), Some(&url));
        }
    }
}

fn block(kind: i32, text: &str) -> MeetBlock {
    MeetBlock {
        kind,
        text: text.into(),
        meta: "".into(),
        id: "".into(),
        tip: "".into(),
        max_h: 0.0,
        buttons: model(Vec::new()),
    }
}

fn boxed(text: &str, max_h: f32) -> MeetBlock {
    MeetBlock { max_h, ..block(2, text) }
}

fn non_blank(parts: &[Option<String>], sep: &str) -> String {
    parts.iter().flatten().filter(|s| !s.trim().is_empty()).cloned().collect::<Vec<_>>().join(sep)
}

pub(crate) fn notes_blocks(notes: &[hb_api::MeetingNote]) -> Vec<MeetBlock> {
    let mut out = Vec::new();
    if notes.is_empty() {
        out.push(block(0, "還沒有會議紀錄"));
    }
    for n in notes {
        let head = match &n.updated_at {
            Some(at) => format!("{}　{}更新", n.title, inputs::ago(at)),
            None => n.title.clone(),
        };
        out.push(block(1, &head));
        let md = n.markdown.trim();
        out.push(if md.is_empty() { block(0, "（空白）") } else { boxed(md, 360.0) });
    }
    out
}

/// 會議紀錄最近 10 版：誰、何時、摘要，差異（＋新增、－刪除、～改了）放在底下。
pub(crate) fn history_blocks(versions: &[hb_api::MeetingNoteVersion]) -> Vec<MeetBlock> {
    let mut out = Vec::new();
    if versions.is_empty() {
        out.push(block(0, "還沒有人改過會議紀錄"));
    }
    for v in versions {
        let head = non_blank(&[Some(v.editor.clone()), v.at.as_ref().map(inputs::ago), Some(v.summary.clone())], "　");
        out.push(block(1, &head));
        if !v.diff.trim().is_empty() {
            out.push(boxed(v.diff.trim(), 220.0));
        }
    }
    out
}

pub(crate) fn files_blocks(files: &[hb_api::MeetingFile]) -> Vec<MeetBlock> {
    let mut out = Vec::new();
    if files.is_empty() {
        out.push(block(0, "沒有檔案"));
    }
    for f in files {
        let meta = non_blank(
            &[
                Some(file_size(f.size_bytes)),
                Some(f.source_text().to_owned()),
                f.uploaded_by.clone(),
                f.uploaded_at.as_ref().map(inputs::ago),
            ],
            " · ",
        );
        let can_read = !f.id.is_empty();
        out.push(MeetBlock {
            meta: meta.into(),
            id: f.id.as_str().into(),
            tip: if can_read {
                "點一下直接看內容（圖片、PDF、Word、Excel、文字檔）"
            } else {
                "在網頁開啟這場會議，從「檔案」下載"
            }
            .into(),
            ..block(3, &f.file_name)
        });
    }
    out
}

/// 一列最多幾個白板按鈕（C# 是自動換行的 WrapPanel）。
const PREVIEWS_PER_ROW: usize = 5;

pub(crate) fn whiteboard_blocks(boards: &[hb_api::MeetingWhiteboard]) -> Vec<MeetBlock> {
    let mut out = Vec::new();
    if boards.is_empty() {
        out.push(block(0, "沒有白板"));
    }
    for b in boards {
        let mut meta = Vec::new();
        if b.element_count > 0 {
            meta.push(format!("{} 個元素", b.element_count));
        }
        if b.image_count > 0 {
            meta.push(format!("{} 張圖片", b.image_count));
        }
        if b.snapshots > 0 {
            meta.push(format!("{} 個快照", b.snapshots));
        }
        if let Some(at) = &b.updated_at {
            meta.push(format!("{}更新", inputs::ago(at)));
        }
        out.push(block(1, &b.title));
        out.push(block(4, &meta.join(" · ")));
        // 白板的縮圖、白板上的圖片：直接看圖（read_file）
        let mut previews = Vec::new();
        if let Some(id) = &b.id {
            previews.push(MeetPreview {
                text: "🖼 看白板".into(),
                id: id.as_str().into(),
                name: format!("白板：{}", b.title).into(),
            });
        }
        for (n, image) in b.image_ids.iter().flatten().enumerate() {
            let n = n + 1;
            previews.push(MeetPreview {
                text: format!("圖片 {n}").into(),
                id: image.as_str().into(),
                name: format!("{} 圖片 {n}", b.title).into(),
            });
        }
        for chunk in previews.chunks(PREVIEWS_PER_ROW) {
            out.push(MeetBlock { buttons: model(chunk.to_vec()), ..block(5, "") });
        }
        if b.text.is_empty() {
            out.push(block(
                0,
                if b.id.is_none() {
                    "（沒有文字，只有手繪或圖形；要看圖請在網頁開啟）"
                } else {
                    "（沒有文字，只有手繪或圖形；按「看白板」看圖）"
                },
            ));
        } else {
            out.push(MeetBlock { max_h: 240.0, ..block(6, &b.text.join("\n")) });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hb_api::{MeetingFile, MeetingWhiteboard};

    #[test]
    fn whiteboard_blocks_按鈕與文字() {
        let boards = vec![
            MeetingWhiteboard {
                title: "架構".into(),
                updated_at: None,
                text: vec!["A".into(), "B".into()],
                element_count: 3,
                image_count: 1,
                snapshots: 0,
                id: Some("wb1".into()),
                image_ids: Some(vec!["img1".into()]),
            },
            MeetingWhiteboard {
                title: "空白".into(),
                updated_at: None,
                text: vec![],
                element_count: 0,
                image_count: 0,
                snapshots: 2,
                id: None,
                image_ids: None,
            },
        ];
        let blocks = whiteboard_blocks(&boards);
        assert_eq!("架構", blocks[0].text.as_str());
        assert_eq!("3 個元素 · 1 張圖片", blocks[1].text.as_str());
        assert_eq!(5, blocks[2].kind);
        let buttons: Vec<_> = blocks[2].buttons.iter().map(|b| (b.text.to_string(), b.name.to_string())).collect();
        assert_eq!(
            vec![("🖼 看白板".to_owned(), "白板：架構".to_owned()), ("圖片 1".to_owned(), "架構 圖片 1".to_owned())],
            buttons
        );
        assert_eq!("A\nB", blocks[3].text.as_str());
        assert_eq!("2 個快照", blocks[5].text.as_str());
        assert_eq!("（沒有文字，只有手繪或圖形；要看圖請在網頁開啟）", blocks[6].text.as_str());
        assert_eq!("沒有白板", whiteboard_blocks(&[])[0].text.as_str());
    }

    #[test]
    fn files_blocks_說明與提示() {
        let files = vec![MeetingFile {
            id: "".into(),
            file_name: "議程.docx".into(),
            size_bytes: 20 * 1024,
            source: "ATTACHMENT".into(),
            uploaded_by: Some("王小明".into()),
            uploaded_at: None,
        }];
        let b = &files_blocks(&files)[0];
        assert_eq!("20 KB · 會議附件 · 王小明", b.meta.as_str());
        assert_eq!("在網頁開啟這場會議，從「檔案」下載", b.tip.as_str());
    }
}
