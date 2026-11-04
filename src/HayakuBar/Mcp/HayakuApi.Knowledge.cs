using System.Text.Json;

namespace HayakuBar.Mcp;

/// <summary>我的筆記本裡的資料夾。</summary>
public sealed record NoteFolder(string Id, string Name);

/// <summary>我的筆記本的一頁（清單用）；TaskCodes：相關任務編號。</summary>
public sealed record NotePage(string Id, string Title, string? FolderId, DateTimeOffset? UpdatedAt, IReadOnlyList<string> TaskCodes);

/// <summary>筆記本：資料夾、頁、隨手記與待辦的數字（list_my_notes）。</summary>
public sealed record Notebook(IReadOnlyList<NoteFolder> Folders, IReadOnlyList<NotePage> Pages, int Inbox, int Todos);

/// <summary>筆記本的一頁內容（get_my_note）；Todos：這頁的待辦與勾了沒。</summary>
public sealed record NoteContent(string Id, string Title, string Markdown, IReadOnlyList<string> TaskCodes,
    IReadOnlyList<(string Text, bool Done)> Todos);

/// <summary>
/// 筆記本、團隊空間的搜尋結果；Space 只有團隊空間才有。
/// FolderId 有值是資料夾裡的檔案（search_spaces 的 files：Id 是檔案 id，Snippet 是所在路徑）。
/// </summary>
public sealed record KnowledgeHit(string Id, string Title, string? Space, string? Snippet, string? FolderId = null)
{
    public bool IsFile => FolderId is not null;
}

/// <summary>團隊空間（list_spaces）；Project：所屬專案的代碼（專案裡開的空間），Archived：專案封存了、只能看。</summary>
public sealed record SpaceInfo(string Key, string Name, string? Role, int? PageCount, string? Project = null, bool Archived = false);

/// <summary>
/// 空間頁面樹的一個節點（get_space_tree）；Kind：DOC 文件、BOARD 白板、FOLDER 資料夾、CHANNEL 頻道、FORUM 論壇
/// （Hayaku 1.4 以前的檔案庫是 FILES，已經轉成資料夾、id 不變）。FileCount：資料夾直接放在裡面的檔案數。
/// </summary>
public sealed record SpaceNode(string Id, string Title, string? ParentId, string? Kind, DateTimeOffset? UpdatedAt, int? FileCount = null)
{
    public bool IsFolder => Kind?.ToUpperInvariant() is "FOLDER" or "FILES";

    /// <summary>頁面樹上的圖示：資料夾、白板、頻道、論壇；文件不加。</summary>
    public string Icon => Kind?.ToUpperInvariant() switch
    {
        "FOLDER" or "FILES" => "📁 ",
        "BOARD" => "🖍 ",
        "CHANNEL" => "# ",
        "FORUM" => "💬 ",
        _ => "",
    };
}

/// <summary>空間的一頁（get_space_page）；Path：上層的頁名。</summary>
public sealed record SpacePageContent(string Id, string Title, string Markdown, string? Owner, DateTimeOffset? UpdatedAt,
    IReadOnlyList<string> Path);

/// <summary>
/// 資料夾裡的一個東西（list_space_files）：上傳的檔案，或子頁（Kind：FOLDER 子資料夾、DOC 文件、BOARD 白板…）。
/// Path：遞迴列出時的所在路徑。
/// </summary>
public sealed record SpaceFile(string Id, string Name, bool IsFolder, long? Size, string? Uploader, DateTimeOffset? At,
    string? Kind = null, string? Path = null)
{
    /// <summary>子頁（不是上傳的檔案）。</summary>
    public bool IsPage => Kind is not null;
}

/// <summary>投票的一個選項；Voters 匿名時是空的，Mine：我投了這個。</summary>
public sealed record PollOption(string Label, int? Votes, IReadOnlyList<string> Voters, bool Mine);

/// <summary>投票（get_poll，唯讀）；Votes 是 null 表示「投了才看得到結果」而我還沒投。</summary>
public sealed record PollInfo(string Question, string? Place, string? Type, bool Anonymous, string? Status, DateTimeOffset? ClosesAt,
    string? CreatedBy, IReadOnlyList<PollOption> Options, int? Voted, int? Pending);

/// <summary>論壇貼文（list_forum_posts）。</summary>
public sealed record ForumPost(string Id, string Title, string Author, DateTimeOffset? CreatedAt, int Replies,
    DateTimeOffset? LastActivityAt, IReadOnlyList<string> Tags, bool Pinned, bool Solved);

public sealed record ForumMessage(string Id, string Author, string Body, DateTimeOffset? CreatedAt, bool Deleted);

/// <summary>一篇貼文與底下的回應（get_forum_post）。</summary>
public sealed record ForumThread(string Title, ForumMessage? Root, IReadOnlyList<ForumMessage> Replies, bool Solved, IReadOnlyList<string> Tags);

/// <summary>我的筆記本、團隊空間、論壇（全部唯讀以外的都是使用者在畫面上按了才送）。</summary>
public sealed partial class HayakuApi
{
    // ───────────── 我的筆記本 ─────────────

    public async Task<Notebook> ListNotesAsync(CancellationToken ct) =>
        ParseNotebook(await Mcp.CallToolAsync("list_my_notes", null, ct));

    public async Task<NoteContent> GetNoteAsync(string pageId, CancellationToken ct) =>
        ParseNote(await Mcp.CallToolAsync("get_my_note", new { pageId }, ct), pageId);

    public async Task<List<KnowledgeHit>> SearchNotesAsync(string q, CancellationToken ct) =>
        ParseHits(await Mcp.CallToolAsync("search_my_notes", new { q }, ct));

    /// <summary>新增一頁；回傳新頁的 id（回應裡找不到就是 null）。</summary>
    public async Task<string?> CreateNoteAsync(string title, string markdown, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("create_my_note", new { title, markdown }, ct);
        return NewId(r);
    }

    public Task AppendNoteAsync(string pageId, string markdown, CancellationToken ct) =>
        Mcp.CallToolAsync("append_to_my_note", new { pageId, markdown }, ct);

    internal static Notebook ParseNotebook(JsonElement r)
    {
        var folders = Items(r, "folders")
            .Where(f => FirstStr(f, "id") is not null)
            .Select(f => new NoteFolder(FirstStr(f, "id")!, FirstStr(f, "name", "title") ?? "（未命名資料夾）"))
            .ToList();
        var pages = Items(r, "pages", "items", "notes")
            .Where(p => FirstStr(p, "id", "pageId") is not null)
            .Select(p => new NotePage(FirstStr(p, "id", "pageId")!, FirstStr(p, "title", "name") ?? "（未命名）", FirstStr(p, "folderId"),
                FirstDate(p, "updatedAt", "editedAt", "createdAt"), TaskCodes(p)))
            .ToList();
        var counts = r.ValueKind == JsonValueKind.Object && r.TryGetProperty("counts", out var c) ? c : default;
        return new Notebook(folders, pages, Count(counts, "inbox"), Count(counts, "todos"));
    }

    internal static NoteContent ParseNote(JsonElement r, string pageId)
    {
        var page = r.ValueKind == JsonValueKind.Object && r.TryGetProperty("page", out var p) && p.ValueKind == JsonValueKind.Object ? p : r;
        var todos = Items(r, "todos")
            .Select(t => (Text: FirstStr(t, "text", "title", "content") ?? "", Done: Bool(t, "done") || Bool(t, "checked") || Bool(t, "completed")))
            .Where(t => t.Text.Length > 0)
            .ToList();
        return new NoteContent(FirstStr(page, "id", "pageId") ?? pageId, FirstStr(page, "title", "name") ?? "（未命名）",
            FirstStr(page, "markdown", "content", "body") ?? "", TaskCodes(page).Concat(TaskCodes(r)).Distinct().ToList(), todos);
    }

    // ───────────── 團隊空間 ─────────────

    public async Task<List<SpaceInfo>> ListSpacesAsync(CancellationToken ct) =>
        ParseSpaces(await Mcp.CallToolAsync("list_spaces", null, ct));

    public async Task<List<SpaceNode>> SpaceTreeAsync(string space, CancellationToken ct) =>
        ParseSpaceTree(await Mcp.CallToolAsync("get_space_tree", new { space }, ct));

    public async Task<SpacePageContent> GetSpacePageAsync(string space, string pageId, CancellationToken ct) =>
        ParseSpacePage(await Mcp.CallToolAsync("get_space_page", new { space, pageId }, ct), pageId);

    /// <summary>space 為 null 時搜全部看得到的空間；project：只搜那個專案的空間。</summary>
    public async Task<List<KnowledgeHit>> SearchSpacesAsync(string q, string? space, CancellationToken ct, string? project = null)
    {
        var args = new Dictionary<string, object?> { ["q"] = q };
        if (space is not null)
            args["space"] = space;
        if (project is not null)
            args["project"] = project;
        var r = await Mcp.CallToolAsync("search_spaces", args, ct);
        return ParseHits(r).Concat(ParseFileHits(r)).ToList();
    }

    /// <summary>
    /// 新增一頁（parentId 為 null 放最上層；上層可以是文件、白板或資料夾）；folder＝建資料夾（沒有內文）。
    /// 回傳新頁的 id（回應裡找不到就是 null）。
    /// </summary>
    public async Task<string?> CreateSpacePageAsync(string space, string? parentId, string title, string markdown, CancellationToken ct,
        bool folder = false)
    {
        var args = new Dictionary<string, object?> { ["space"] = space, ["title"] = title };
        if (folder)
            args["kind"] = "FOLDER";
        else
            args["markdown"] = markdown;
        if (parentId is not null)
            args["parentId"] = parentId;
        return NewId(await Mcp.CallToolAsync("create_space_page", args, ct));
    }

    public Task AppendSpacePageAsync(string space, string pageId, string markdown, CancellationToken ct) =>
        Mcp.CallToolAsync("append_to_space_page", new { space, pageId, markdown }, ct);

    /// <summary>改一頁：title、markdown 為 null 的不改；整頁換掉前伺服器會自動存一版。</summary>
    public Task UpdateSpacePageAsync(string space, string pageId, string? title, string? markdown, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["space"] = space, ["pageId"] = pageId };
        if (title is not null)
            args["title"] = title;
        if (markdown is not null)
            args["markdown"] = markdown;
        return Mcp.CallToolAsync("update_space_page", args, ct);
    }

    /// <summary>移到同一個空間的別的上層（parentId 為 null＝最上層），排在最後。</summary>
    public Task MoveSpacePageAsync(string space, string pageId, string? parentId, CancellationToken ct) =>
        Mcp.CallToolAsync("move_space_page", new Dictionary<string, object?> { ["space"] = space, ["pageId"] = pageId, ["parentId"] = parentId }, ct);

    /// <summary>刪除一頁（連子頁）；30 天內空間管理員可以在網頁還原。</summary>
    public Task DeleteSpacePageAsync(string space, string pageId, CancellationToken ct) =>
        Mcp.CallToolAsync("delete_space_page", new { space, pageId }, ct);

    /// <summary>資料夾頁（kind 是 FOLDER，舊的檔案庫也是）裡的子頁與上傳的檔案。</summary>
    public async Task<List<SpaceFile>> ListSpaceFilesAsync(string space, string folder, CancellationToken ct) =>
        ParseSpaceFiles(await Mcp.CallToolAsync("list_space_files", new { spaceKey = space, folder }, ct));

    /// <summary>下載連結（1 小時有效，只有本人能用）。</summary>
    public async Task<string?> SpaceFileLinkAsync(string fileId, CancellationToken ct)
    {
        var r = await Mcp.CallToolAsync("get_space_file_link", new { fileId }, ct);
        return r.ValueKind == JsonValueKind.String ? r.GetString() : FirstStr(r, "url", "link", "downloadUrl", "href");
    }

    internal static List<SpaceInfo> ParseSpaces(JsonElement r) =>
        Items(r, "spaces", "items")
            .Where(s => FirstStr(s, "key", "code") is not null)
            .Select(s => new SpaceInfo(FirstStr(s, "key", "code")!, FirstStr(s, "name", "title") ?? FirstStr(s, "key", "code")!,
                FirstStr(s, "role", "myRole"), Number(s, "pageCount", "pages"),
                s.TryGetProperty("project", out var p) && p.ValueKind == JsonValueKind.Object ? FirstStr(p, "key", "code")
                    : FirstStr(s, "project", "projectKey"),
                Bool(s, "archived") || Bool(s, "isArchived") || Bool(s, "readOnly")))
            .ToList();

    /// <summary>頁面樹：平的清單（parentId）或巢狀（children）都接受，攤平成照樹狀順序的清單。</summary>
    internal static List<SpaceNode> ParseSpaceTree(JsonElement r)
    {
        var list = new List<SpaceNode>();
        void Walk(IEnumerable<JsonElement> nodes, string? parent)
        {
            foreach (var n in nodes)
            {
                if (FirstStr(n, "id", "pageId") is not { } id)
                    continue;
                list.Add(new SpaceNode(id, FirstStr(n, "title", "name") ?? "（未命名）", FirstStr(n, "parentId") ?? parent,
                    FirstStr(n, "kind", "type"), FirstDate(n, "updatedAt", "editedAt"), Number(n, "fileCount", "files")));
                Walk(Items(n, "children"), id);
            }
        }
        Walk(Items(r, "pages", "tree", "nodes", "items"), null);
        return list;
    }

    internal static SpacePageContent ParseSpacePage(JsonElement r, string pageId)
    {
        var page = r.ValueKind == JsonValueKind.Object && r.TryGetProperty("page", out var p) && p.ValueKind == JsonValueKind.Object ? p : r;
        var owner = page.ValueKind == JsonValueKind.Object && page.TryGetProperty("owner", out var o)
            ? o.ValueKind == JsonValueKind.String ? o.GetString() : Person(o)?.DisplayName ?? FirstStr(o, "name")
            : null;
        var path = Items(page, "path", "breadcrumbs")
            .Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() : FirstStr(x, "title", "name"))
            .Where(x => !string.IsNullOrWhiteSpace(x))
            .Select(x => x!)
            .ToList();
        return new SpacePageContent(FirstStr(page, "id", "pageId") ?? pageId, FirstStr(page, "title", "name") ?? "（未命名）",
            FirstStr(page, "markdown", "content", "body") ?? "", owner, FirstDate(page, "updatedAt", "editedAt"), path);
    }

    /// <summary>
    /// 資料夾裡的子頁（pages／children；舊的檔案庫回 folders）與檔案（files）；子頁沒寫種類的當資料夾。
    /// 實際回應還沒有樣本（帳號看不到任何空間），照工具說明寬鬆讀。
    /// </summary>
    internal static List<SpaceFile> ParseSpaceFiles(JsonElement r)
    {
        var pages = new[] { "pages", "children", "subpages", "folders" }
            .SelectMany(name => Items(r, name))
            .Concat(Items(r, "files", "items").Where(f => FirstStr(f, "kind") is not null)) // 子頁和檔案混在同一個陣列時，有 kind 的是子頁
            .Where(f => FirstStr(f, "id", "pageId") is not null)
            .Select(f =>
            {
                var kind = (FirstStr(f, "kind", "type") ?? "FOLDER").ToUpperInvariant();
                var folder = kind is "FOLDER" or "FILES";
                return new SpaceFile(FirstStr(f, "id", "pageId")!, FirstStr(f, "title", "name") ?? (folder ? "（未命名資料夾）" : "（未命名）"), folder,
                    null, null, FirstDate(f, "updatedAt", "createdAt"), kind, PathText(f));
            });
        var files = Items(r, "files", "items")
            .Where(f => FirstStr(f, "id", "fileId") is not null && FirstStr(f, "kind") is null)
            .Select(f => new SpaceFile(FirstStr(f, "id", "fileId")!, FirstStr(f, "name", "fileName", "title") ?? "（未命名）", false,
                Number(f, "size", "bytes"), PersonName(f, "uploader", "uploadedBy", "createdBy"), FirstDate(f, "createdAt", "uploadedAt", "updatedAt"),
                Path: PathText(f)));
        return pages.DistinctBy(p => p.Id).OrderByDescending(p => p.IsFolder).Concat(files).ToList();
    }

    /// <summary>所在路徑：字串，或頁名陣列（字串或 { title }）。</summary>
    static string? PathText(JsonElement f)
    {
        if (FirstStr(f, "path", "folderPath") is { } path)
            return path;
        var parts = Items(f, "path")
            .Select(x => x.ValueKind == JsonValueKind.String ? x.GetString() : FirstStr(x, "title", "name"))
            .Where(x => !string.IsNullOrWhiteSpace(x))
            .ToList();
        return parts.Count > 0 ? string.Join(" › ", parts) : null;
    }

    /// <summary>search_spaces 的 files：資料夾裡檔名符合的檔案（帶所在資料夾 folderId 與路徑 path）。</summary>
    internal static List<KnowledgeHit> ParseFileHits(JsonElement r) =>
        Items(r, "files")
            .Where(f => FirstStr(f, "id", "fileId") is not null && FirstStr(f, "folderId", "pageId", "folder") is not null)
            .Select(f => new KnowledgeHit(FirstStr(f, "id", "fileId")!, FirstStr(f, "name", "fileName", "title") ?? "（未命名）",
                FirstStr(f, "space", "spaceKey"), PathText(f), FirstStr(f, "folderId", "pageId", "folder")))
            .ToList();

    // ───────────── 論壇 ─────────────

    /// <summary>project＋channel（論壇名稱）的貼文；置頂的在最上面，其他照最新活動。頻道不是論壇時伺服器回「找不到這個論壇」。</summary>
    public async Task<List<ForumPost>> ListForumPostsAsync(string project, string channel, bool unsolved, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["project"] = project, ["channel"] = channel };
        if (unsolved)
            args["unsolved"] = true;
        return ParseForumPosts(await Mcp.CallToolAsync("list_forum_posts", args, ct));
    }

    public async Task<ForumThread> GetForumPostAsync(string project, string channel, string post, CancellationToken ct) =>
        ParseForumThread(await Mcp.CallToolAsync("get_forum_post", new { project, channel, post }, ct));

    public Task CreateForumPostAsync(string project, string channel, string title, string body, CancellationToken ct) =>
        Mcp.CallToolAsync("create_forum_post", new { project, channel, title, body }, ct);

    public Task ReplyForumPostAsync(string project, string channel, string post, string body, CancellationToken ct) =>
        Mcp.CallToolAsync("reply_forum_post", new { project, channel, post, body }, ct);

    internal static List<ForumPost> ParseForumPosts(JsonElement r) =>
        Items(r, "posts", "items")
            .Where(p => FirstStr(p, "id") is not null)
            .Select(p => new ForumPost(FirstStr(p, "id")!, FirstStr(p, "title") ?? "（沒有標題）", PersonName(p, "author") ?? "",
                FirstDate(p, "createdAt"), Number(p, "replies", "replyCount") ?? 0, FirstDate(p, "lastActivityAt", "updatedAt"),
                Tags(p), Bool(p, "pinned"), Bool(p, "solved")))
            .ToList();

    internal static ForumThread ParseForumThread(JsonElement r)
    {
        var thread = r.ValueKind == JsonValueKind.Object && r.TryGetProperty("thread", out var t) && t.ValueKind == JsonValueKind.Object ? t : default;
        var rootElement = thread.ValueKind == JsonValueKind.Object && thread.TryGetProperty("root", out var root) && root.ValueKind == JsonValueKind.Object
            ? root : default;
        var forum = rootElement.ValueKind == JsonValueKind.Object && rootElement.TryGetProperty("forum", out var f) && f.ValueKind == JsonValueKind.Object
            ? f : default;
        var replies = Items(r, "items", "replies")
            .Select(ForumMessageOf)
            .Where(m => m is not null)
            .Select(m => m!)
            .OrderBy(m => m.CreatedAt ?? DateTimeOffset.MinValue)
            .ToList();
        var solved = forum.ValueKind == JsonValueKind.Object && forum.TryGetProperty("solvedAt", out var s) && s.ValueKind == JsonValueKind.String;
        return new ForumThread(FirstStr(forum, "title") ?? FirstStr(rootElement, "title") ?? "", ForumMessageOf(rootElement), replies, solved, Tags(forum));
    }

    static ForumMessage? ForumMessageOf(JsonElement m) =>
        FirstStr(m, "id") is { } id
            ? new ForumMessage(id, PersonName(m, "author") ?? "", FirstStr(m, "body", "content") ?? "", FirstDate(m, "createdAt"),
                Str(m, "deletedAt") is not null)
            : null;

    // ───────────── 投票 ─────────────

    public async Task<PollInfo> GetPollAsync(string poll, CancellationToken ct) =>
        ParsePoll(await Mcp.CallToolAsync("get_poll", new { poll }, ct));

    internal static PollInfo ParsePoll(JsonElement r) =>
        new(FirstStr(r, "question", "title") ?? "（沒有題目）", FirstStr(r, "place"), FirstStr(r, "type"), Bool(r, "anonymous"),
            FirstStr(r, "status"), FirstDate(r, "closesAt"), PersonName(r, "createdBy"),
            Items(r, "options")
                .Select(o => new PollOption(FirstStr(o, "label", "text") ?? "", Number(o, "votes"),
                    Items(o, "voters").Select(v => v.ValueKind == JsonValueKind.String ? v.GetString() : Person(v)?.DisplayName ?? FirstStr(v, "name"))
                        .Where(v => !string.IsNullOrWhiteSpace(v)).Select(v => v!).ToList(),
                    Bool(o, "mine")))
                .ToList(),
            Number(r, "votedCount"), Number(r, "pendingCount"));

    // ───────────── 共用 ─────────────

    /// <summary>人可能是字串（顯示名稱）或 { displayName, username }。</summary>
    static string? PersonName(JsonElement e, params string[] names)
    {
        foreach (var name in names)
        {
            if (e.ValueKind != JsonValueKind.Object || !e.TryGetProperty(name, out var v))
                continue;
            if (v.ValueKind == JsonValueKind.String)
                return v.GetString();
            var display = Person(v)?.DisplayName ?? FirstStr(v, "displayName", "name");
            if (display is not null)
                return display;
        }
        return null;
    }

    static List<string> Tags(JsonElement e) =>
        Items(e, "tags")
            .Select(t => t.ValueKind == JsonValueKind.String ? t.GetString() : FirstStr(t, "name", "label"))
            .Where(t => !string.IsNullOrWhiteSpace(t))
            .Select(t => t!)
            .ToList();

    static List<KnowledgeHit> ParseHits(JsonElement r) =>
        Items(r, "items", "results", "pages")
            .Where(h => FirstStr(h, "pageId", "id") is not null)
            .Select(h => new KnowledgeHit(FirstStr(h, "pageId", "id")!, FirstStr(h, "title", "name") ?? "（未命名）",
                FirstStr(h, "space", "spaceKey"), FirstStr(h, "snippet", "excerpt", "match", "preview")))
            .ToList();

    internal static List<KnowledgeHit> ParseKnowledgeHits(JsonElement r) => ParseHits(r);

    static List<string> TaskCodes(JsonElement e) =>
        Items(e, "taskCodes", "tasks")
            .Select(t => t.ValueKind == JsonValueKind.String ? t.GetString() : FirstStr(t, "key", "code", "task"))
            .Where(t => !string.IsNullOrWhiteSpace(t))
            .Select(t => t!)
            .ToList();

    static int? Number(JsonElement e, params string[] names)
    {
        foreach (var name in names)
            if (e.ValueKind == JsonValueKind.Object && e.TryGetProperty(name, out var v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt32(out var n))
                return n;
        return null;
    }

    static int Count(JsonElement e, string name) => Number(e, name) ?? 0;

    static string? NewId(JsonElement r)
    {
        if (FirstStr(r, "id", "pageId") is { } id)
            return id;
        return r.ValueKind == JsonValueKind.Object && r.TryGetProperty("page", out var p) ? FirstStr(p, "id", "pageId") : null;
    }
}
