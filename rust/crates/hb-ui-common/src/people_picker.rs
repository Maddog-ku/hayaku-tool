//! 選人小視窗（跟上 Hayaku 1.5.5 全站新的選人方式；C# 版沒有）。
//! 名字、帳號都能搜；字首圓頭像；多選時〔本專案成員〕一次加入、〔全不選〕，按〔確定〕才回傳；
//! 單選時點一下就回傳。MCP 的專案成員只有帳號與顯示名稱，網頁的照片、職稱、在線狀態、群組這裡沒有。
//! 外觀在 `ui/windows/people-picker.slint`。

use std::cell::RefCell;
use std::rc::Rc;

use hb_api::PersonRef;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::dialog;
use crate::ui::{PeoplePickerWindow, PersonRow};
use crate::window_util::{self, Placement};

/// 開選人視窗要給的東西。
pub struct PeoplePick {
    /// 視窗上方的標題，例如「選擇執行者 · F2P-12」
    pub heading: String,
    /// 標題下的說明（空字串不顯示）
    pub hint: String,
    /// 可以選的人（通常是專案成員）
    pub members: Vec<PersonRef>,
    /// 目前選了誰（帳號；不在 members 裡的也列出來，標「不在專案成員中」）
    pub selected: Vec<String>,
    pub multi: bool,
}

/// 清單裡的一個人。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Person {
    pub username: String,
    pub name: String,
    pub in_project: bool,
}

/// 頭像上的字：名字的第一個字（英文大寫）。
pub fn initial(name: &str) -> String {
    name.trim().chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// 名字或帳號有包含搜尋字（不分大小寫）。
pub fn matches(p: &Person, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    q.is_empty() || p.name.to_lowercase().contains(&q) || p.username.to_lowercase().contains(&q)
}

/// 清單：已選但不在成員裡的放最前面，再來是成員（照原本順序）；同一個帳號只列一次（不分大小寫）。
pub fn people(members: &[PersonRef], selected: &[String]) -> Vec<Person> {
    let is_member = |u: &str| members.iter().any(|m| m.username.eq_ignore_ascii_case(u));
    let mut list: Vec<Person> = Vec::new();
    for u in selected.iter().filter(|u| !u.trim().is_empty() && !is_member(u)) {
        if !list.iter().any(|p| p.username.eq_ignore_ascii_case(u)) {
            list.push(Person { username: u.clone(), name: u.clone(), in_project: false });
        }
    }
    for m in members.iter().filter(|m| !m.username.is_empty()) {
        if !list.iter().any(|p| p.username.eq_ignore_ascii_case(&m.username)) {
            let name = if m.display_name.trim().is_empty() { m.username.clone() } else { m.display_name.clone() };
            list.push(Person { username: m.username.clone(), name, in_project: true });
        }
    }
    list
}

/// 按鈕列右邊的字。
pub fn summary(checked: usize) -> String {
    if checked == 0 { "還沒選人".to_owned() } else { format!("已選 {checked} 人") }
}

struct State {
    people: Vec<Person>,
    checked: Vec<bool>,
    query: String,
}

fn rows(st: &State) -> ModelRc<PersonRow> {
    let rows: Vec<PersonRow> = st
        .people
        .iter()
        .zip(&st.checked)
        .map(|(p, c)| PersonRow {
            username: p.username.as_str().into(),
            name: p.name.as_str().into(),
            initial: initial(&p.name).into(),
            detail: if p.in_project { p.username.clone() } else { format!("{} · 不在專案成員中", p.username) }.into(),
            checked: *c,
            shown: matches(p, &st.query),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

fn render(ui: &PeoplePickerWindow, st: &State) {
    ui.set_rows(rows(st));
    ui.set_summary(summary(st.checked.iter().filter(|c| **c).count()).into());
}

/// 開選人視窗；tag 一樣的已經開著就叫到前面。選好（多選按〔確定〕、單選點一下）時呼叫 done（帳號，照清單順序），
/// 取消不呼叫。
pub fn open(pick: PeoplePick, tag: Option<String>, done: impl Fn(Vec<String>) + 'static) {
    if let Some(t) = tag.as_deref()
        && window_util::activate_tag(t)
    {
        return;
    }
    let Some(ui) = dialog::create("選擇人員", PeoplePickerWindow::new) else { return };
    let list = people(&pick.members, &pick.selected);
    let checked = list.iter().map(|p| pick.selected.iter().any(|s| s.eq_ignore_ascii_case(&p.username))).collect();
    let st = Rc::new(RefCell::new(State { people: list, checked, query: String::new() }));
    ui.set_heading(pick.heading.into());
    ui.set_hint(pick.hint.into());
    ui.set_multi(pick.multi);
    render(&ui, &st.borrow());
    let id = window_util::keep(&ui, tag, ());
    let done: Rc<dyn Fn(Vec<String>)> = Rc::new(done);
    let chosen = |st: &State| -> Vec<String> {
        st.people.iter().zip(&st.checked).filter(|(_, c)| **c).map(|(p, _)| p.username.clone()).collect()
    };
    {
        let (weak, st) = (ui.as_weak(), st.clone());
        ui.on_search_edited(move |q| {
            st.borrow_mut().query = q.to_string();
            if let Some(ui) = weak.upgrade() {
                render(&ui, &st.borrow());
            }
        });
    }
    {
        let (weak, st, done) = (ui.as_weak(), st.clone(), done.clone());
        let multi = pick.multi;
        ui.on_toggled(move |i| {
            let Some(ui) = weak.upgrade() else { return };
            let Ok(i) = usize::try_from(i) else { return };
            if !multi {
                let picked = st.borrow().people.get(i).map(|p| p.username.clone());
                if let Some(u) = picked {
                    window_util::close(&ui, id);
                    done(vec![u]);
                }
                return;
            }
            if let Some(c) = st.borrow_mut().checked.get_mut(i) {
                *c = !*c;
            }
            render(&ui, &st.borrow());
        });
    }
    {
        let (weak, st) = (ui.as_weak(), st.clone());
        ui.on_select_members(move || {
            {
                let mut s = st.borrow_mut();
                let s = &mut *s;
                for (p, c) in s.people.iter().zip(s.checked.iter_mut()) {
                    if p.in_project {
                        *c = true;
                    }
                }
            }
            if let Some(ui) = weak.upgrade() {
                render(&ui, &st.borrow());
            }
        });
    }
    {
        let (weak, st) = (ui.as_weak(), st.clone());
        ui.on_clear_all(move || {
            st.borrow_mut().checked.iter_mut().for_each(|c| *c = false);
            if let Some(ui) = weak.upgrade() {
                render(&ui, &st.borrow());
            }
        });
    }
    {
        let (weak, st, done) = (ui.as_weak(), st.clone(), done.clone());
        ui.on_accept(move || {
            let Some(ui) = weak.upgrade() else { return };
            let list = chosen(&st.borrow());
            window_util::close(&ui, id);
            done(list);
        });
    }
    {
        let weak = ui.as_weak();
        ui.on_cancel(move || {
            if let Some(ui) = weak.upgrade() {
                window_util::close(&ui, id);
            }
        });
    }
    window_util::present(&ui, id, Placement::Screen, true, (420.0, 560.0), crate::apply_theme);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(u: &str, n: &str) -> PersonRef {
        PersonRef::new(u, n)
    }

    #[test]
    fn 清單_不在成員的已選放前面_同帳號只列一次() {
        let list = people(&[m("amy", "小美"), m("bob", ""), m("AMY", "重複")], &["carl".into(), "Amy".into()]);
        assert_eq!(
            list,
            vec![
                Person { username: "carl".into(), name: "carl".into(), in_project: false },
                Person { username: "amy".into(), name: "小美".into(), in_project: true },
                Person { username: "bob".into(), name: "bob".into(), in_project: true },
            ]
        );
    }

    #[test]
    fn 搜尋名字或帳號_頭像字() {
        let p = Person { username: "harry.gu".into(), name: "古信汯".into(), in_project: true };
        assert!(matches(&p, "HARRY") && matches(&p, "信") && matches(&p, "  ") && !matches(&p, "amy"));
        assert_eq!((initial("古信汯"), initial("amy"), initial("")), ("古".to_owned(), "A".to_owned(), String::new()));
        assert_eq!((summary(0).as_str(), summary(3).as_str()), ("還沒選人", "已選 3 人"));
    }
}
