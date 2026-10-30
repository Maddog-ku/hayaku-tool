using System.Text.Json;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class KnowledgeTests
{
    static JsonElement Json(string json) => JsonDocument.Parse(json).RootElement.Clone();

    [Fact]
    public void 空間頁面樹_巢狀攤平成樹狀順序()
    {
        var nodes = HayakuApi.ParseSpaceTree(Json("""
            {"pages":[{"id":"a","title":"首頁","children":[{"id":"b","title":"規範"},{"id":"c","title":"檔案","kind":"FILES"}]},{"id":"d","title":"其他"}]}
            """));
        Assert.Equal(["a", "b", "c", "d"], nodes.Select(n => n.Id));
        Assert.Equal("a", nodes[1].ParentId);
        Assert.True(nodes[2].IsFiles);
        Assert.Null(nodes[3].ParentId);
    }

    [Fact]
    public void 論壇貼文與回覆()
    {
        var posts = HayakuApi.ParseForumPosts(Json("""
            {"posts":[{"id":"p1","title":"列印失敗","author":{"username":"a","displayName":"小明"},"replyCount":2,"pinned":true,"tags":["bug"]}]}
            """));
        Assert.Equal(("p1", "列印失敗", "小明", 2, true), (posts[0].Id, posts[0].Title, posts[0].Author, posts[0].Replies, posts[0].Pinned));
        Assert.Equal(["bug"], posts[0].Tags);

        var thread = HayakuApi.ParseForumThread(Json("""
            {"thread":{"root":{"id":"p1","body":"請看","author":"小明","forum":{"title":"列印失敗","solvedAt":"2026-10-30T01:00:00Z"}}},
             "items":[{"id":"r2","body":"後","createdAt":"2026-10-30T02:00:00Z"},{"id":"r1","body":"先","createdAt":"2026-10-30T01:00:00Z"}]}
            """));
        Assert.Equal("列印失敗", thread.Title);
        Assert.True(thread.Solved);
        Assert.Equal("請看", thread.Root?.Body);
        Assert.Equal(["先", "後"], thread.Replies.Select(r => r.Body));
    }

    [Fact]
    public void 筆記本()
    {
        var book = HayakuApi.ParseNotebook(Json("""
            {"folders":[{"id":"f1","name":"會議"}],"pages":[{"id":"n1","title":"週會","folderId":"f1","taskCodes":["F2P-1"]}],"counts":{"inbox":3,"todos":2}}
            """));
        Assert.Equal(("n1", "週會", "f1"), (book.Pages[0].Id, book.Pages[0].Title, book.Pages[0].FolderId));
        Assert.Equal(["F2P-1"], book.Pages[0].TaskCodes);
        Assert.Equal((3, 2), (book.Inbox, book.Todos));
    }
}
