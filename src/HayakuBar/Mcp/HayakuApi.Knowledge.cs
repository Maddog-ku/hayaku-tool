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

/// <summary>筆記本、團隊空間的搜尋結果；Space 只有團隊空間才有。</summary>
public sealed record KnowledgeHit(string Id, string Title, string? Space, string? Snippet);

/// <summary>團隊空間（list_spaces）。</summary>
public sealed record SpaceInfo(string Key, string Name, string? Role, int? PageCount);

/// <summary>空間頁面樹的一個節點（get_space_tree）；Kind：一般頁、檔案庫（FILES）…</summary>
public sealed record SpaceNode(string Id, string Title, string? ParentId, string? Kind, DateTimeOffset? UpdatedAt)
{
    public bool IsFiles => string.Equals(Kind, "FILES", StringComparison.OrdinalIgnoreCase);
}

/// <summary>空間的一頁（get_space_page）；Path：上層的頁名。</summary>
public sealed record SpacePageContent(string Id, string Title, string Markdown, string? Owner, DateTimeOffset? UpdatedAt,
    IReadOnlyList<string> Path);

/// <summary>檔案庫的資料夾或檔案（list_space_files）。</summary>
public sealed record SpaceFile(string Id, string Name, bool IsFolder, long? Size, string? Uploader, DateTimeOffset? At);

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

    /// <summary>space 為 null 時搜全部看得到的空間。</summary>
    public async Task<List<KnowledgeHit>> SearchSpacesAsync(string q, string? space, CancellationToken ct) =>
        ParseHits(await Mcp.CallToolAsync("search_spaces", space is null ? new { q } : new { q, space }, ct));

    /// <summary>新增一頁（parentId 為 null 放最上層）；回傳新頁的 id（回應裡找不到就是 null）。</summary>
    public async Task<string?> CreateSpacePageAsync(string space, string? parentId, string title, string markdown, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["space"] = space, ["title"] = title, ["markdown"] = markdown };
        if (parentId is not null)
            args["parentId"] = parentId;
        return NewId(await Mcp.CallToolAsync("create_space_page", args, ct));
    }

    public Task AppendSpacePageAsync(string space, string pageId, string markdown, CancellationToken ct) =>
        Mcp.CallToolAsync("append_to_space_page", new { space, pageId, markdown }, ct);

    /// <summary>檔案庫頁（kind 是 FILES）裡的資料夾與檔案；folderId 看某個資料夾。</summary>
    public async Task<List<SpaceFile>> ListSpaceFilesAsync(string space, string pageId, string? folderId, CancellationToken ct)
    {
        var args = new Dictionary<string, object?> { ["spaceKey"] = space, ["pageId"] = pageId };
        if (folderId is not null)
            args["folderId"] = folderId;
        return ParseSpaceFiles(await Mcp.CallToolAsync("list_space_files", args, ct));
    }

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
                FirstStr(s, "role", "myRole"), Number(s, "pageCount", "pages")))
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
                    FirstStr(n, "kind", "type"), FirstDate(n, "updatedAt", "editedAt")));
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

    internal static List<SpaceFile> ParseSpaceFiles(JsonElement r)
    {
        var folders = Items(r, "folders")
            .Where(f => FirstStr(f, "id") is not null)
            .Select(f => new SpaceFile(FirstStr(f, "id")!, FirstStr(f, "name", "title") ?? "（未命名資料夾）", true, null, null,
                FirstDate(f, "updatedAt", "createdAt")));
        var files = Items(r, "files", "items")
            .Where(f => FirstStr(f, "id", "fileId") is not null)
            .Select(f => new SpaceFile(FirstStr(f, "id", "fileId")!, FirstStr(f, "name", "fileName", "title") ?? "（未命名）", false,
                Number(f, "size", "bytes"), PersonName(f, "uploader", "uploadedBy", "createdBy"), FirstDate(f, "createdAt", "uploadedAt", "updatedAt")));
        return folders.Concat(files).ToList();
    }

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
