using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

/// <summary>v1.5.0 接上的 MCP 工具：會議紀錄歷程、子任務樹、投票、批次新增的清單解析。回應都取自實際呼叫。</summary>
public class NewToolsTests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement;

    [Fact]
    public void ParseMeetingNoteHistory_讀編輯者時間摘要與差異()
    {
        var versions = HayakuApi.ParseMeetingNoteHistory(Json("""
            {"meeting":"REVIVAL-9","note":"會議紀錄","versions":[
              {"editor":"邱子恩","startedAt":"2026-10-27T07:42:49.206Z","endedAt":"2026-10-27T07:44:12.090Z","summary":"＋1 段 · 改了 1 段",
               "stats":{"added":1,"changed":1,"checked":0,"removed":0,"tasksInserted":0},"diff":"～ {++ok++}\n＋ `123`"},
              {"editor":"羅瑞騰","startedAt":"2026-10-27T07:41:57.451Z","endedAt":"2026-10-27T07:42:10.167Z","summary":"＋1 段 · 插入 1 個任務",
               "diff":"＋ 任務 EMMHP-123 程式開發"}],"timezone":"Asia/Taipei"}
            """));
        Assert.Equal(2, versions.Count);
        Assert.Equal(("邱子恩", "＋1 段 · 改了 1 段", "～ {++ok++}\n＋ `123`"), (versions[0].Editor, versions[0].Summary, versions[0].Diff));
        Assert.Equal(DateTimeOffset.Parse("2026-10-27T07:44:12.090Z"), versions[0].At);
        Assert.Empty(HayakuApi.ParseMeetingNoteHistory(Json("""{"meeting":"REVIVAL-57","note":"會議紀錄","versions":[]}""")));
    }

    [Fact]
    public void ParseTaskTree_照樹狀順序攤平並記層數()
    {
        var rows = HayakuApi.ParseTaskTree(Json("""
            {"task":"F2P-14","title":"機密列印","children":[
              {"task":"F2P-15","title":"事務機APP開發","status":"已完成","statusCategory":"DONE","assignees":["harry.gu"],"children":[]},
              {"task":"F2P-16","title":"Server端開發","status":"已完成","statusCategory":"DONE","assignees":["nick.chio"],"children":[
                {"task":"F2P-17","title":"規劃研究","status":"已完成","statusCategory":"DONE","assignees":["nick.chio"],"children":[]}]},
              {"task":"F2P-101","title":"面板新增全部列印及選擇列印按鈕","status":"處理中","statusCategory":"IN_PROGRESS","assignees":[],"children":[]}],
             "truncated":false}
            """));
        Assert.Equal(["F2P-15", "F2P-16", "F2P-17", "F2P-101"], rows.Select(r => r.Key));
        Assert.Equal([1, 1, 2, 1], rows.Select(r => r.Depth));
        Assert.Equal(("規劃研究", "已完成", "DONE", "nick.chio"), (rows[2].Title, rows[2].Status, rows[2].StatusCategory, rows[2].Assignees.Single()));
    }

    [Fact]
    public void ParsePoll_讀選項票數投票的人與狀態()
    {
        var poll = HayakuApi.ParsePoll(Json("""
            {"id":"0a793a8b-b232-4ed0-9fe0-e3aa7dd3d373","place":"聊天","question":"午餐吃什麼","type":"單選","anonymous":false,"status":"進行中",
             "closesAt":null,"closedAt":null,"createdBy":"羅瑞騰","resultsVisible":true,
             "options":[{"label":"便當","votes":3,"voters":["丁鈺昕","羅瑞騰","測試人員"],"mine":false},
                        {"label":"麵","votes":0,"voters":[],"mine":true}],
             "votedCount":3,"pendingCount":5,"timezone":"Asia/Taipei"}
            """));
        Assert.Equal(("午餐吃什麼", "單選", "進行中", "羅瑞騰", 3, 5), (poll.Question, poll.Type, poll.Status, poll.CreatedBy, poll.Voted, poll.Pending));
        Assert.Null(poll.ClosesAt);
        Assert.Equal((3, 3, false), (poll.Options[0].Votes, poll.Options[0].Voters.Count, poll.Options[0].Mine));
        Assert.True(poll.Options[1].Mine);
    }

    [Fact]
    public void ParsePoll_投了才看得到結果時票數是null()
    {
        var poll = HayakuApi.ParsePoll(Json("""{"question":"Q","options":[{"label":"A","mine":false}],"votedCount":null}"""));
        Assert.Null(poll.Options[0].Votes);
        Assert.Null(poll.Voted);
    }

    [Fact]
    public void ParseChat_投票訊息帶投票id()
    {
        var messages = HayakuApi.ParseChat(Json("""
            {"messages":[{"id":"m1","author":{"username":"return.lo","displayName":"羅瑞騰"},"body":"Q","createdAt":"2026-10-27T07:10:35.148Z",
              "deletedAt":null,"poll":{"id":"0a793a8b-b232-4ed0-9fe0-e3aa7dd3d373","place":"CHAT","question":"Q","options":[]}},
             {"id":"m2","author":{"username":"a","displayName":"A"},"body":"hi","createdAt":"2026-10-27T07:11:00Z","deletedAt":null}]}
            """));
        Assert.Equal(("Q", "0a793a8b-b232-4ed0-9fe0-e3aa7dd3d373"), (messages[0].PollQuestion, messages[0].PollId));
        Assert.Null(messages[1].PollId);
    }

    [Fact]
    public void TaskOutline_縮排是子任務_項目符號與編號拿掉()
    {
        var nodes = TaskOutline.Parse("""
            - 第一階段
              1. 規劃
              2. 開發
                  [ ] 後端
              	前端
            • 第二階段

            3、驗收
            """);
        Assert.Equal(["第一階段", "第二階段", "驗收"], nodes.Select(n => n.Title));
        Assert.Equal(["規劃", "開發"], nodes[0].Children.Select(n => n.Title));
        Assert.Equal(["後端", "前端"], nodes[0].Children[1].Children.Select(n => n.Title));
        Assert.Equal(7, TaskOutline.Count(nodes));
    }

    [Fact]
    public void TaskOutline_一般文字不誤刪_空白清單是空的()
    {
        var nodes = TaskOutline.Parse("2026 年度規劃\n-負號開頭\n");
        Assert.Equal(["2026 年度規劃", "-負號開頭"], nodes.Select(n => n.Title));
        Assert.Empty(TaskOutline.Parse("  \n\n"));
    }
}
