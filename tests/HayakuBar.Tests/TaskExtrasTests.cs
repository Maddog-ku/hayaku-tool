using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class TaskExtrasTests
{
    static JsonElement Json(string json) => JsonDocument.Parse(json).RootElement.Clone();

    [Fact]
    public void 異動紀錄_換成中文說明()
    {
        var items = HayakuApi.ParseTaskActivity(Json("""
            {"items":[
              {"type":"change","action":"task.update","actor":{"username":"a","displayName":"小明"},"source":"MCP",
               "changes":{"title":["第一階段研究","第一階段"],"status":["客戶驗證中","處理中"]},"undone":false,"createdAt":"2026-10-30T05:19:49.782Z"},
              {"type":"change","action":"time_entry.auto_pause","actor":null,"source":"SYSTEM",
               "changes":{"stopReason":[null,"BREAK"],"durationSeconds":[null,12600]},"undone":false,"createdAt":"2026-10-27T08:26:55.782Z"},
              {"type":"change","action":"task.subtask.create","actor":{"username":"a","displayName":"小明"},"source":"WEB",
               "changes":{"subtask":[null,"F2P-76 系統規劃"]},"undone":true,"createdAt":"2026-10-27T07:34:50.333Z"},
              {"type":"change","action":"something.new","actor":{"username":"a","displayName":"小明"},"createdAt":"2026-10-27T07:00:00Z"}
            ],"nextCursor":null}
            """));
        Assert.Equal(4, items.Count);
        Assert.Equal("修改：名稱：第一階段研究 → 第一階段、狀態：客戶驗證中 → 處理中（透過 MCP）", items[0].Text);
        Assert.Equal("小明", items[0].Actor);
        Assert.Equal("休息時間自動暫停（3 小時 30 分）", items[1].Text);
        Assert.Equal("系統", items[1].Actor);
        Assert.Equal("新增子任務 F2P-76 系統規劃（已復原）", items[2].Text);
        Assert.Equal("something.new", items[3].Text);
    }

    [Fact]
    public void 文件與白板()
    {
        var docs = HayakuApi.ParseContent(Json("""
            {"task":"F2P-27","documents":[{"id":"x","title":"文件","markdown":"# 標題\n內容"}],
             "whiteboards":[{"title":"流程","texts":["開始","結束"]}]}
            """));
        Assert.Equal(2, docs.Count);
        Assert.Equal(("文件", "# 標題\n內容"), (docs[0].Title, docs[0].Text));
        Assert.Equal(("白板：流程", "開始\n結束"), (docs[1].Title, docs[1].Text));
    }

    [Fact]
    public void 相依關係_另一端的任務放在不同位置都認得()
    {
        var deps = HayakuApi.ParseDependencies(Json("""
            {"predecessors":[
               {"id":"d1","template":"finishToStart","blocking":true,"task":{"task":"F2P-10","title":"設計","status":"處理中"}},
               {"id":"d2","kind":"GATE","condition":["完成"],"predecessor":{"key":"F2P-11","title":"審核"}}],
             "successors":[{"id":"d3","templateName":"完成後自動開始","taskKey":"F2P-20"}]}
            """));
        Assert.Equal(2, deps.Predecessors.Count);
        Assert.Equal(("F2P-10", "設計", "處理中", "完成後才能開始", true),
            (deps.Predecessors[0].OtherKey, deps.Predecessors[0].OtherTitle, deps.Predecessors[0].OtherStatus, deps.Predecessors[0].Rule, deps.Predecessors[0].Blocking));
        Assert.Equal(("F2P-11", "限制（完成）", false), (deps.Predecessors[1].OtherKey, deps.Predecessors[1].Rule, deps.Predecessors[1].Blocking));
        Assert.Equal(("d3", "F2P-20", "完成後自動開始"), (deps.Successors[0].Id, deps.Successors[0].OtherKey, deps.Successors[0].Rule));
    }

    [Fact]
    public void 附件()
    {
        var items = HayakuApi.ParseAttachments(Json("""
            {"webUrl":"https://h/x","attachments":[{"name":"規格.pdf","size":2048,"uploadedBy":{"username":"a","displayName":"小明"},
              "createdAt":"2026-10-30T01:00:00Z","webUrl":"https://h/a"},{"fileName":"b.png"}]}
            """));
        Assert.Equal(("規格.pdf", 2048L, "小明", "https://h/a"), (items[0].Name, items[0].Size, items[0].Uploader, items[0].Url));
        Assert.Equal(("b.png", (long?)null), (items[1].Name, items[1].Size));
    }

    static NotificationItem Notice(string type, string? task = null, string? meeting = null) =>
        new("n1", type, "", "", null, task, "F2P", meeting, DateTimeOffset.Now, true, 1);

    [Fact]
    public void 通知_在工作台對應的頁打開()
    {
        Assert.Equal(("reviews", (object?)null), TimerEngine.NotificationTarget(Notice("CHANGE_REQUESTED", "F2P-1")));
        Assert.Equal(("tasks", (object?)"F2P-1"), TimerEngine.NotificationTarget(Notice("STATUS_CHANGED", "F2P-1")));
        Assert.Equal(("meetings", (object?)"m1"), TimerEngine.NotificationTarget(Notice("MEETING_INVITED", meeting: "m1")));
        Assert.Equal(("hours", (object?)null), TimerEngine.NotificationTarget(Notice("HOURS_UNFILLED")));
        Assert.Equal(((string?)null, (object?)null), TimerEngine.NotificationTarget(Notice("SECURITY_ALERT")));
    }
}
