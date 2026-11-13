//! 我的筆記本、團隊空間、論壇、投票（全部唯讀以外的都是使用者在畫面上按了才送）。對應 C# `Mcp/HayakuApi.Knowledge.cs`。

use serde_json::{Map, Value, json};

use super::HayakuApi;
use crate::json::*;
use crate::models::*;
use crate::transport::McpError;

/// `PathText`：所在路徑：字串，或頁名陣列（字串或 { title }）。
fn path_text(f: &Value) -> Option<String> {
    if let Some(path) = first_str(f, &["path", "folderPath"]) {
        return Some(path.to_owned());
    }
    let parts: Vec<&str> = items(f, &["path"])
        .iter()
        .filter_map(|x| match x.as_str() {
            Some(s) => Some(s),
            None => first_str(x, &["title", "name"]),
        })
        .filter(|x| !is_blank(x))
        .collect();
    if parts.is_empty() { None } else { Some(parts.join(" › ")) }
}

/// `NewId`：{ id / pageId } 或 { page: { id } }。
fn new_id(r: &Value) -> Option<String> {
    if let Some(id) = first_string(r, &["id", "pageId"]) {
        return Some(id);
    }
    prop(r, "page").and_then(|p| first_string(p, &["id", "pageId"]))
}

fn forum_message_of(m: &Value) -> Option<ForumMessage> {
    let id = first_string(m, &["id"])?;
    Some(ForumMessage {
        id,
        author: person_name(m, &["author"]).unwrap_or_default(),
        body: first_string(m, &["body", "content"]).unwrap_or_default(),
        created_at: first_date(m, &["createdAt"]),
        deleted: str_(m, "deletedAt").is_some(),
    })
}

/// `ParseHits`。
fn parse_hits(r: &Value) -> Vec<KnowledgeHit> {
    items(r, &["items", "results", "pages"])
        .iter()
        .filter_map(|h| {
            Some(KnowledgeHit {
                id: first_string(h, &["pageId", "id"])?,
                title: first_string(h, &["title", "name"]).unwrap_or_else(|| "（未命名）".to_owned()),
                space: first_string(h, &["space", "spaceKey"]),
                snippet: first_string(h, &["snippet", "excerpt", "match", "preview"]),
                folder_id: None,
            })
        })
        .collect()
}

impl HayakuApi {
    // ───────────── 我的筆記本 ─────────────

    pub async fn list_notes(&self) -> Result<Notebook, McpError> {
        Ok(Self::parse_notebook(&self.call("list_my_notes", json!({})).await?))
    }

    pub async fn get_note(&self, page_id: &str) -> Result<NoteContent, McpError> {
        Ok(Self::parse_note(&self.call("get_my_note", json!({ "pageId": page_id })).await?, page_id))
    }

    pub async fn search_notes(&self, q: &str) -> Result<Vec<KnowledgeHit>, McpError> {
        Ok(parse_hits(&self.call("search_my_notes", json!({ "q": q })).await?))
    }

    /// 新增一頁；回傳新頁的 id（回應裡找不到就是 None）。
    pub async fn create_note(&self, title: &str, markdown: &str) -> Result<Option<String>, McpError> {
        Ok(new_id(&self.call("create_my_note", json!({ "title": title, "markdown": markdown })).await?))
    }

    pub async fn append_note(&self, page_id: &str, markdown: &str) -> Result<(), McpError> {
        self.call("append_to_my_note", json!({ "pageId": page_id, "markdown": markdown })).await.map(drop)
    }

    pub fn parse_notebook(r: &Value) -> Notebook {
        let folders = items(r, &["folders"])
            .iter()
            .filter_map(|f| {
                Some(NoteFolder {
                    id: first_string(f, &["id"])?,
                    name: first_string(f, &["name", "title"]).unwrap_or_else(|| "（未命名資料夾）".to_owned()),
                })
            })
            .collect();
        let pages = items(r, &["pages", "items", "notes"])
            .iter()
            .filter_map(|p| {
                Some(NotePage {
                    id: first_string(p, &["id", "pageId"])?,
                    title: first_string(p, &["title", "name"]).unwrap_or_else(|| "（未命名）".to_owned()),
                    folder_id: first_string(p, &["folderId"]),
                    updated_at: first_date(p, &["updatedAt", "editedAt", "createdAt"]),
                    task_codes: task_codes(p),
                })
            })
            .collect();
        let counts = prop(r, "counts").unwrap_or(&NULL);
        Notebook { folders, pages, inbox: count(counts, "inbox"), todos: count(counts, "todos") }
    }

    pub fn parse_note(r: &Value, page_id: &str) -> NoteContent {
        let page = if obj(r, "page").is_object() { obj(r, "page") } else { r };
        let todos = items(r, &["todos"])
            .iter()
            .map(|t| {
                (
                    first_string(t, &["text", "title", "content"]).unwrap_or_default(),
                    bool_(t, "done") || bool_(t, "checked") || bool_(t, "completed"),
                )
            })
            .filter(|(text, _)| !text.is_empty())
            .collect();
        let mut codes: Vec<String> = Vec::new();
        for c in task_codes(page).into_iter().chain(task_codes(r)) {
            if !codes.contains(&c) {
                codes.push(c);
            }
        }
        NoteContent {
            id: first_string(page, &["id", "pageId"]).unwrap_or_else(|| page_id.to_owned()),
            title: first_string(page, &["title", "name"]).unwrap_or_else(|| "（未命名）".to_owned()),
            markdown: first_string(page, &["markdown", "content", "body"]).unwrap_or_default(),
            task_codes: codes,
            todos,
        }
    }

    // ───────────── 團隊空間 ─────────────

    pub async fn list_spaces(&self) -> Result<Vec<SpaceInfo>, McpError> {
        Ok(Self::parse_spaces(&self.call("list_spaces", json!({})).await?))
    }

    pub async fn space_tree(&self, space: &str) -> Result<Vec<SpaceNode>, McpError> {
        Ok(Self::parse_space_tree(&self.call("get_space_tree", json!({ "space": space })).await?))
    }

    pub async fn get_space_page(&self, space: &str, page_id: &str) -> Result<SpacePageContent, McpError> {
        Ok(Self::parse_space_page(
            &self.call("get_space_page", json!({ "space": space, "pageId": page_id })).await?,
            page_id,
        ))
    }

    /// space 為 None 時搜全部看得到的空間；project：只搜那個專案的空間（C# 預設 null）。
    pub async fn search_spaces(
        &self,
        q: &str,
        space: Option<&str>,
        project: Option<&str>,
    ) -> Result<Vec<KnowledgeHit>, McpError> {
        let mut args = Map::new();
        args.insert("q".into(), json!(q));
        if let Some(s) = space {
            args.insert("space".into(), json!(s));
        }
        if let Some(p) = project {
            args.insert("project".into(), json!(p));
        }
        let r = self.call("search_spaces", Value::Object(args)).await?;
        let mut hits = parse_hits(&r);
        hits.extend(Self::parse_file_hits(&r));
        Ok(hits)
    }

    /// 新增一頁（parent_id 為 None 放最上層；上層可以是文件、白板或資料夾）；folder＝建資料夾（沒有內文，C# 預設 false）。
    /// 回傳新頁的 id（回應裡找不到就是 None）。
    pub async fn create_space_page(
        &self,
        space: &str,
        parent_id: Option<&str>,
        title: &str,
        markdown: &str,
        folder: bool,
    ) -> Result<Option<String>, McpError> {
        let mut args = Map::new();
        args.insert("space".into(), json!(space));
        args.insert("title".into(), json!(title));
        if folder {
            args.insert("kind".into(), json!("FOLDER"));
        } else {
            args.insert("markdown".into(), json!(markdown));
        }
        if let Some(p) = parent_id {
            args.insert("parentId".into(), json!(p));
        }
        Ok(new_id(&self.call("create_space_page", Value::Object(args)).await?))
    }

    pub async fn append_space_page(&self, space: &str, page_id: &str, markdown: &str) -> Result<(), McpError> {
        self.call("append_to_space_page", json!({ "space": space, "pageId": page_id, "markdown": markdown }))
            .await
            .map(drop)
    }

    /// 改一頁：title、markdown 為 None 的不改；整頁換掉前伺服器會自動存一版。
    pub async fn update_space_page(
        &self,
        space: &str,
        page_id: &str,
        title: Option<&str>,
        markdown: Option<&str>,
    ) -> Result<(), McpError> {
        let mut args = Map::new();
        args.insert("space".into(), json!(space));
        args.insert("pageId".into(), json!(page_id));
        if let Some(t) = title {
            args.insert("title".into(), json!(t));
        }
        if let Some(m) = markdown {
            args.insert("markdown".into(), json!(m));
        }
        self.call("update_space_page", Value::Object(args)).await.map(drop)
    }

    /// 移到同一個空間的別的上層（parent_id 為 None＝最上層），排在最後。
    pub async fn move_space_page(&self, space: &str, page_id: &str, parent_id: Option<&str>) -> Result<(), McpError> {
        self.call("move_space_page", json!({ "space": space, "pageId": page_id, "parentId": parent_id }))
            .await
            .map(drop)
    }

    /// 移到同一個空間的上層 parent_id 底下、排在 after_id 那頁後面（None＝放最前面）。
    /// 跟上 Hayaku 1.5.4（move_space_page 多了 afterId：null 放最前面、不給放最後）；C# 版沒有。
    pub async fn move_space_page_after(
        &self,
        space: &str,
        page_id: &str,
        parent_id: Option<&str>,
        after_id: Option<&str>,
    ) -> Result<(), McpError> {
        self.call(
            "move_space_page",
            json!({ "space": space, "pageId": page_id, "parentId": parent_id, "afterId": after_id }),
        )
        .await
        .map(drop)
    }

    /// 刪除一頁（連子頁）；30 天內空間管理員可以在網頁還原。
    pub async fn delete_space_page(&self, space: &str, page_id: &str) -> Result<(), McpError> {
        self.call("delete_space_page", json!({ "space": space, "pageId": page_id })).await.map(drop)
    }

    /// 資料夾頁（kind 是 FOLDER，舊的檔案庫也是）裡的子頁與上傳的檔案。
    pub async fn list_space_files(&self, space: &str, folder: &str) -> Result<Vec<SpaceFile>, McpError> {
        Ok(Self::parse_space_files(
            &self.call("list_space_files", json!({ "spaceKey": space, "folder": folder })).await?,
        ))
    }

    /// 下載連結（1 小時有效，只有本人能用）。
    pub async fn space_file_link(&self, file_id: &str) -> Result<Option<String>, McpError> {
        let r = self.call("get_space_file_link", json!({ "fileId": file_id })).await?;
        Ok(match r.as_str() {
            Some(s) => Some(s.to_owned()),
            None => first_string(&r, &["url", "link", "downloadUrl", "href"]),
        })
    }

    pub fn parse_spaces(r: &Value) -> Vec<SpaceInfo> {
        items(r, &["spaces", "items"])
            .iter()
            .filter_map(|s| {
                let key = first_string(s, &["key", "code"])?;
                Some(SpaceInfo {
                    name: first_string(s, &["name", "title"]).unwrap_or_else(|| key.clone()),
                    key,
                    role: first_string(s, &["role", "myRole"]),
                    page_count: number(s, &["pageCount", "pages"]),
                    project: match prop(s, "project") {
                        Some(p @ Value::Object(_)) => first_string(p, &["key", "code"]),
                        _ => first_string(s, &["project", "projectKey"]),
                    },
                    archived: bool_(s, "archived") || bool_(s, "isArchived") || bool_(s, "readOnly"),
                })
            })
            .collect()
    }

    /// 頁面樹：平的清單（parentId）或巢狀（children）都接受，攤平成照樹狀順序的清單。
    pub fn parse_space_tree(r: &Value) -> Vec<SpaceNode> {
        fn walk(list: &mut Vec<SpaceNode>, nodes: &[Value], parent: Option<&str>) {
            for n in nodes {
                let Some(id) = first_string(n, &["id", "pageId"]) else { continue };
                list.push(SpaceNode {
                    id: id.clone(),
                    title: first_string(n, &["title", "name"]).unwrap_or_else(|| "（未命名）".to_owned()),
                    parent_id: first_string(n, &["parentId"]).or_else(|| parent.map(str::to_owned)),
                    kind: first_string(n, &["kind", "type"]),
                    updated_at: first_date(n, &["updatedAt", "editedAt"]),
                    file_count: number(n, &["fileCount", "files"]),
                });
                walk(list, items(n, &["children"]), Some(&id));
            }
        }
        let mut list = Vec::new();
        walk(&mut list, items(r, &["pages", "tree", "nodes", "items"]), None);
        list
    }

    pub fn parse_space_page(r: &Value, page_id: &str) -> SpacePageContent {
        let page = if obj(r, "page").is_object() { obj(r, "page") } else { r };
        let owner = match prop(page, "owner") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(o) => person(o).map(|p| p.display_name).or_else(|| first_string(o, &["name"])),
            None => None,
        };
        let path = items(page, &["path", "breadcrumbs"])
            .iter()
            .filter_map(|x| match x.as_str() {
                Some(s) => Some(s),
                None => first_str(x, &["title", "name"]),
            })
            .filter(|x| !is_blank(x))
            .map(str::to_owned)
            .collect();
        SpacePageContent {
            id: first_string(page, &["id", "pageId"]).unwrap_or_else(|| page_id.to_owned()),
            title: first_string(page, &["title", "name"]).unwrap_or_else(|| "（未命名）".to_owned()),
            markdown: first_string(page, &["markdown", "content", "body"]).unwrap_or_default(),
            owner,
            updated_at: first_date(page, &["updatedAt", "editedAt"]),
            path,
        }
    }

    /// 資料夾裡的子頁（pages／children；舊的檔案庫回 folders）與檔案（files）；子頁沒寫種類的當資料夾。
    /// 實際回應還沒有樣本（帳號看不到任何空間），照工具說明寬鬆讀。
    pub fn parse_space_files(r: &Value) -> Vec<SpaceFile> {
        let mut pages: Vec<SpaceFile> = Vec::new();
        let page_sources = ["pages", "children", "subpages", "folders"]
            .iter()
            .flat_map(|name| items(r, &[name]).iter())
            // 子頁和檔案混在同一個陣列時，有 kind 的是子頁
            .chain(items(r, &["files", "items"]).iter().filter(|f| first_str(f, &["kind"]).is_some()));
        for f in page_sources {
            let Some(id) = first_string(f, &["id", "pageId"]) else { continue };
            if pages.iter().any(|p| p.id == id) {
                continue; // DistinctBy(Id)
            }
            let kind = first_str(f, &["kind", "type"]).unwrap_or("FOLDER").to_uppercase();
            let folder = kind == "FOLDER" || kind == "FILES";
            pages.push(SpaceFile {
                id,
                name: first_string(f, &["title", "name"])
                    .unwrap_or_else(|| if folder { "（未命名資料夾）" } else { "（未命名）" }.to_owned()),
                is_folder: folder,
                size: None,
                uploader: None,
                at: first_date(f, &["updatedAt", "createdAt"]),
                kind: Some(kind),
                path: path_text(f),
            });
        }
        // OrderByDescending(IsFolder)：資料夾在前，同組維持原順序
        pages.sort_by_key(|p| !p.is_folder);
        let files =
            items(r, &["files", "items"]).iter().filter(|f| first_str(f, &["kind"]).is_none()).filter_map(|f| {
                Some(SpaceFile {
                    id: first_string(f, &["id", "fileId"])?,
                    name: first_string(f, &["name", "fileName", "title"]).unwrap_or_else(|| "（未命名）".to_owned()),
                    is_folder: false,
                    size: number(f, &["size", "bytes"]).map(i64::from),
                    uploader: person_name(f, &["uploader", "uploadedBy", "createdBy"]),
                    at: first_date(f, &["createdAt", "uploadedAt", "updatedAt"]),
                    kind: None,
                    path: path_text(f),
                })
            });
        pages.extend(files);
        pages
    }

    /// search_spaces 的 files：資料夾裡檔名符合的檔案（帶所在資料夾 folderId 與路徑 path）。
    pub fn parse_file_hits(r: &Value) -> Vec<KnowledgeHit> {
        items(r, &["files"])
            .iter()
            .filter_map(|f| {
                let id = first_string(f, &["id", "fileId"])?;
                let folder = first_string(f, &["folderId", "pageId", "folder"])?;
                Some(KnowledgeHit {
                    id,
                    title: first_string(f, &["name", "fileName", "title"]).unwrap_or_else(|| "（未命名）".to_owned()),
                    space: first_string(f, &["space", "spaceKey"]),
                    snippet: path_text(f),
                    folder_id: Some(folder),
                })
            })
            .collect()
    }

    // ───────────── 論壇 ─────────────

    /// project＋channel（論壇名稱）的貼文；置頂的在最上面，其他照最新活動。頻道不是論壇時伺服器回「找不到這個論壇」。
    pub async fn list_forum_posts(
        &self,
        project: &str,
        channel: &str,
        unsolved: bool,
    ) -> Result<Vec<ForumPost>, McpError> {
        let mut args = Map::new();
        args.insert("project".into(), json!(project));
        args.insert("channel".into(), json!(channel));
        if unsolved {
            args.insert("unsolved".into(), json!(true));
        }
        Ok(Self::parse_forum_posts(&self.call("list_forum_posts", Value::Object(args)).await?))
    }

    pub async fn get_forum_post(&self, project: &str, channel: &str, post: &str) -> Result<ForumThread, McpError> {
        Ok(Self::parse_forum_thread(
            &self.call("get_forum_post", json!({ "project": project, "channel": channel, "post": post })).await?,
        ))
    }

    pub async fn create_forum_post(
        &self,
        project: &str,
        channel: &str,
        title: &str,
        body: &str,
    ) -> Result<(), McpError> {
        self.call("create_forum_post", json!({ "project": project, "channel": channel, "title": title, "body": body }))
            .await
            .map(drop)
    }

    pub async fn reply_forum_post(&self, project: &str, channel: &str, post: &str, body: &str) -> Result<(), McpError> {
        self.call("reply_forum_post", json!({ "project": project, "channel": channel, "post": post, "body": body }))
            .await
            .map(drop)
    }

    pub fn parse_forum_posts(r: &Value) -> Vec<ForumPost> {
        items(r, &["posts", "items"])
            .iter()
            .filter_map(|p| {
                Some(ForumPost {
                    id: first_string(p, &["id"])?,
                    title: first_string(p, &["title"]).unwrap_or_else(|| "（沒有標題）".to_owned()),
                    author: person_name(p, &["author"]).unwrap_or_default(),
                    created_at: first_date(p, &["createdAt"]),
                    replies: number(p, &["replies", "replyCount"]).unwrap_or(0),
                    last_activity_at: first_date(p, &["lastActivityAt", "updatedAt"]),
                    tags: tags(p),
                    pinned: bool_(p, "pinned"),
                    solved: bool_(p, "solved"),
                })
            })
            .collect()
    }

    pub fn parse_forum_thread(r: &Value) -> ForumThread {
        let thread = obj(r, "thread");
        let root = obj(thread, "root");
        let forum = obj(root, "forum");
        let mut replies: Vec<ForumMessage> =
            items(r, &["items", "replies"]).iter().filter_map(forum_message_of).collect();
        let min = min_value();
        replies.sort_by_key(|a| a.created_at.unwrap_or(min));
        let solved = matches!(prop(forum, "solvedAt"), Some(Value::String(_)));
        ForumThread {
            title: first_string(forum, &["title"]).or_else(|| first_string(root, &["title"])).unwrap_or_default(),
            root: forum_message_of(root),
            replies,
            solved,
            tags: tags(forum),
        }
    }

    // ───────────── 投票 ─────────────

    pub async fn get_poll(&self, poll: &str) -> Result<PollInfo, McpError> {
        Ok(Self::parse_poll(&self.call("get_poll", json!({ "poll": poll })).await?))
    }

    pub fn parse_poll(r: &Value) -> PollInfo {
        PollInfo {
            question: first_string(r, &["question", "title"]).unwrap_or_else(|| "（沒有題目）".to_owned()),
            place: first_string(r, &["place"]),
            r#type: first_string(r, &["type"]),
            anonymous: bool_(r, "anonymous"),
            status: first_string(r, &["status"]),
            closes_at: first_date(r, &["closesAt"]),
            created_by: person_name(r, &["createdBy"]),
            options: items(r, &["options"])
                .iter()
                .map(|o| PollOption {
                    label: first_string(o, &["label", "text"]).unwrap_or_default(),
                    votes: number(o, &["votes"]),
                    voters: items(o, &["voters"])
                        .iter()
                        .filter_map(|v| match v.as_str() {
                            Some(s) => Some(s.to_owned()),
                            None => person(v).map(|p| p.display_name).or_else(|| first_string(v, &["name"])),
                        })
                        .filter(|v| !is_blank(v))
                        .collect(),
                    mine: bool_(o, "mine"),
                })
                .collect(),
            voted: number(r, &["votedCount"]),
            pending: number(r, &["pendingCount"]),
        }
    }

    /// C# `ParseKnowledgeHits`（筆記本、空間搜尋結果）。
    pub fn parse_knowledge_hits(r: &Value) -> Vec<KnowledgeHit> {
        parse_hits(r)
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::*;
    use super::*;

    // 對應 KnowledgeTests

    #[test]
    fn 空間頁面樹_巢狀攤平成樹狀順序() {
        let nodes = HayakuApi::parse_space_tree(&j(r#"
            {"pages":[{"id":"a","title":"首頁","children":[{"id":"b","title":"規範"},{"id":"c","title":"檔案","kind":"FILES"}]},{"id":"d","title":"其他"}]}"#));
        assert_eq!(nodes.iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c", "d"]);
        assert_eq!(nodes[1].parent_id.as_deref(), Some("a"));
        assert!(nodes[2].is_folder()); // 舊的檔案庫（FILES）當資料夾
        assert_eq!(nodes[3].parent_id, None);
    }

    #[test]
    fn 論壇貼文與回覆() {
        let posts = HayakuApi::parse_forum_posts(&j(r#"
            {"posts":[{"id":"p1","title":"列印失敗","author":{"username":"a","displayName":"小明"},"replyCount":2,"pinned":true,"tags":["bug"]}]}"#));
        assert_eq!(
            (
                posts[0].id.as_str(),
                posts[0].title.as_str(),
                posts[0].author.as_str(),
                posts[0].replies,
                posts[0].pinned
            ),
            ("p1", "列印失敗", "小明", 2, true)
        );
        assert_eq!(posts[0].tags, vec!["bug"]);

        let thread = HayakuApi::parse_forum_thread(&j(r#"
            {"thread":{"root":{"id":"p1","body":"請看","author":"小明","forum":{"title":"列印失敗","solvedAt":"2026-10-30T01:00:00Z"}}},
             "items":[{"id":"r2","body":"後","createdAt":"2026-10-30T02:00:00Z"},{"id":"r1","body":"先","createdAt":"2026-10-30T01:00:00Z"}]}"#));
        assert_eq!(thread.title, "列印失敗");
        assert!(thread.solved);
        assert_eq!(thread.root.as_ref().map(|r| r.body.as_str()), Some("請看"));
        assert_eq!(thread.replies.iter().map(|r| r.body.as_str()).collect::<Vec<_>>(), vec!["先", "後"]);
    }

    #[test]
    fn 筆記本() {
        let book = HayakuApi::parse_notebook(&j(r#"
            {"folders":[{"id":"f1","name":"會議"}],"pages":[{"id":"n1","title":"週會","folderId":"f1","taskCodes":["F2P-1"]}],"counts":{"inbox":3,"todos":2}}"#));
        assert_eq!(
            (book.pages[0].id.as_str(), book.pages[0].title.as_str(), book.pages[0].folder_id.as_deref()),
            ("n1", "週會", Some("f1"))
        );
        assert_eq!(book.pages[0].task_codes, vec!["F2P-1"]);
        assert_eq!((book.inbox, book.todos), (3, 2));
    }

    // 對應 Hayaku14Tests 的空間部分

    #[test]
    fn 空間頁面樹_資料夾與檔案數() {
        let nodes = HayakuApi::parse_space_tree(&j(r#"
            {"pages":[{"id":"f","title":"設計稿","kind":"FOLDER","fileCount":4,"children":[{"id":"d","title":"說明","kind":"DOC"}]},
                      {"id":"b","title":"白板","kind":"BOARD"}]}"#));
        assert!(nodes[0].is_folder());
        assert_eq!(nodes[0].file_count, Some(4));
        assert!(!nodes[1].is_folder());
        assert_eq!(nodes[1].parent_id.as_deref(), Some("f"));
        assert_eq!(nodes[2].icon(), "🖍 ");
    }

    #[test]
    fn 資料夾內容_子頁在前_檔案在後() {
        let items = HayakuApi::parse_space_files(&j(r#"
            {"pages":[{"id":"d","title":"說明","kind":"DOC"},{"id":"s","title":"子資料夾","kind":"FOLDER"}],
             "files":[{"id":"x","name":"a.pdf","size":1024,"uploader":{"displayName":"小明"},"path":"設計稿 › 子資料夾"}]}"#));
        assert_eq!(items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["s", "d", "x"]);
        assert!(items[0].is_folder);
        assert!(items[1].is_page());
        assert!(!items[1].is_folder);
        assert!(!items[2].is_page());
        assert_eq!(items[2].path.as_deref(), Some("設計稿 › 子資料夾"));

        // 舊的檔案庫回應（folders＋files）照樣讀得到
        let old = HayakuApi::parse_space_files(&j(
            r#"{"folders":[{"id":"o","name":"舊資料夾"}],"files":[{"id":"y","name":"b.txt"}]}"#,
        ));
        assert_eq!(old.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["o", "y"]);
        assert!(old[0].is_folder);
    }

    #[test]
    fn 搜尋空間_檔案結果帶所在資料夾() {
        let hits = HayakuApi::parse_file_hits(&j(r#"
            {"results":[{"pageId":"p","title":"規範"}],"files":[{"id":"x","name":"報價.xlsx","folderId":"f","path":["業務","報價"]}]}"#));
        assert_eq!(hits.len(), 1);
        let hit = &hits[0];
        assert!(hit.is_file());
        assert_eq!(
            (hit.id.as_str(), hit.folder_id.as_deref(), hit.snippet.as_deref()),
            ("x", Some("f"), Some("業務 › 報價"))
        );
    }

    // 對應 TestingTests.ParseSpaces_讀所屬專案與封存
    #[test]
    fn parse_spaces_讀所屬專案與封存() {
        let spaces = HayakuApi::parse_spaces(&j(r#"
            {"spaces":[{"key":"RULES","name":"公司規章","role":"EDITOR","pageCount":12},
                       {"key":"F2PDOC","name":"F2P 文件","role":"MEMBER","pageCount":3,"project":{"key":"F2P","name":"F2P"},"archived":true}]}"#));
        assert_eq!(spaces[0].project, None);
        assert_eq!((spaces[1].project.as_deref(), spaces[1].archived), (Some("F2P"), true));
    }

    // 對應 NewToolsTests 的投票部分

    #[test]
    fn parse_poll_讀選項票數投票的人與狀態() {
        let poll = HayakuApi::parse_poll(&j(r#"
            {"id":"0a793a8b-b232-4ed0-9fe0-e3aa7dd3d373","place":"聊天","question":"午餐吃什麼","type":"單選","anonymous":false,"status":"進行中",
             "closesAt":null,"closedAt":null,"createdBy":"羅瑞騰","resultsVisible":true,
             "options":[{"label":"便當","votes":3,"voters":["丁鈺昕","羅瑞騰","測試人員"],"mine":false},
                        {"label":"麵","votes":0,"voters":[],"mine":true}],
             "votedCount":3,"pendingCount":5,"timezone":"Asia/Taipei"}"#));
        assert_eq!(
            (
                poll.question.as_str(),
                poll.r#type.as_deref(),
                poll.status.as_deref(),
                poll.created_by.as_deref(),
                poll.voted,
                poll.pending
            ),
            ("午餐吃什麼", Some("單選"), Some("進行中"), Some("羅瑞騰"), Some(3), Some(5))
        );
        assert_eq!(poll.closes_at, None);
        assert_eq!((poll.options[0].votes, poll.options[0].voters.len(), poll.options[0].mine), (Some(3), 3, false));
        assert!(poll.options[1].mine);
    }

    #[test]
    fn move_space_page_原本排最後_after_id_換位置() {
        use std::sync::Arc;
        let fake = Arc::new(FakeTransport::default());
        let api = HayakuApi::new(fake.clone());
        block_on(api.move_space_page("RULES", "p1", Some("p0"))).expect("move");
        block_on(api.move_space_page_after("RULES", "p1", Some("p0"), Some("p2"))).expect("move");
        block_on(api.move_space_page_after("RULES", "p1", None, None)).expect("move");
        let calls = fake.calls.lock().expect("lock");
        // 原本的呼叫不送 afterId（排在最後）
        assert_eq!(calls[0].1, json!({ "space": "RULES", "pageId": "p1", "parentId": "p0" }));
        assert_eq!(calls[1].1, json!({ "space": "RULES", "pageId": "p1", "parentId": "p0", "afterId": "p2" }));
        // afterId: null＝放最前面
        assert_eq!(calls[2].1, json!({ "space": "RULES", "pageId": "p1", "parentId": null, "afterId": null }));
    }

    #[test]
    fn parse_poll_投了才看得到結果時票數是null() {
        let poll =
            HayakuApi::parse_poll(&j(r#"{"question":"Q","options":[{"label":"A","mine":false}],"votedCount":null}"#));
        assert_eq!(poll.options[0].votes, None);
        assert_eq!(poll.voted, None);
    }
}
