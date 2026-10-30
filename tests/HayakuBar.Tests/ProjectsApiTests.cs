using System.Text.Json;
using HayakuBar.Config;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

/// <summary>Hayaku 1.2 新增的工具（釘選托盤、變更申請、專案動態、工作負荷）與我的任務的新欄位；樣本取自實際回應。</summary>
public class ProjectsApiTests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement;

    [Fact]
    public void ParsePins_讀取今天累積與能不能計時()
    {
        var pins = HayakuApi.ParsePins(Json("""
            {"today":"2026-10-30","pins":[
              {"task":"F2P-126","title":"更新版本覆蓋設定問題","project":"F2P","status":"處理中","statusCategory":"IN_PROGRESS",
               "today":{"minutes":5,"display":"0.08 小時"},"running":false,"canTime":true,"isCategory":false},
              {"task":"F2P-25","title":"第二階段","project":"F2P","status":"進行中","today":{"minutes":0},"running":true,"canTime":false}
            ]}
            """));
        Assert.Equal(2, pins.Count);
        Assert.Equal(("F2P-126", 5, false, true), (pins[0].Key, pins[0].TodayMinutes, pins[0].Running, pins[0].CanTime));
        Assert.True(pins[1].Running);
        Assert.False(pins[1].CanTime);
    }

    [Fact]
    public void ParseChangeRequests_讀取任務申請人與改了什麼()
    {
        var list = HayakuApi.ParseChangeRequests(Json("""
            {"items":[
              {"project":"F2P","id":"a1","type":"UPDATE","status":"PENDING",
               "task":{"key":"F2P-97","number":97,"title":"報表頁面新增歸0按鈕"},
               "requester":{"username":"harry.gu","displayName":"古信汯"},
               "changes":{"statusId":"dcc4d548","plannedEnd":"2026-11-05","estimatedMinutes":90,"note":null},
               "reason":"客戶要求延後","loggedSeconds":12876,"descendantCount":2,"createdAt":"2026-10-27T08:53:13.107Z"},
              {"project":"TEST","id":"b2","type":"CREATE","status":"PENDING","task":{"key":"TEST-11","title":"42"},
               "requester":{"username":"bruce.cheng","displayName":"鄭為元"},"changes":null,"createdAt":"2026-10-27T07:49:26.094Z"}
            ]}
            """));
        Assert.Equal(2, list.Count);
        var c = list[0];
        Assert.Equal(("a1", "UPDATE", "F2P-97", "harry.gu"), (c.Id, c.Type, c.TaskKey, c.Requester?.Username));
        Assert.Equal(["狀態：有變更", "預計結束：2026-11-05", "預計工時：1.5 小時", "備註：清空"], c.Changes);
        Assert.Equal((214, 2, "客戶要求延後"), (c.LoggedMinutes, c.DescendantCount, c.Reason));
        Assert.Empty(list[1].Changes);
    }

    [Fact]
    public void ParseActivity_讀取摘要與動態()
    {
        var feed = HayakuApi.ParseActivity(Json("""
            {"from":"2026-10-26","to":"2026-10-30",
             "summary":{"created":294,"started":63,"completed":54,"overdue":45,"topMembers":[{"name":"古信汯","count":109}]},
             "items":[{"at":"10/30 13:41","project":"EG","text":"邱子恩 開始做 EG-10 新增一個轉正類別"}]}
            """), default, default);
        Assert.Equal((294, 63, 54, 45), (feed.Created, feed.Started, feed.Completed, feed.Overdue));
        Assert.Equal(new DateOnly(2026, 10, 26), feed.From);
        Assert.Equal([("古信汯", 109)], feed.TopMembers);
        Assert.Equal("10/30 13:41", feed.Items[0].At);
        Assert.Equal("EG", feed.Items[0].Project);
    }

    [Fact]
    public void ParseWorkload_實際模式讀每天的時數與容量()
    {
        var load = HayakuApi.ParseWorkload(Json("""
            {"from":"2026-10-26","to":"2026-11-01","mode":"actual","actual":{"minutes":1724},
             "people":[{"username":"harry.gu","capacity":{"minutes":1920},"actual":{"minutes":1724},
               "days":[{"date":"2026-10-27","actual":{"minutes":485},"capacity":{"minutes":480}}],
               "tasks":[{"task":"F2P-31","title":"Server端開發","actual":{"minutes":434}}]}]}
            """), default, default, "actual");
        Assert.Equal(("actual", 1724, 1920), (load.Mode, load.TotalMinutes, load.CapacityMinutes));
        Assert.Equal(new WorkloadDay(new DateOnly(2026, 10, 27), 485, 480, 0), load.Days[0]);
        Assert.Equal(new WorkloadTask("F2P-31", "Server端開發", 434), load.Tasks[0]);
    }

    [Fact]
    public void ParseWorkload_預計模式列出算不進來的任務()
    {
        var load = HayakuApi.ParseWorkload(Json("""
            {"mode":"planned","scheduled":{"minutes":0},"people":[{"scheduled":{"minutes":0},"capacity":{"minutes":1920},"days":[],"tasks":[]}],
             "incomplete":{"count":1,"tasks":[{"task":"F2P-19","title":"自訂槽測試","reason":"尚未填寫預計工時"}]}}
            """), new DateOnly(2026, 10, 26), new DateOnly(2026, 11, 1), "planned");
        Assert.Equal("planned", load.Mode);
        Assert.Equal(1920, load.CapacityMinutes);
        Assert.Equal([("F2P-19", "自訂槽測試", "尚未填寫預計工時")], load.Incomplete);
    }

    [Fact]
    public void 我的任務讀子任務數_分類與專案圖示()
    {
        var tasks = HayakuApi.ParseTaskSummaries(Json("""
            {"tasks":[{"task":"F2P-26","title":"點數系統","project":"F2P","status":"處理中","statusCategory":"IN_PROGRESS","childCount":2},
                      {"task":"F2P-28","title":"面板顯示餘額","project":"F2P","status":"處理中","statusCategory":"IN_PROGRESS","childCount":0},
                      {"task":"F2P-30","title":"舊版回應"}]}
            """));
        Assert.Equal([false, true, true], tasks.Select(t => t.IsLeaf));

        var projectTasks = HayakuApi.ParseProjectTasks(Json("""{"tasks":[{"task":"F2P-2","title":"文管","isCategory":true,"path":[]}]}"""));
        Assert.True(projectTasks[0].IsCategory);

        var projects = HayakuApi.ParseProjects(Json("""
            {"projects":[{"key":"EG","name":"EG-長榮","role":"ASSIGNEE","isArchived":false,"icon":"rocket","iconColor":"moss"},
                         {"key":"HYK","name":"Hayaku","role":"ADMIN","isArchived":true}]}
            """));
        Assert.Equal(("rocket", "moss", false), (projects[0].Icon, projects[0].IconColor, projects[0].CanReview));
        Assert.True(projects[1].CanReview);
        Assert.True(projects[1].IsArchived);
    }

    [Fact]
    public void TaskDetail讀分類()
    {
        var d = HayakuApi.ParseTaskDetail(Json("""{"task":"F2P-25","title":"第二階段","isCategory":true,"children":[]}"""));
        Assert.True(d.IsCategory);
    }

    [Fact]
    public void ParseEntries_讀執行者與來源()
    {
        var entries = HayakuApi.ParseEntries(Json("""
            {"entries":[{"id":"x","task":"F2P-26","title":"點數系統","user":"古信汯","startedAt":"2026-10-27T05:00:00.000Z",
              "endedAt":"2026-10-27T08:43:40.790Z","duration":{"minutes":224},"running":false,"source":"TIMER","note":null}]}
            """), "entries");
        Assert.Equal(("古信汯", "TIMER", 224), (entries[0].User, entries[0].Source, entries[0].DurationMinutes));
    }
}

public class StatusFilterAndColumnsTests
{
    static TaskSummary Task(string key, string status, string category) =>
        new(key, key, "P", status, category, "", null, null, false, "");

    [Fact]
    public void 狀態篩選_類別與專案狀態混著選符合任一個就顯示()
    {
        var filter = new StatusFilter();
        var tasks = new[] { Task("A-1", "處理中", "IN_PROGRESS"), Task("A-2", "待客戶回應", "IN_PROGRESS"), Task("A-3", "未開始", "TODO") };
        Assert.All(tasks, t => Assert.True(filter.Matches(t)));

        filter.Statuses.Add("待客戶回應");
        filter.Categories.Add("TODO");
        Assert.Equal(["A-2", "A-3"], tasks.Where(filter.Matches).Select(t => t.Key));

        filter.KeepOnly(["IN_PROGRESS"], ["處理中", "待客戶回應"]);
        Assert.Equal(1, filter.Count);
    }

    [Fact]
    public void 依狀態排序_同一類照專案設定的順序()
    {
        var tasks = new[]
        {
            Task("A-1", "待客戶回應", "IN_PROGRESS"), Task("A-2", "完成", "DONE"), Task("A-3", "處理中", "IN_PROGRESS"), Task("A-4", "未開始", "TODO"),
        };
        var order = new Dictionary<string, int> { ["未開始"] = 0, ["處理中"] = 1, ["待客戶回應"] = 2, ["完成"] = 3 };
        var sorted = TaskOrder.Sort(tasks, "status", new HashSet<string>(), t => order[t.Status]);
        Assert.Equal(["A-4", "A-3", "A-1", "A-2"], sorted.Select(t => t.Key));
    }

    [Fact]
    public void 狀態類別依流程順序統計()
    {
        var categories = TaskOrder.Categories([Task("A-1", "完成", "DONE"), Task("A-2", "處理中", "IN_PROGRESS"), Task("A-3", "不處理", "CANCELLED")]);
        Assert.Equal([("IN_PROGRESS", "進行中", 1), ("DONE", "完成", 1), ("CANCELLED", "已取消", 1)], categories);
    }

    [Fact]
    public void 欄位設定補齊新欄位_名稱欄一定顯示且填滿()
    {
        var columns = TaskColumns.Normalize([
            new TaskColumnConfig { Key = "status", Visible = true, Width = 9999 },
            new TaskColumnConfig { Key = "title", Visible = false, Width = 100 },
            new TaskColumnConfig { Key = "unknown", Visible = true, Width = 50 },
            new TaskColumnConfig { Key = "status", Visible = false, Width = 10 },
        ]);
        Assert.Equal(TaskColumns.All.Count, columns.Count);
        Assert.Equal(["status", "title"], columns.Take(2).Select(c => c.Key));
        Assert.Equal(TaskColumns.MaxWidth, columns[0].Width);
        Assert.True(columns[1].Visible);
        Assert.Equal(0, columns[1].Width);

        TaskColumns.Move(columns, "title", -1);
        Assert.Equal("title", columns[0].Key);
        TaskColumns.Move(columns, "title", -1);
        Assert.Equal("title", columns[0].Key);
    }
}
