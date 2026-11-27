//! 工作台「團隊空間」頁（對應 C# `UI/Work/SpacesPage.cs`，含 `SpaceMoveWindow`）。
//! 選空間 → 左邊頁面樹（可全文搜尋）→ 右邊看那頁的 Markdown；可以新增頁、在頁面最後附加內容、
//! 編輯（改名、整頁換內容）、移到別的上層、刪除（連子頁，30 天內空間管理員可以在網頁還原）。
//! 資料夾（Hayaku 1.4 起取代檔案庫，舊的檔案庫自動變成同名資料夾）列出子資料夾、文件、白板與檔案：
//! 點兩下檔案直接看內容，〔下載〕拿下載連結（1 小時有效）用瀏覽器開；可以在資料夾裡新增文件、子資料夾。
//! 搜尋也會找到檔名符合的檔案，點了打開所在的資料夾和預覽。上傳、打包 zip 下載請到網頁。
//! 第一次切到這頁才讀；頁面樹、頁面內容快取到按重新整理或換空間。
//! 跟上 Hayaku 1.5.4（C# 版沒有）：〔上移〕〔下移〕在同一個上層裡跟前一頁、後一頁換位置（move_space_page 的 afterId），
//! 跟刪除一樣先確認（會改公司的正式文件）。拖拉排序沒有做。
//! 跟上 Hayaku 1.5.9：綁進空間的會議（會議資料夾、一場會議、那場的紀錄與白板）也在頁面樹上；
//! 紀錄跟會議頁同一份，這裡只看，〔打開會議〕到會議頁改。會議相關的頁不能新增子頁、移動、刪除，也不能當移動的目標。
//! 跟上 Hayaku 1.6.0：「我的筆記本」升級成「我的空間」（space me），導覽「我的空間」也用這一頁（personal 模式）：
//! 下拉選單是「我的空間」與「與我分享」（list_spaces 的 sharedWithMe，別人分享給我的頁與檔案，一樣用 me 讀），
//! 團隊空間的下拉選單不再列 me。兩個模式共用頁面樹與內容的快取，各自記得選了哪個空間、哪一頁。
//! 搜尋框打字時在本機篩頁名（不送請求），按 Enter 才搜內文與檔名。分享設定只能在網頁改。
//!
//! 契約（不可改簽章）：`init` 在第一次切到這頁時呼叫一次（綁定 callback）；
//! `activate` 每次切到這頁時呼叫（對應 C# `IWorkPage.ActivateAsync(arg)`）。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use hb_api::{KnowledgeHit, SharedItem, SpaceFile, SpaceInfo, SpaceNode, SpacePageContent};
use hb_ui_common::file_preview;
use hb_ui_common::inputs::{self, ActionError, StatusMessage};
use slint::ComponentHandle;

use super::meetings::w3_common::{escape_data, model, size_text, spawn, status_parts, strings, wire_md};
use crate::{PageArg, SpacesPageData, W3Row, WorkCtx};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ComposeMode {
    #[default]
    Child,
    Folder,
    Append,
    Edit,
}

/// 我的空間（list_spaces 的 key，跟上 Hayaku 1.6.0）。
const ME: &str = "me";
/// 「與我分享」在下拉選單上當成另一個空間；送 API 時一樣用 me。
const SHARED: &str = "me#shared";

/// 送 API 用的空間代碼（「與我分享」是 me）。
fn api_space(key: &str) -> &str {
    if key == SHARED { ME } else { key }
}

struct Page {
    ctx: WorkCtx,
    st: RefCell<State>,
}

/// 一個模式（我的空間／團隊空間）的選取：切換導覽時存起來，切回來還原。
#[derive(Default)]
struct View {
    space_key: Option<String>,
    node: Option<SpaceNode>,
    expanded: HashSet<String>,
    tree_selected: Option<String>,
}

#[derive(Default)]
struct State {
    /// 我的空間模式（導覽「我的空間」）；false 是團隊空間
    personal: bool,
    /// 切到過這頁了（第一次要把模式設到畫面上）
    entered: bool,
    /// 另一個模式的選取
    stash: Option<View>,
    /// 隨手記、待辦的數字（list_my_notes；我的空間才讀，打開這頁或按重新整理時）
    counts: Option<(i32, i32)>,
    trees: HashMap<String, Vec<SpaceNode>>,
    pages: HashMap<String, SpacePageContent>,
    spaces: Vec<SpaceInfo>,
    /// 選著的空間（C# `_space.SelectedItem` 的 Tag）
    space_key: Option<String>,
    node: Option<SpaceNode>,
    mode: ComposeMode,
    loaded: bool,
    version: u64,
    /// 頁面樹：展開的節點、畫面上每一列的節點 id、選中的節點
    expanded: HashSet<String>,
    tree_ids: Vec<String>,
    tree_selected: Option<String>,
    hits: Vec<KnowledgeHit>,
    files: Vec<SpaceFile>,
    /// 移動：選項的節點 id（None＝最上層）、目前位置
    move_choices: Vec<Option<String>>,
    move_current: Option<String>,
    confirm: Option<Box<dyn FnOnce()>>,
}

impl State {
    /// 選著的空間；專案封存了是唯讀（不能新增頁、附加內容）。
    fn current_space(&self) -> Option<&SpaceInfo> {
        self.spaces.iter().find(|s| Some(&s.key) == self.space_key.as_ref())
    }

    fn writable(&self) -> bool {
        !self.current_space().is_some_and(|s| s.archived)
    }

    /// 目前空間頁面樹裡的一頁（沒讀過樹、或不在樹上是 None）。
    fn node(&self, id: Option<&str>) -> Option<SpaceNode> {
        let id = id?;
        let space = self.space_key.as_ref()?;
        self.trees.get(&space.to_lowercase())?.iter().find(|n| n.id == id).cloned()
    }

    fn tree(&self) -> Option<&Vec<SpaceNode>> {
        self.trees.get(&self.space_key.as_ref()?.to_lowercase())
    }

    /// 送 API 用的空間代碼。
    fn space_api(&self) -> Option<String> {
        self.space_key.as_deref().map(|k| api_space(k).to_owned())
    }

    /// 正在看「與我分享」。
    fn shared_view(&self) -> bool {
        self.space_key.as_deref() == Some(SHARED)
    }

    fn me(&self) -> Option<&SpaceInfo> {
        self.spaces.iter().find(|s| s.is_personal())
    }

    /// 分享給我的那一筆（「與我分享」清單上的）。
    fn shared_item(&self, id: &str) -> Option<&SharedItem> {
        self.me()?.shared.iter().find(|s| s.id == id)
    }

    /// 這個模式下拉選單上的空間：我的空間是 me 與「與我分享」，團隊空間是 me 以外的。
    fn visible_keys(&self) -> Vec<String> {
        if self.personal {
            match self.me() {
                Some(me) => vec![me.key.clone(), SHARED.to_owned()],
                None => Vec::new(),
            }
        } else {
            self.spaces.iter().filter(|s| !s.is_personal()).map(|s| s.key.clone()).collect()
        }
    }

    /// 下拉選單上的文字與滑過的提示。
    fn key_text(&self, key: &str) -> (String, String) {
        if key == SHARED {
            let n = self.me().map_or(0, |m| m.shared.len());
            return (
                if n > 0 { format!("與我分享（{n}）") } else { "與我分享".to_owned() },
                "別人分享給你的頁與檔案（可檢視或可編輯，分享設定在網頁）".to_owned(),
            );
        }
        match self.spaces.iter().find(|s| s.key == key) {
            Some(s) if s.is_personal() => {
                ("我的空間".to_owned(), "只有你看得到；頁、檔案可以在網頁一個一個分享給別人".to_owned())
            }
            Some(s) => (space_text(s), space_tip(s)),
            None => (key.to_owned(), String::new()),
        }
    }

    /// 頁面樹上方的說明（幾頁；我的空間加隨手記、待辦的數字）。
    fn tree_message(&self, count: usize) -> String {
        if self.shared_view() {
            return if count == 0 {
                "還沒有人分享東西給你".to_owned()
            } else {
                format!("{count} 個分享給你的頁與檔案")
            };
        }
        if !self.personal {
            return tree_message(count);
        }
        let mut text = if count == 0 {
            "我的空間還沒有頁面，按「＋ 新頁」新增".to_owned()
        } else {
            format!("{count} 頁")
        };
        if let Some((inbox, todos)) = self.counts.filter(|(a, b)| *a > 0 || *b > 0) {
            text += &format!("・隨手記 {inbox}・待辦 {todos}");
        }
        text
    }

    /// 打開的資料夾的上層資料夾（〔上一層〕用）。
    fn parent_folder(&self) -> Option<SpaceNode> {
        let parent = self.node(self.node.as_ref()?.parent_id.as_deref())?;
        parent.is_folder().then_some(parent)
    }
}

thread_local! {
    static PAGE: RefCell<Option<Rc<Page>>> = const { RefCell::new(None) };
}

macro_rules! bind {
    // 沒有參數的回呼：rustfmt 會把 `| |` 排成 `||`，兩種都接受
    ($g:ident . $on:ident, $page:ident, || $body:expr) => {
        bind!($g.$on, $page, | | $body)
    };
    ($g:ident . $on:ident, $page:ident, | $($arg:ident),* | $body:expr) => {{
        let weak: Weak<Page> = Rc::downgrade(&$page);
        $g.$on(move |$($arg),*| {
            if let Some($page) = weak.upgrade() {
                $body
            }
        });
    }};
}

pub fn init(ctx: &WorkCtx) {
    // 導覽「我的空間」與「團隊空間」共用這一頁，只初始化一次
    if PAGE.with(|p| p.borrow().is_some()) {
        return;
    }
    let page = Rc::new(Page { ctx: ctx.clone(), st: RefCell::new(State::default()) });
    PAGE.with(|p| *p.borrow_mut() = Some(page.clone()));
    let Some(w) = ctx.window() else { return };
    let g = w.global::<SpacesPageData>();
    bind!(g.on_space_selected, page, |i| {
        let key = page.st.borrow().visible_keys().get(i as usize).cloned();
        page.st.borrow_mut().space_key = key;
        spawn(async move { page.load_tree(false).await });
    });
    bind!(g.on_search_accepted, page, || spawn(async move { page.search().await }));
    bind!(g.on_search_edited, page, |text| page.filter(text.trim()));
    bind!(g.on_search_escape, page, || {
        page.with(|g| g.set_search("".into()));
        page.show_tree();
    });
    bind!(g.on_tree_selected, page, |i| {
        let node = {
            let mut st = page.st.borrow_mut();
            let id = st.tree_ids.get(i as usize).cloned();
            st.tree_selected = id.clone();
            st.node(id.as_deref())
        };
        if let Some(node) = node {
            spawn(async move { page.open(node, None).await });
        }
    });
    bind!(g.on_tree_toggled, page, |i| {
        {
            let mut st = page.st.borrow_mut();
            if let Some(id) = st.tree_ids.get(i as usize).cloned()
                && !st.expanded.remove(&id)
            {
                st.expanded.insert(id);
            }
        }
        page.render_tree_rows();
    });
    bind!(g.on_hit_selected, page, |i| {
        let hit = page.st.borrow().hits.get(i as usize).cloned();
        if let Some(hit) = hit {
            spawn(async move { page.open_hit(hit).await });
        }
    });
    bind!(g.on_up, page, || {
        let parent = page.st.borrow().parent_folder();
        if let Some(parent) = parent {
            spawn(async move { page.open(parent, None).await });
        }
    });
    bind!(g.on_file_activated, page, |_i| spawn(async move { page.open_file(false).await }));
    bind!(g.on_download, page, || spawn(async move { page.open_file(true).await }));
    bind!(g.on_child, page, || page.compose(ComposeMode::Child, false));
    bind!(g.on_folder, page, || page.compose(ComposeMode::Folder, false));
    bind!(g.on_append, page, || page.compose(ComposeMode::Append, false));
    bind!(g.on_edit, page, || page.compose(ComposeMode::Edit, false));
    bind!(g.on_new_top, page, || page.compose(ComposeMode::Child, true));
    bind!(g.on_move, page, || page.open_move());
    bind!(g.on_move_up, page, || page.reorder(true));
    bind!(g.on_move_down, page, || page.reorder(false));
    bind!(g.on_delete, page, || page.delete());
    bind!(g.on_compose_submit, page, || spawn(async move { page.submit().await }));
    bind!(g.on_compose_cancel, page, || {
        page.with(|g| g.set_composer_open(false));
    });
    bind!(g.on_reload, page, || spawn(async move { page.load_spaces().await }));
    bind!(g.on_web, page, || {
        let (key, personal) = {
            let st = page.st.borrow();
            (st.space_key.clone(), st.personal)
        };
        let path = match key {
            // 我的空間：舊的筆記本網址會自動轉到新位置（Hayaku 1.6.0）
            _ if personal => "/notebook".to_owned(),
            Some(k) => format!("/spaces/{}", escape_data(&k)),
            None => "/spaces".to_owned(),
        };
        hb_win::open(&page.ctx.engine.web_url(&path));
    });
    bind!(g.on_open_meeting, page, || {
        let key = page.with(|g| g.get_meeting_key().to_string()).unwrap_or_default();
        if !key.is_empty() {
            page.ctx.show_page("meetings", Some(PageArg::Text(key)));
        }
    });
    bind!(g.on_move_ok, page, || page.move_ok());
    bind!(g.on_move_cancel, page, || {
        page.with(|g| g.set_move_open(false));
    });
    bind!(g.on_confirm_ok, page, || {
        let action = page.st.borrow_mut().confirm.take();
        page.with(|g| g.set_confirm_open(false));
        if let Some(action) = action {
            action();
        }
    });
    bind!(g.on_confirm_cancel, page, || {
        page.st.borrow_mut().confirm = None;
        page.with(|g| g.set_confirm_open(false));
    });
    // 跟上 Hayaku 1.5.10：頁面樹拖分隔線調寬，記住調的寬度（0＝預設）
    g.set_tree_width(ctx.engine.config().spaces_tree_width.max(0) as f32);
    {
        let engine = ctx.engine.clone();
        g.on_tree_width_saved(move |w| {
            let w = if w > 0.0 { w.round() as i32 } else { 0 };
            if engine.config().spaces_tree_width != w {
                engine.config_mut().spaces_tree_width = w;
                engine.config().save();
            }
        });
    }
    wire_md!(&g);
    page.show_empty();
}

/// 導覽「團隊空間」。
pub async fn activate(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    page.enter(false, arg).await;
}

/// 導覽「我的空間」（跟上 Hayaku 1.6.0，取代原本的筆記頁）；arg "shared" 打開「與我分享」。
pub async fn activate_personal(_ctx: WorkCtx, arg: Option<PageArg>) {
    let Some(page) = PAGE.with(|p| p.borrow().clone()) else { return };
    page.enter(true, arg).await;
}

impl Page {
    /// 切到這頁：模式不同就把目前的選取存起來、換回另一個模式的；第一次才讀空間清單，之後用快取。
    async fn enter(self: Rc<Self>, personal: bool, arg: Option<PageArg>) {
        let switched = {
            let mut st = self.st.borrow_mut();
            let switched = !st.entered || st.personal != personal;
            st.entered = true;
            if st.personal != personal {
                let current = View {
                    space_key: st.space_key.take(),
                    node: st.node.take(),
                    expanded: std::mem::take(&mut st.expanded),
                    tree_selected: st.tree_selected.take(),
                };
                let back = st.stash.replace(current).unwrap_or_default();
                st.space_key = back.space_key;
                st.node = back.node;
                st.expanded = back.expanded;
                st.tree_selected = back.tree_selected;
                st.personal = personal;
            }
            switched
        };
        if switched {
            self.st.borrow_mut().confirm = None;
            self.with(|g| {
                g.set_personal(personal);
                g.set_search("".into());
                g.set_show_hits(false);
                g.set_composer_open(false);
                g.set_move_open(false);
                g.set_confirm_open(false);
            });
        }
        let to_shared = personal && matches!(&arg, Some(PageArg::Text(t)) if t == "shared");
        if to_shared {
            self.st.borrow_mut().space_key = Some(SHARED.to_owned());
        }
        if !self.st.borrow().loaded {
            self.load_spaces().await;
            return;
        }
        if personal && self.st.borrow().counts.is_none() {
            let page = self.clone();
            spawn(async move { page.load_counts().await });
        }
        if !switched && !to_shared {
            return;
        }
        if self.render_space_list() {
            self.restore_view().await;
        } else {
            self.show_no_spaces();
        }
    }

    /// 換回這個模式記得的空間與頁：頁面樹用快取，打開原本那頁（內容有快取就不再讀）。
    async fn restore_view(self: Rc<Self>) {
        let (node, expanded, selected) = {
            let st = self.st.borrow();
            (st.node.clone(), st.expanded.clone(), st.tree_selected.clone())
        };
        self.clone().load_tree(false).await;
        let node = node.filter(|n| self.st.borrow().node(Some(&n.id)).is_some());
        if !expanded.is_empty() {
            {
                let mut st = self.st.borrow_mut();
                st.expanded.extend(expanded);
                st.tree_selected = selected;
            }
            self.render_tree_rows();
        }
        if let Some(node) = node {
            self.open(node, None).await;
        }
    }

    /// 隨手記、待辦的數字（我的空間頁面樹上方）；讀不到就不顯示。
    async fn load_counts(self: Rc<Self>) {
        match self.ctx.engine.api().list_notes().await {
            Ok(nb) => {
                self.st.borrow_mut().counts = Some((nb.inbox, nb.todos));
                let showing_tree = self.with(|g| !g.get_show_hits()).unwrap_or(false);
                if self.st.borrow().personal && showing_tree {
                    self.show_tree();
                }
            }
            Err(e) => log::warn!("讀取隨手記、待辦的數字失敗: {e}"),
        }
    }

    /// 下拉選單換成這個模式的空間；選回原本的空間，沒有就選第一個。回傳有沒有空間可選。
    fn render_space_list(&self) -> bool {
        let (items, tips, index) = {
            let mut st = self.st.borrow_mut();
            let keys = st.visible_keys();
            let (items, tips): (Vec<String>, Vec<String>) = keys.iter().map(|k| st.key_text(k)).unzip();
            let index = keys
                .iter()
                .position(|k| Some(k) == st.space_key.as_ref())
                .or(if keys.is_empty() { None } else { Some(0) });
            st.space_key = index.map(|i| keys[i].clone());
            (items, tips, index)
        };
        let any = !items.is_empty();
        self.with(|g| {
            g.set_spaces(strings(items));
            g.set_space_tips(strings(tips));
            g.set_space_index(index.map_or(-1, |i| i as i32));
        });
        any
    }

    fn show_no_spaces(&self) {
        let personal = self.st.borrow().personal;
        self.left(if personal { "讀不到我的空間（Hayaku 1.6.0 以後才有）" } else { "你還沒有看得到的團隊空間" });
        self.st.borrow_mut().tree_ids.clear();
        self.with(|g| {
            g.set_tree(model(Vec::new()));
            g.set_new_top_visible(false);
        });
        self.show_empty();
    }

    fn with<R>(&self, f: impl FnOnce(&SpacesPageData<'_>) -> R) -> Option<R> {
        let w = self.ctx.window()?;
        let g = w.global::<SpacesPageData>();
        Some(f(&g))
    }

    fn show(&self, m: Option<StatusMessage>) {
        let (text, tone) = status_parts(m.as_ref());
        self.with(|g| {
            g.set_status(text);
            g.set_status_tone(tone);
        });
    }

    fn status_text(&self) -> String {
        self.with(|g| g.get_status().to_string()).unwrap_or_default()
    }

    fn next_version(&self) -> u64 {
        let mut st = self.st.borrow_mut();
        st.version += 1;
        st.version
    }

    fn left(&self, text: &str) {
        self.with(|g| g.set_left_message(text.into()));
    }

    fn right(&self, text: &str, visible: bool) {
        self.with(|g| {
            g.set_right_message(text.into());
            g.set_right_visible(visible);
        });
    }

    /// 重新整理：重抓空間清單，清掉頁面樹與內容的快取。
    async fn load_spaces(self: Rc<Self>) {
        let version = self.next_version();
        {
            let mut st = self.st.borrow_mut();
            st.trees.clear();
            st.pages.clear();
        }
        self.show(None);
        self.left("讀取中…");
        match self.ctx.engine.api().list_spaces().await {
            Ok(spaces) => {
                if version != self.st.borrow().version {
                    return;
                }
                let mut st = self.st.borrow_mut();
                st.spaces = spaces;
                st.loaded = true;
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取團隊空間失敗: {e}");
                self.left("");
                self.show(Some(StatusMessage::error(format!("讀不到團隊空間：{e}"))));
                return;
            }
        }
        // 專案裡開的空間標專案代碼（跟網頁側邊欄一樣），專案封存了的標唯讀；我的空間只在導覽「我的空間」列
        if !self.render_space_list() {
            self.show_no_spaces();
            return;
        }
        if self.st.borrow().personal {
            let page = self.clone();
            spawn(async move { page.load_counts().await });
        }
        self.load_tree(true).await;
    }

    async fn load_tree(self: Rc<Self>, refresh: bool) {
        let Some(space) = self.st.borrow().space_key.clone() else { return };
        let writable = self.st.borrow().writable();
        self.with(|g| {
            g.set_new_top_visible(writable && space != SHARED);
            g.set_search("".into());
        });
        self.show_tree();
        // 與我分享：清單在 list_spaces 裡，不用另外讀（重新整理是重讀空間清單）
        if space == SHARED {
            let nodes = self.st.borrow().me().map(|m| shared_nodes(&m.shared)).unwrap_or_default();
            self.st.borrow_mut().trees.insert(SHARED.to_owned(), nodes.clone());
            self.render_tree(&nodes);
            return;
        }
        let cached = if refresh { None } else { self.st.borrow().trees.get(&space.to_lowercase()).cloned() };
        if let Some(nodes) = cached {
            self.render_tree(&nodes);
            return;
        }
        let version = self.next_version();
        self.left("讀取中…");
        self.st.borrow_mut().tree_ids.clear();
        self.with(|g| g.set_tree(model(Vec::new())));
        match self.ctx.engine.api().space_tree(&space).await {
            Ok(nodes) => {
                if version != self.st.borrow().version || self.st.borrow().space_key.as_ref() != Some(&space) {
                    return;
                }
                self.st.borrow_mut().trees.insert(space.to_lowercase(), nodes.clone());
                self.render_tree(&nodes);
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取團隊空間 {space} 的頁面樹失敗: {e}");
                self.left("");
                self.show(Some(StatusMessage::error(format!("讀不到頁面：{e}"))));
            }
        }
    }

    fn render_tree(&self, nodes: &[SpaceNode]) {
        {
            let mut st = self.st.borrow_mut();
            // 最上層預設展開（C# IsExpanded = depth == 0）；重畫後沒有選取
            st.expanded = tree_roots(nodes).into_iter().map(|n| n.id.clone()).collect();
            st.tree_selected = None;
        }
        self.render_tree_rows();
        let message = self.st.borrow().tree_message(nodes.len());
        self.left(&message);
        let gone = {
            let st = self.st.borrow();
            st.node.as_ref().is_none_or(|n| !nodes.iter().any(|x| x.id == n.id))
        };
        if gone {
            self.show_empty();
        }
    }

    /// 依展開狀態把頁面樹攤平成清單。
    fn render_tree_rows(&self) {
        let (rows, ids, current) = {
            let st = self.st.borrow();
            let nodes = st.tree().cloned().unwrap_or_default();
            let flat = flatten_tree(&nodes, &st.expanded);
            let current = st.tree_selected.as_ref().and_then(|s| flat.iter().position(|(n, _, _)| &n.id == s));
            let ids: Vec<String> = flat.iter().map(|(n, _, _)| n.id.clone()).collect();
            let rows: Vec<W3Row> = flat
                .iter()
                .map(|(n, depth, has_children)| W3Row {
                    id: n.id.as_str().into(),
                    title: node_text(n).into(),
                    tip: node_tip(n).into(),
                    depth: *depth as i32,
                    expandable: *has_children,
                    expanded: st.expanded.contains(&n.id),
                    ..Default::default()
                })
                .collect();
            (rows, ids, current)
        };
        self.st.borrow_mut().tree_ids = ids;
        self.with(|g| {
            g.set_tree(model(rows));
            g.set_tree_current(current.map_or(-1, |i| i as i32));
        });
    }

    fn show_tree(&self) {
        let message = {
            let st = self.st.borrow();
            st.tree().map(|t| st.tree_message(t.len()))
        };
        self.with(|g| g.set_show_hits(false));
        if let Some(message) = message {
            self.left(&message);
        }
    }

    /// 打字時篩頁名（跟上 Hayaku 1.6.0「篩頁名，Enter 搜內文」）：只看已經讀好的頁面樹，不送請求。
    fn filter(&self, q: &str) {
        if q.is_empty() {
            self.show_tree();
            return;
        }
        let (hits, rows) = {
            let st = self.st.borrow();
            let nodes = st.tree().cloned().unwrap_or_default();
            let space = st.space_api();
            filter_nodes(&nodes, q)
                .into_iter()
                .map(|(n, path)| {
                    let hit = KnowledgeHit {
                        id: n.id.clone(),
                        title: n.title.clone(),
                        space: space.clone(),
                        snippet: (!path.is_empty()).then(|| path.clone()),
                        folder_id: None,
                        meeting: n.meeting.clone(),
                    };
                    let row = W3Row { title: node_text(n).into(), meta: path.into(), ..Default::default() };
                    (hit, row)
                })
                .unzip::<_, _, Vec<_>, Vec<_>>()
        };
        let message = if hits.is_empty() {
            format!("沒有頁名含「{q}」的頁，按 Enter 搜內文與檔名")
        } else {
            format!("頁名含「{q}」的有 {} 個（Enter 搜內文，Esc 回到頁面樹）", hits.len())
        };
        self.st.borrow_mut().hits = hits;
        self.with(|g| {
            g.set_show_hits(true);
            g.set_hits(model(rows));
            g.set_hit_current(-1);
        });
        self.left(&message);
    }

    async fn search(self: Rc<Self>) {
        let q = self.with(|g| g.get_search().trim().to_owned()).unwrap_or_default();
        if q.is_empty() {
            self.show_tree();
            return;
        }
        let version = self.next_version();
        self.st.borrow_mut().hits.clear();
        self.with(|g| {
            g.set_show_hits(true);
            g.set_hits(model(Vec::new()));
            g.set_hit_current(-1);
        });
        self.left("搜尋中…");
        let space = self.st.borrow().space_api();
        match self.ctx.engine.api().search_spaces(&q, space.as_deref(), None).await {
            Ok(hits) => {
                // 搜尋中又打字（改成篩頁名）就不蓋掉
                let still = self.with(|g| g.get_search().trim() == q).unwrap_or(false);
                if version != self.st.borrow().version || !still {
                    return;
                }
                let rows: Vec<W3Row> = hits
                    .iter()
                    .map(|h| W3Row {
                        title: format!(
                            "{}{}",
                            if h.is_file() {
                                "📄 "
                            } else if h.meeting.is_some() {
                                "🗓 "
                            } else {
                                ""
                            },
                            h.title
                        )
                        .into(),
                        meta: h.snippet.clone().unwrap_or_default().into(),
                        ..Default::default()
                    })
                    .collect();
                let files = hits.iter().filter(|h| h.is_file()).count();
                let message = if hits.is_empty() {
                    format!("找不到「{q}」（Esc 回到頁面樹）")
                } else {
                    format!("「{q}」找到 {} 頁", hits.len() - files)
                        + &if files > 0 { format!("、{files} 個檔案") } else { String::new() }
                        + "（Esc 回到頁面樹）"
                };
                self.st.borrow_mut().hits = hits;
                self.with(|g| g.set_hits(model(rows)));
                self.left(&message);
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("搜尋團隊空間失敗: {e}");
                self.left("");
                self.show(Some(StatusMessage::error(format!("搜尋失敗：{e}"))));
            }
        }
    }

    /// 頁：打開那頁；檔案：打開所在的資料夾，再開預覽。
    async fn open_hit(self: Rc<Self>, hit: KnowledgeHit) {
        let Some(space) = hit.space.clone().or_else(|| self.st.borrow().space_api()) else { return };
        let id = hit.folder_id.clone().unwrap_or_else(|| hit.id.clone());
        let node = self.st.borrow().node(Some(&id)).unwrap_or_else(|| {
            if hit.is_file() {
                new_node(&id, "資料夾", None, Some("FOLDER"), None)
            } else {
                new_node(&hit.id, &hit.title, None, None, None)
            }
        });
        self.clone().open(node, Some(space)).await;
        if hit.is_file() {
            let w = self.ctx.window();
            file_preview::show(&self.ctx.engine, &hit.id, &hit.title, w.as_ref().map(|w| w.window()), None);
        }
    }

    async fn open(self: Rc<Self>, node: SpaceNode, space: Option<String>) {
        let Some(space) = space.or_else(|| self.st.borrow().space_key.clone()) else { return };
        let space = api_space(&space).to_owned();
        self.st.borrow_mut().node = Some(node.clone());
        self.with(|g| {
            g.set_composer_open(false);
            g.set_title(node.title.as_str().into());
            g.set_meeting_key("".into());
        });
        if node.is_folder() {
            self.load_files().await;
            return;
        }
        // 分享給我的檔案（與我分享）：直接看內容
        if node.is_file() {
            let meta = self.st.borrow().shared_item(&node.id).map(shared_meta).unwrap_or_default();
            self.with(|g| {
                g.set_meta(meta.into());
                g.set_content_visible(false);
                g.set_files_visible(false);
                g.set_actions_visible(false);
            });
            self.right("檔案在預覽視窗打開了；要下載、換新版本請到網頁", true);
            let w = self.ctx.window();
            file_preview::show(&self.ctx.engine, &node.id, &node.title, w.as_ref().map(|w| w.window()), None);
            return;
        }
        if node.is_meeting_occurrence() {
            self.show_occurrence(&node);
            return;
        }
        self.with(|g| g.set_files_visible(false));
        let cached = self.st.borrow().pages.get(&node.id).cloned();
        if let Some(page) = cached {
            self.show_page(&page);
            return;
        }
        let version = self.next_version();
        self.with(|g| {
            g.set_meta("".into());
            g.set_content_visible(false);
            g.set_actions_visible(false);
        });
        self.right("讀取中…", true);
        match self.ctx.engine.api().get_space_page(&space, &node.id).await {
            Ok(page) => {
                self.st.borrow_mut().pages.insert(node.id.clone(), page.clone());
                if version != self.st.borrow().version
                    || self.st.borrow().node.as_ref().map(|n| &n.id) != Some(&node.id)
                {
                    return;
                }
                self.show_page(&page);
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取團隊空間頁面 {} 失敗: {e}", node.id);
                self.right(&format!("讀不到這頁：{e}"), true);
            }
        }
    }

    fn show_page(&self, page: &SpacePageContent) {
        let mut meta = Vec::new();
        if !page.path.is_empty() {
            meta.push(page.path.join(" › "));
        }
        if let Some(owner) = &page.owner {
            meta.push(format!("負責人 {owner}"));
        }
        if let Some(at) = &page.updated_at {
            meta.push(format!("{}更新", inputs::ago(at)));
        }
        // 綁進空間的會議（跟上 Hayaku 1.5.9）：紀錄跟會議頁同一份，要改到會議頁
        let node = self.st.borrow().node.clone();
        let meeting = page.meeting.clone().or_else(|| node.as_ref().and_then(|n| n.meeting.clone()));
        let meeting_page =
            meeting.is_some() || !page.occurrences.is_empty() || node.as_ref().is_some_and(SpaceNode::is_meeting);
        if !page.occurrences.is_empty() {
            meta.insert(0, format!("會議資料夾・{} 場", page.occurrences.len()));
        } else if meeting.is_some() {
            meta.insert(0, "會議紀錄・跟會議頁同一份（要改請打開會議）".to_owned());
        }
        // 與我分享：誰分享的、可檢視或可編輯
        if let Some(shared) = node.as_ref().and_then(|n| self.st.borrow().shared_item(&n.id).map(shared_meta)) {
            meta.insert(0, shared);
        }
        let mut meta = meta.join("・");
        let mut content = page.markdown.trim().to_owned();
        if content.is_empty() && !page.occurrences.is_empty() {
            content = occurrences_markdown(&page.occurrences);
        }
        let has_content = !content.is_empty();
        self.with(|g| {
            g.set_title(page.title.as_str().into());
            g.set_content(content.into());
            g.set_content_visible(has_content);
            g.set_meeting_key(meeting.clone().unwrap_or_default().into());
        });
        self.right("這頁還沒有內容", !has_content);
        if meeting_page {
            self.with(|g| g.set_actions_visible(false));
        } else {
            self.show_actions(false);
        }
        if !self.st.borrow().writable() {
            meta += &format!("{}專案已封存，只能看", if meta.is_empty() { "" } else { "・" });
        }
        self.with(|g| g.set_meta(meta.into()));
    }

    /// 一場會議（MEETING_OCCURRENCE）：沒有自己的內文，紀錄、白板在樹上它的底下；〔打開會議〕到會議頁。
    fn show_occurrence(&self, node: &SpaceNode) {
        self.with(|g| {
            g.set_title(node.title.as_str().into());
            g.set_meta("一場會議・紀錄和白板在左邊這場底下".into());
            g.set_content_visible(false);
            g.set_files_visible(false);
            g.set_actions_visible(false);
            g.set_meeting_key(node.meeting.clone().unwrap_or_else(|| node.id.clone()).into());
        });
        self.right("按〔打開會議〕看時間、參加者、會議紀錄", true);
    }

    /// 頁面的按鈕：資料夾沒有內文，不能附加內容、編輯。
    /// 與我分享（跟上 Hayaku 1.6.0）：別人的頁只能附加內容、編輯（可編輯才有；看不出來的照樣給，Hayaku 會擋），
    /// 不能新增子頁、移動、刪除。
    fn show_actions(&self, folder: bool) {
        let (writable, structure) = {
            let st = self.st.borrow();
            let shared = st.shared_view();
            let view_only = shared
                && st.node.as_ref().and_then(|n| st.shared_item(&n.id)).and_then(|s| s.can_edit) == Some(false);
            (st.writable() && !view_only && (!shared || !folder), !shared)
        };
        self.with(|g| {
            g.set_doc_only_visible(!folder);
            g.set_structure_visible(structure);
            g.set_actions_visible(writable);
        });
        self.render_order_buttons();
    }

    /// 〔上移〕〔下移〕：已經在同一層的最前、最後時停用。
    fn render_order_buttons(&self) {
        let (up, down) = {
            let st = self.st.borrow();
            match (st.node.as_ref(), st.tree()) {
                (Some(node), Some(nodes)) => {
                    (reorder_target(nodes, node, true).is_some(), reorder_target(nodes, node, false).is_some())
                }
                _ => (false, false),
            }
        };
        self.with(|g| {
            g.set_move_up_enabled(up);
            g.set_move_down_enabled(down);
        });
    }

    fn show_empty(&self) {
        let message = {
            let mut st = self.st.borrow_mut();
            st.node = None;
            if st.spaces.is_empty() && st.loaded { "" } else { "左邊選一頁" }
        };
        self.with(|g| {
            g.set_title("".into());
            g.set_meta("".into());
            g.set_content_visible(false);
            g.set_files_visible(false);
            g.set_actions_visible(false);
            g.set_meeting_key("".into());
        });
        self.right(message, true);
    }

    // ───────────── 資料夾 ─────────────

    async fn load_files(self: Rc<Self>) {
        let (node, space) = {
            let st = self.st.borrow();
            match (st.node.clone(), st.space_api()) {
                (Some(n), Some(s)) if n.is_folder() => (n, s),
                _ => return,
            }
        };
        let version = self.next_version();
        let (up, path, writable) = {
            let st = self.st.borrow();
            (st.parent_folder().is_some(), self.folder_path(&node).join(" › "), st.writable())
        };
        self.st.borrow_mut().files.clear();
        self.with(|g| {
            g.set_content_visible(false);
            g.set_files_visible(true);
            g.set_up_visible(up);
            g.set_meta(format!("{path}{}", if writable { "" } else { "・專案已封存，只能看" }).into());
            g.set_files(model(Vec::new()));
            g.set_file_current(-1);
        });
        self.show_actions(true);
        self.right("讀取中…", true);
        match self.ctx.engine.api().list_space_files(&space, &node.id).await {
            Ok(files) => {
                if version != self.st.borrow().version
                    || self.st.borrow().node.as_ref().map(|n| &n.id) != Some(&node.id)
                {
                    return;
                }
                let rows: Vec<W3Row> = files
                    .iter()
                    .map(|f| W3Row {
                        title: format!("{}{}", file_icon(f), f.name).into(),
                        meta: file_meta(f).into(),
                        // 滑過看得到星期（跟上 Hayaku 1.5.5）
                        tip: f.at.as_ref().map(|at| format!("修改時間 {}", modified_text(at, true))).unwrap_or_default().into(),
                        ..Default::default()
                    })
                    .collect();
                let empty = files.is_empty();
                self.st.borrow_mut().files = files;
                self.with(|g| g.set_files(model(rows)));
                self.right(if empty { "這個資料夾是空的" } else { "" }, empty);
            }
            Err(e) => {
                if version != self.st.borrow().version {
                    return;
                }
                log::error!("讀取資料夾 {} 失敗: {e}", node.id);
                self.right(&format!("讀不到資料夾：{e}"), true);
            }
        }
    }

    /// 資料夾在空間裡的路徑（頁面樹上的上層名稱）。
    fn folder_path(&self, node: &SpaceNode) -> Vec<String> {
        let st = self.st.borrow();
        let mut path = vec![node.title.clone()];
        let mut seen: HashSet<String> = HashSet::from([node.id.clone()]);
        let mut p = st.node(node.parent_id.as_deref());
        while let Some(n) = p {
            if !seen.insert(n.id.clone()) {
                break;
            }
            path.insert(0, n.title.clone());
            p = st.node(n.parent_id.as_deref());
        }
        path
    }

    /// 子資料夾、文件：打開；檔案：直接看內容（read_file），download 才拿下載連結用瀏覽器下載。
    async fn open_file(self: Rc<Self>, download: bool) {
        let index = self.with(|g| g.get_file_current()).unwrap_or(-1);
        let file = if index >= 0 { self.st.borrow().files.get(index as usize).cloned() } else { None };
        let Some(file) = file else {
            if download {
                self.show(Some(StatusMessage::error("先在清單選一個檔案")));
            }
            return;
        };
        if file.is_folder || file.is_page() {
            if download {
                self.show(Some(StatusMessage::error(if file.is_folder {
                    "資料夾要打包下載請到網頁"
                } else {
                    "這是頁面，不是檔案"
                })));
                return;
            }
            let node = {
                let st = self.st.borrow();
                st.node(Some(&file.id)).unwrap_or_else(|| {
                    let parent = st.node.as_ref().map(|n| n.id.clone());
                    let kind = if file.is_folder { Some("FOLDER".to_owned()) } else { file.kind.clone() };
                    SpaceNode {
                        id: file.id.clone(),
                        title: file.name.clone(),
                        parent_id: parent,
                        kind,
                        updated_at: file.at,
                        file_count: None,
                        meeting: None,
                    }
                })
            };
            self.open(node, None).await;
            return;
        }
        if !download {
            let w = self.ctx.window();
            file_preview::show(&self.ctx.engine, &file.id, &file.name, w.as_ref().map(|w| w.window()), None);
            return;
        }
        let api = self.ctx.engine.api();
        let link: Rc<RefCell<Option<String>>> = Rc::default();
        let l = link.clone();
        let id = file.id.clone();
        let ok = inputs::run(
            |_| {},
            |m| self.show(m),
            || async move {
                *l.borrow_mut() = api.space_file_link(&id).await?;
                Ok::<(), ActionError>(())
            },
            None,
        )
        .await;
        if ok {
            match link.borrow_mut().take() {
                None => self.show(Some(StatusMessage::error("拿不到下載連結，請到網頁下載"))),
                Some(link) => hb_win::open(&link),
            }
        }
    }

    // ───────────── 新增頁、附加內容、編輯 ─────────────

    fn compose(&self, mode: ComposeMode, top_level: bool) {
        let (label, page) = {
            let mut st = self.st.borrow_mut();
            if st.space_key.is_none() {
                return;
            }
            if top_level {
                st.node = None;
            }
            let page = match mode {
                ComposeMode::Edit => match st.node.as_ref().and_then(|n| st.pages.get(&n.id)) {
                    Some(p) => Some(p.clone()),
                    None => return,
                },
                _ => None,
            };
            st.mode = mode;
            let title = st.node.as_ref().map(|n| n.title.clone());
            let label = match mode {
                ComposeMode::Append => format!("附加到「{}」最後", title.unwrap_or_default()),
                ComposeMode::Edit => format!("編輯「{}」", title.unwrap_or_default()),
                ComposeMode::Folder => match title {
                    None => "在最上層新增資料夾".to_owned(),
                    Some(t) => format!("在「{t}」底下新增資料夾"),
                },
                ComposeMode::Child => match title {
                    None => "在最上層新增一頁".to_owned(),
                    Some(t) => format!("在「{t}」底下新增一頁"),
                },
            };
            (label, page)
        };
        let hint = match mode {
            ComposeMode::Append => "要加在這頁最後的內容（Markdown，不會取代原本的內容）",
            ComposeMode::Edit => "整頁的內容（Markdown，會取代原本的內容）",
            _ => "內容（Markdown，可以先空著）",
        };
        self.with(|g| {
            g.set_composer_label(label.into());
            g.set_compose_title_visible(mode != ComposeMode::Append);
            g.set_compose_body_visible(mode != ComposeMode::Folder);
            g.set_compose_body_hint(hint.into());
            if let Some(page) = &page {
                g.set_compose_title(page.title.as_str().into());
                g.set_compose_body(page.markdown.trim().into());
            }
            g.set_composer_open(true);
            g.set_focus_body(mode == ComposeMode::Append);
            g.set_focus_token(g.get_focus_token() + 1);
        });
    }

    async fn submit(self: Rc<Self>) {
        let Some(space) = self.st.borrow().space_api() else { return };
        let Some((title, body)) =
            self.with(|g| (g.get_compose_title().trim().to_owned(), g.get_compose_body().trim().to_owned()))
        else {
            return;
        };
        let (mode, parent) = {
            let st = self.st.borrow();
            (st.mode, st.node.clone())
        };
        let focus = |body: bool| {
            self.with(|g| {
                g.set_focus_body(body);
                g.set_focus_token(g.get_focus_token() + 1);
            });
        };
        if mode != ComposeMode::Append && title.is_empty() {
            self.show(Some(StatusMessage::error("請填名稱")));
            focus(false);
            return;
        }
        if mode == ComposeMode::Append && (body.is_empty() || parent.is_none()) {
            self.show(Some(StatusMessage::error("請先填要附加的內容")));
            focus(true);
            return;
        }
        let api = self.ctx.engine.api();
        let parent_id = parent.as_ref().map(|p| p.id.clone());
        let p = self.clone();
        let busy = |b: bool| {
            p.with(|g| g.set_composing(b));
        };
        let show = |m| p.show(m);
        let ok = match mode {
            ComposeMode::Child => {
                let (t, b) = (title.clone(), body.clone());
                inputs::run(
                    busy,
                    show,
                    || async move {
                        api.create_space_page(&space, parent_id.as_deref(), &t, &b, false)
                            .await
                            .map(drop)
                            .map_err(ActionError::from)
                    },
                    Some(&format!("已新增「{title}」")),
                )
                .await
            }
            ComposeMode::Folder => {
                let t = title.clone();
                inputs::run(
                    busy,
                    show,
                    || async move {
                        api.create_space_page(&space, parent_id.as_deref(), &t, "", true)
                            .await
                            .map(drop)
                            .map_err(ActionError::from)
                    },
                    Some(&format!("已新增資料夾「{title}」")),
                )
                .await
            }
            ComposeMode::Append => {
                let (id, b) = (parent_id.clone().unwrap_or_default(), body.clone());
                inputs::run(
                    busy,
                    show,
                    || async move { api.append_space_page(&space, &id, &b).await.map_err(ActionError::from) },
                    Some("已附加到這頁"),
                )
                .await
            }
            ComposeMode::Edit => {
                let old = parent.as_ref().and_then(|n| self.st.borrow().pages.get(&n.id).cloned());
                let (Some(parent), Some(old)) = (parent.clone(), old) else { return };
                // 只送有改的：沒改名不送 title、內容沒變不送 markdown（免得多存一版）
                let new_title = (title != old.title).then(|| title.clone());
                let new_body = (body != old.markdown.trim()).then(|| body.clone());
                if new_title.is_none() && new_body.is_none() {
                    self.with(|g| g.set_composer_open(false));
                    return;
                }
                inputs::run(
                    busy,
                    show,
                    || async move {
                        api.update_space_page(&space, &parent.id, new_title.as_deref(), new_body.as_deref())
                            .await
                            .map_err(ActionError::from)
                    },
                    Some("已儲存"),
                )
                .await
            }
        };
        if !ok {
            return;
        }
        self.with(|g| {
            g.set_compose_title("".into());
            g.set_compose_body("".into());
            g.set_composer_open(false);
        });
        let message = self.status_text();
        let created = matches!(mode, ComposeMode::Child | ComposeMode::Folder);
        if created || (mode == ComposeMode::Edit && Some(&title) != parent.as_ref().map(|p| &p.title)) {
            self.clone().load_tree(true).await;
        }
        match parent {
            Some(parent) if created && parent.is_folder() => self.clone().open(parent, None).await, // 資料夾裡新增的，重讀資料夾的清單
            Some(parent) if !created => {
                self.st.borrow_mut().pages.remove(&parent.id);
                let title = if mode == ComposeMode::Edit { title } else { parent.title.clone() };
                self.clone().open(SpaceNode { title, ..parent }, None).await;
            }
            _ => {}
        }
        self.show(Some(StatusMessage::ok(message)));
    }

    // ───────────── 移動、刪除 ─────────────

    /// 選新的上層（同一個空間、不能放進自己或子頁底下），移過去排在最後。
    fn open_move(&self) {
        let (choices, current, title) = {
            let st = self.st.borrow();
            let (Some(node), Some(nodes)) = (st.node.clone(), st.tree()) else { return };
            let (choices, current) = move_choices(nodes, &node);
            (choices, current, node.title)
        };
        let index = choices.iter().position(|(id, _)| *id == current).unwrap_or(0);
        let rows: Vec<W3Row> = choices
            .iter()
            .map(|(id, text)| W3Row {
                title: format!("{text}{}", if *id == current { "　· 目前位置" } else { "" }).into(),
                ..Default::default()
            })
            .collect();
        {
            let mut st = self.st.borrow_mut();
            st.move_choices = choices.into_iter().map(|(id, _)| id).collect();
            st.move_current = current;
        }
        self.with(|g| {
            g.set_move_label(format!("把「{title}」移到哪一頁底下？（排在最後）").into());
            g.set_move_choices(model(rows));
            g.set_move_index(index as i32);
            g.set_move_open(true);
        });
    }

    fn move_ok(self: &Rc<Self>) {
        let index = self.with(|g| g.get_move_index()).unwrap_or(-1);
        let Some(target) = self.st.borrow().move_choices.get(index.max(0) as usize).cloned() else { return };
        self.with(|g| g.set_move_open(false));
        let (space, node, current) = {
            let st = self.st.borrow();
            (st.space_api(), st.node.clone(), st.move_current.clone())
        };
        let (Some(space), Some(node)) = (space, node) else { return };
        if target == current {
            return;
        }
        let page = self.clone();
        spawn(async move {
            let api = page.ctx.engine.api();
            let id = node.id.clone();
            let ok = inputs::run(
                |_| {},
                |m| page.show(m),
                || async move { api.move_space_page(&space, &id, target.as_deref()).await.map_err(ActionError::from) },
                Some(&format!("已移動「{}」", node.title)),
            )
            .await;
            if ok {
                let message = page.status_text();
                page.clone().load_tree(true).await;
                page.show(Some(StatusMessage::ok(message)));
            }
        });
    }

    /// 上移／下移（跟上 Hayaku 1.5.4，C# 版沒有）：同一個上層裡跟前一頁或後一頁換位置；先確認再送。
    fn reorder(self: &Rc<Self>, up: bool) {
        let (space, node, target, personal) = {
            let st = self.st.borrow();
            let (Some(space), Some(node), Some(nodes)) = (st.space_api(), st.node.clone(), st.tree()) else {
                return;
            };
            let Some(target) = reorder_target(nodes, &node, up) else { return };
            (space, node, target, st.personal)
        };
        let (title, text, done) = reorder_texts(&node.title, &target.neighbor, up, personal);
        let page = self.clone();
        self.st.borrow_mut().confirm = Some(Box::new(move || {
            spawn(async move {
                let api = page.ctx.engine.api();
                let id = node.id.clone();
                let parent = node.parent_id.clone();
                let after = target.after.clone();
                let ok = inputs::run(
                    |_| {},
                    |m| page.show(m),
                    || async move {
                        api.move_space_page_after(&space, &id, parent.as_deref(), after.as_deref())
                            .await
                            .map_err(ActionError::from)
                    },
                    Some(&done),
                )
                .await;
                if ok {
                    let message = page.status_text();
                    // 重讀頁面樹後保留原本展開的節點與選取（C# 的重畫會收起來；這裡換位置要看得到結果）
                    let (expanded, selected) = {
                        let st = page.st.borrow();
                        (st.expanded.clone(), st.tree_selected.clone())
                    };
                    page.clone().load_tree(true).await;
                    {
                        let mut st = page.st.borrow_mut();
                        st.expanded.extend(expanded);
                        st.tree_selected = selected;
                    }
                    page.render_tree_rows();
                    page.render_order_buttons();
                    page.show(Some(StatusMessage::ok(message)));
                }
            });
        }));
        self.with(|g| {
            g.set_confirm_title(title.into());
            g.set_confirm_text(text.into());
            g.set_confirm_open(true);
        });
    }

    fn delete(self: &Rc<Self>) {
        let (space, node, children, files, personal) = {
            let st = self.st.borrow();
            let (Some(space), Some(node)) = (st.space_api(), st.node.clone()) else { return };
            let children =
                st.tree().map_or(0, |nodes| nodes.iter().filter(|n| n.parent_id.as_ref() == Some(&node.id)).count());
            let files = if node.is_folder() {
                st.node(Some(&node.id)).unwrap_or_else(|| node.clone()).file_count
            } else {
                Some(0)
            };
            (space, node, children, files, st.personal)
        };
        let text = delete_text(&node, children, files, personal);
        let title = if node.is_folder() { "刪除資料夾" } else { "刪除頁面" };
        let page = self.clone();
        self.st.borrow_mut().confirm = Some(Box::new(move || {
            spawn(async move {
                let api = page.ctx.engine.api();
                let id = node.id.clone();
                let ok = inputs::run(
                    |_| {},
                    |m| page.show(m),
                    || async move { api.delete_space_page(&space, &id).await.map_err(ActionError::from) },
                    Some(&format!("已刪除「{}」", node.title)),
                )
                .await;
                if ok {
                    let message = page.status_text();
                    page.st.borrow_mut().pages.remove(&node.id);
                    page.show_empty();
                    page.clone().load_tree(true).await;
                    page.show(Some(StatusMessage::ok(message)));
                }
            });
        }));
        self.with(|g| {
            g.set_confirm_title(title.into());
            g.set_confirm_text(text.into());
            g.set_confirm_open(true);
        });
    }
}

fn new_node(id: &str, title: &str, parent: Option<&str>, kind: Option<&str>, file_count: Option<i32>) -> SpaceNode {
    SpaceNode {
        id: id.to_owned(),
        title: title.to_owned(),
        parent_id: parent.map(str::to_owned),
        kind: kind.map(str::to_owned),
        updated_at: None,
        file_count,
        meeting: None,
    }
}

/// 下拉選單上的空間名稱。
pub(crate) fn space_text(s: &SpaceInfo) -> String {
    format!(
        "{}{}{}",
        s.project.as_ref().map(|p| format!("[{p}] ")).unwrap_or_default(),
        match s.page_count {
            Some(n) => format!("{}（{n} 頁）", s.name),
            None => s.name.clone(),
        },
        if s.archived { "・唯讀" } else { "" }
    )
}

pub(crate) fn space_tip(s: &SpaceInfo) -> String {
    [
        Some(s.key.clone()),
        s.role.as_ref().map(|r| format!("我的角色：{r}")),
        s.project.as_ref().map(|p| format!("專案 {p} 的空間（成員和權限跟著專案）")),
        s.archived.then(|| "專案已封存，只能看".to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("・")
}

fn tree_message(count: usize) -> String {
    if count == 0 {
        "這個空間還沒有頁面，按「＋ 新頁」新增".to_owned()
    } else {
        format!("{count} 頁")
    }
}

fn node_text(n: &SpaceNode) -> String {
    let count = match n.file_count {
        Some(c) if n.is_folder() && c > 0 => format!("（{c}）"),
        _ => String::new(),
    };
    format!("{}{}{count}", n.icon(), n.title)
}

fn node_tip(n: &SpaceNode) -> String {
    if n.is_meeting_folder() {
        return "會議資料夾（綁進空間的會議，最新 10 場；更早的點資料夾看）".to_owned();
    }
    if n.is_meeting_occurrence() {
        return "一場會議".to_owned();
    }
    if n.meeting.is_some() {
        return "會議的紀錄／白板，跟會議頁同一份".to_owned();
    }
    match n.file_count {
        Some(c) if n.is_folder() => format!("資料夾・直接放在裡面的檔案 {c} 個"),
        _ => String::new(),
    }
}

/// 會議資料夾的場次清單 → Markdown（時間＋名稱，新的在前，照伺服器順序）。
pub(crate) fn occurrences_markdown(list: &[hb_api::SpaceMeetingOccurrence]) -> String {
    list.iter()
        .map(|o| match &o.start_at {
            Some(at) => {
                let local = at.with_timezone(&chrono::Local);
                format!(
                    "- {}（{}）{}　{}",
                    local.format("%Y/%m/%d"),
                    inputs::weekday_short(local.date_naive()),
                    local.format("%H:%M"),
                    o.title
                )
            }
            None => format!("- {}", o.title),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 上層不在樹上的節點當最上層。
fn effective_parent<'a>(nodes: &[SpaceNode], n: &'a SpaceNode) -> Option<&'a str> {
    n.parent_id.as_deref().filter(|p| nodes.iter().any(|x| x.id == *p))
}

fn tree_roots(nodes: &[SpaceNode]) -> Vec<&SpaceNode> {
    nodes.iter().filter(|n| effective_parent(nodes, n).is_none()).collect()
}

/// 頁面樹攤平：(節點, 深度, 有沒有子節點)；只展開 expanded 裡的節點。
pub(crate) fn flatten_tree<'a>(
    nodes: &'a [SpaceNode],
    expanded: &HashSet<String>,
) -> Vec<(&'a SpaceNode, usize, bool)> {
    fn walk<'a>(
        nodes: &'a [SpaceNode],
        parent: Option<&str>,
        depth: usize,
        expanded: &HashSet<String>,
        seen: &mut HashSet<&'a str>,
        out: &mut Vec<(&'a SpaceNode, usize, bool)>,
    ) {
        for n in nodes.iter().filter(|n| effective_parent(nodes, n) == parent) {
            if !seen.insert(n.id.as_str()) {
                continue;
            }
            let has_children = nodes.iter().any(|c| effective_parent(nodes, c) == Some(n.id.as_str()));
            out.push((n, depth, has_children));
            if has_children && expanded.contains(&n.id) {
                walk(nodes, Some(&n.id), depth + 1, expanded, seen, out);
            }
        }
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    walk(nodes, None, 0, expanded, &mut seen, &mut out);
    out
}

/// 移動的選項（C# MoveAsync）：最上層＋不是自己或子頁、不是頻道或論壇的節點（依深度縮排）；第二個值是目前的上層。
pub(crate) fn move_choices(nodes: &[SpaceNode], node: &SpaceNode) -> (Vec<(Option<String>, String)>, Option<String>) {
    let mut blocked: HashSet<&str> = HashSet::from([node.id.as_str()]);
    loop {
        let mut added = false;
        for n in nodes {
            if let Some(p) = n.parent_id.as_deref()
                && blocked.contains(p)
                && blocked.insert(n.id.as_str())
            {
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    let by_id: HashMap<&str, &SpaceNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let depth = |n: &SpaceNode| {
        let mut d = 0;
        let mut seen = HashSet::from([n.id.as_str()]);
        let mut p = n.parent_id.as_deref().and_then(|p| by_id.get(p));
        while let Some(parent) = p {
            if !seen.insert(parent.id.as_str()) {
                break;
            }
            d += 1;
            p = parent.parent_id.as_deref().and_then(|x| by_id.get(x));
        }
        d
    };
    let mut choices = vec![(None, "（最上層）".to_owned())];
    choices.extend(
        nodes
            .iter()
            .filter(|n| !blocked.contains(n.id.as_str()))
            .filter(|n| !matches!(n.kind.as_deref().map(str::to_uppercase).as_deref(), Some("CHANNEL" | "FORUM")))
            // 綁進空間的會議（資料夾、場次、紀錄）不能放別的頁進去
            .filter(|n| !n.is_meeting())
            .map(|n| (Some(n.id.clone()), format!("{}{}", " ".repeat(depth(n) * 3), n.title))),
    );
    let current = node.parent_id.clone().filter(|p| by_id.contains_key(p.as_str()));
    (choices, current)
}

/// 上移／下移的目標：after＝排在哪一頁後面（None＝放最前面，送 afterId: null）、neighbor＝跟誰換位置。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReorderTarget {
    pub after: Option<String>,
    pub neighbor: String,
}

/// 同一個上層裡的上一頁／下一頁（照頁面樹的順序）；已經在最前（上移）或最後（下移）回 None。
pub(crate) fn reorder_target(nodes: &[SpaceNode], node: &SpaceNode, up: bool) -> Option<ReorderTarget> {
    let parent = effective_parent(nodes, node);
    let siblings: Vec<&SpaceNode> = nodes.iter().filter(|n| effective_parent(nodes, n) == parent).collect();
    let i = siblings.iter().position(|n| n.id == node.id)?;
    if up {
        let prev = siblings.get(i.checked_sub(1)?)?;
        let after = i.checked_sub(2).and_then(|j| siblings.get(j)).map(|n| n.id.clone());
        Some(ReorderTarget { after, neighbor: prev.title.clone() })
    } else {
        let next = siblings.get(i + 1)?;
        Some(ReorderTarget { after: Some(next.id.clone()), neighbor: next.title.clone() })
    }
}

/// 上移／下移的確認標題、內容與完成訊息；personal：我的空間（只有自己看得到，不用提醒）。
pub(crate) fn reorder_texts(title: &str, neighbor: &str, up: bool, personal: bool) -> (&'static str, String, String) {
    let warn = if personal { "" } else { "\n團隊空間是公司的正式文件，大家都看得到。" };
    if up {
        ("上移頁面", format!("把「{title}」移到「{neighbor}」前面？{warn}"), format!("已上移「{title}」"))
    } else {
        ("下移頁面", format!("把「{title}」移到「{neighbor}」後面？{warn}"), format!("已下移「{title}」"))
    }
}

/// 「與我分享」清單變成頁面樹的節點（都放最上層）；檔案的 kind 是 FILE。
fn shared_nodes(items: &[SharedItem]) -> Vec<SpaceNode> {
    items.iter().map(|s| new_node(&s.id, &s.title, None, s.kind.as_deref(), None)).collect()
}

/// 分享給我的那一筆的說明：誰分享的、可檢視或可編輯。
fn shared_meta(s: &SharedItem) -> String {
    let who = match &s.shared_by {
        Some(by) => format!("{by} 分享給你"),
        None => "分享給你的".to_owned(),
    };
    match s.can_edit {
        Some(true) => format!("{who}・可編輯"),
        Some(false) => format!("{who}・可檢視"),
        None => who,
    }
}

/// 頁名含 q（不分大小寫）的節點與所在路徑（上層名稱，照頁面樹的順序）。
pub(crate) fn filter_nodes<'a>(nodes: &'a [SpaceNode], q: &str) -> Vec<(&'a SpaceNode, String)> {
    let q = q.to_lowercase();
    let by_id: HashMap<&str, &SpaceNode> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    nodes
        .iter()
        .filter(|n| n.title.to_lowercase().contains(&q))
        .map(|n| {
            let mut path = Vec::new();
            let mut seen = HashSet::from([n.id.as_str()]);
            let mut p = n.parent_id.as_deref().and_then(|p| by_id.get(p));
            while let Some(parent) = p {
                if !seen.insert(parent.id.as_str()) {
                    break;
                }
                path.insert(0, parent.title.as_str());
                p = parent.parent_id.as_deref().and_then(|x| by_id.get(x));
            }
            (n, path.join(" › "))
        })
        .collect()
}

/// 刪除前的確認文字；personal：我的空間（Hayaku 1.6.0 起保留 30 天，在網頁「最近刪除」還原）。
pub(crate) fn delete_text(node: &SpaceNode, children: usize, files: Option<i32>, personal: bool) -> String {
    let mut text = format!("刪除「{}」？", node.title);
    if children > 0 {
        text.push_str("底下的子頁會一起刪除。");
    }
    if node.is_folder() {
        match files {
            Some(n) if n > 0 => text.push_str(&format!("資料夾裡的 {n} 個檔案也會一起刪除。")),
            _ => text.push_str("資料夾裡的檔案也會一起刪除。"),
        }
    }
    text.push_str(if personal {
        "\n30 天內可以在網頁的「最近刪除」還原。"
    } else {
        "\n30 天內空間管理員可以在網頁還原。"
    });
    text
}

fn file_icon(f: &SpaceFile) -> &'static str {
    if f.is_folder {
        return "📁 ";
    }
    match f.kind.as_deref() {
        None => "📄 ",
        Some("BOARD") => "🖍 ",
        Some("CHANNEL") => "# ",
        Some("FORUM") => "💬 ",
        _ => "📝 ",
    }
}

/// 資料夾頁的修改時間（跟上 Hayaku 1.5.5）：顯示到秒「2026/11/16 10:05:33」；with_weekday 時加星期（滑過的提示）。
pub(crate) fn modified_text(at: &hb_api::DateTimeOffset, with_weekday: bool) -> String {
    let local = at.with_timezone(&chrono::Local);
    if with_weekday {
        format!(
            "{}（{}）{}",
            local.format("%Y/%m/%d"),
            inputs::weekday_short(local.date_naive()),
            local.format("%H:%M:%S")
        )
    } else {
        local.format("%Y/%m/%d %H:%M:%S").to_string()
    }
}

pub(crate) fn file_meta(f: &SpaceFile) -> String {
    let kind = if f.is_folder {
        "資料夾"
    } else {
        match f.kind.as_deref() {
            None => "",
            Some("BOARD") => "白板",
            Some("CHANNEL") => "頻道",
            Some("FORUM") => "論壇",
            _ => "文件",
        }
    };
    [
        kind.to_owned(),
        size_text(f.size),
        f.uploader.clone().unwrap_or_default(),
        f.at.as_ref().map(|at| modified_text(at, false)).unwrap_or_default(),
        f.path.clone().unwrap_or_default(),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect::<Vec<_>>()
    .join("・")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn n(id: &str, parent: Option<&str>, kind: Option<&str>) -> SpaceNode {
        new_node(id, &id.to_uppercase(), parent, kind, None)
    }

    #[test]
    fn flatten_tree_只展開選的節點() {
        let nodes =
            vec![n("a", None, None), n("b", Some("a"), None), n("c", Some("b"), None), n("d", Some("zz"), None)];
        let expanded = HashSet::from(["a".to_owned()]);
        let flat: Vec<_> = flatten_tree(&nodes, &expanded).into_iter().map(|(n, d, c)| (n.id.clone(), d, c)).collect();
        // 上層不在樹上的 d 當最上層
        assert_eq!(vec![("a".into(), 0, true), ("b".into(), 1, true), ("d".into(), 0, false)], flat);
    }

    #[test]
    fn move_choices_排除自己子頁與頻道() {
        let nodes = vec![
            n("a", None, None),
            n("b", Some("a"), None),
            n("c", Some("b"), None),
            n("x", None, Some("channel")),
            n("y", Some("a"), Some("FOLDER")),
        ];
        let (choices, current) = move_choices(&nodes, &nodes[1]);
        let texts: Vec<_> = choices.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(vec!["（最上層）", "A", "   Y"], texts);
        assert_eq!(Some("a".to_owned()), current);
    }

    #[test]
    fn reorder_target_同一層跟前後一頁換位置() {
        let nodes = vec![
            n("a", None, None),
            n("a1", Some("a"), None),
            n("b", None, None),
            n("a2", Some("a"), None),
            n("c", None, None),
            n("a3", Some("a"), None),
            n("x", Some("zz"), None), // 上層不在樹上：當最上層
        ];
        let t = |i: usize, up: bool| reorder_target(&nodes, &nodes[i], up);
        // 最上層：a b c x
        assert_eq!(None, t(0, true));
        assert_eq!(Some(ReorderTarget { after: Some("b".into()), neighbor: "B".into() }), t(0, false));
        // b 上移：放最前面（afterId: null）
        assert_eq!(Some(ReorderTarget { after: None, neighbor: "A".into() }), t(2, true));
        // c 上移：排在 a 後面
        assert_eq!(Some(ReorderTarget { after: Some("a".into()), neighbor: "B".into() }), t(4, true));
        assert_eq!(None, t(6, false));
        // a 底下：a1 a2 a3
        assert_eq!(Some(ReorderTarget { after: Some("a1".into()), neighbor: "A2".into() }), t(5, true));
        assert_eq!(None, t(5, false));
        assert_eq!(Some(ReorderTarget { after: Some("a3".into()), neighbor: "A3".into() }), t(3, false));
    }

    #[test]
    fn reorder_texts_確認文字() {
        let (title, text, done) = reorder_texts("規章", "請假", true, false);
        assert_eq!("上移頁面", title);
        assert_eq!("把「規章」移到「請假」前面？\n團隊空間是公司的正式文件，大家都看得到。", text);
        assert_eq!("已上移「規章」", done);
        assert_eq!("已下移「規章」", reorder_texts("規章", "請假", false, false).2);
        // 我的空間不用提醒「大家都看得到」
        assert_eq!("把「規章」移到「請假」後面？", reorder_texts("規章", "請假", false, true).1);
    }

    #[test]
    fn filter_nodes_篩頁名帶路徑() {
        let nodes = vec![n("a", None, None), n("b", Some("a"), None), n("c", Some("b"), None), n("ab", None, None)];
        let titles = |q: &str| {
            filter_nodes(&nodes, q).into_iter().map(|(n, p)| format!("{}|{p}", n.title)).collect::<Vec<_>>()
        };
        assert_eq!(vec!["C|A › B"], titles("c"));
        // 不分大小寫
        assert_eq!(vec!["B|A", "AB|"], titles("b"));
        assert!(titles("zz").is_empty());
    }

    #[test]
    fn 與我分享_節點與說明() {
        let items = vec![
            SharedItem {
                id: "p".into(),
                title: "規格".into(),
                kind: Some("DOC".into()),
                is_file: false,
                shared_by: Some("Amy".into()),
                can_edit: Some(true),
            },
            SharedItem {
                id: "f".into(),
                title: "報價.xlsx".into(),
                kind: Some("FILE".into()),
                is_file: true,
                shared_by: None,
                can_edit: None,
            },
        ];
        let nodes = shared_nodes(&items);
        assert!(!nodes[0].is_file() && nodes[1].is_file());
        assert_eq!("📄 報價.xlsx", node_text(&nodes[1]));
        assert_eq!("Amy 分享給你・可編輯", shared_meta(&items[0]));
        assert_eq!("分享給你的", shared_meta(&items[1]));
        assert_eq!("Amy 分享給你・可檢視", shared_meta(&SharedItem { can_edit: Some(false), ..items[0].clone() }));
    }

    #[test]
    fn 模式_下拉選單與說明() {
        let space = |key: &str, shared: Vec<SharedItem>| SpaceInfo {
            key: key.into(),
            name: key.into(),
            role: None,
            page_count: Some(3),
            project: None,
            archived: false,
            shared,
        };
        let item = SharedItem {
            id: "p".into(),
            title: "規格".into(),
            kind: None,
            is_file: false,
            shared_by: None,
            can_edit: None,
        };
        let mut st = State { spaces: vec![space("me", vec![item]), space("RULES", Vec::new())], ..Default::default() };
        assert_eq!(vec!["RULES".to_owned()], st.visible_keys());
        assert_eq!("3 頁", st.tree_message(3));
        st.personal = true;
        assert_eq!(vec!["me".to_owned(), SHARED.to_owned()], st.visible_keys());
        assert_eq!("我的空間", st.key_text("me").0);
        assert_eq!("與我分享（1）", st.key_text(SHARED).0);
        st.counts = Some((2, 0));
        assert_eq!("3 頁・隨手記 2・待辦 0", st.tree_message(3));
        st.space_key = Some(SHARED.into());
        assert_eq!((Some("me".to_owned()), true), (st.space_api(), st.shared_view()));
        assert_eq!("還沒有人分享東西給你", st.tree_message(0));
    }

    #[test]
    fn delete_text_資料夾與子頁() {
        let folder = new_node("f", "文件", None, Some("FOLDER"), Some(3));
        assert_eq!(
            "刪除「文件」？底下的子頁會一起刪除。資料夾裡的 3 個檔案也會一起刪除。\n30 天內空間管理員可以在網頁還原。",
            delete_text(&folder, 2, Some(3), false)
        );
        assert_eq!(
            "刪除「文件」？資料夾裡的檔案也會一起刪除。\n30 天內空間管理員可以在網頁還原。",
            delete_text(&folder, 0, None, false)
        );
        let doc = n("d", None, None);
        assert_eq!("刪除「D」？\n30 天內空間管理員可以在網頁還原。", delete_text(&doc, 0, Some(0), false));
        assert_eq!("刪除「D」？\n30 天內可以在網頁的「最近刪除」還原。", delete_text(&doc, 0, Some(0), true));
    }

    #[test]
    fn space_text_與提示() {
        let s = SpaceInfo {
            key: "ENG".into(),
            name: "工程".into(),
            role: Some("ADMIN".into()),
            page_count: Some(12),
            project: Some("F2P".into()),
            archived: true,
            shared: Vec::new(),
        };
        assert_eq!("[F2P] 工程（12 頁）・唯讀", space_text(&s));
        assert_eq!("ENG・我的角色：ADMIN・專案 F2P 的空間（成員和權限跟著專案）・專案已封存，只能看", space_tip(&s));
    }

    #[test]
    fn file_meta_組合說明() {
        let f = SpaceFile {
            id: "1".into(),
            name: "a.png".into(),
            is_folder: false,
            size: Some(2048),
            uploader: Some("Amy".into()),
            at: None,
            kind: None,
            path: Some("/設計".into()),
        };
        assert_eq!("2 KB・Amy・/設計", file_meta(&f));
        let at = chrono::Local.with_ymd_and_hms(2026, 11, 16, 10, 5, 33).unwrap().fixed_offset();
        assert_eq!(modified_text(&at, false), "2026/11/16 10:05:33");
        assert_eq!(modified_text(&at, true), "2026/11/16（週一）10:05:33");
        assert_eq!("📄 ", file_icon(&f));
        let board = SpaceFile { kind: Some("BOARD".into()), size: None, uploader: None, path: None, ..f };
        assert_eq!("白板", file_meta(&board));
    }
}
