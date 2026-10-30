using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class ProjectViewsTests
{
    static JsonElement Json(string json) => JsonDocument.Parse(json).RootElement.Clone();

    static ProjectTask Task(string key, string status, string category = "IN_PROGRESS", string[]? assignees = null, string? priority = null) =>
        new(key, key, status, category, null, null, null, false, assignees, priority);

    [Fact]
    public void 視圖_認得的條件在本機套用()
    {
        var views = HayakuApi.ParseViews(Json("""
            {"views":[
              {"id":"v1","name":"列表","type":"列表","isDefault":true,"filter":{"statuses":["處理中","客戶驗證中"]}},
              {"id":"v2","name":"我的","type":"列表","filter":{"assignees":["me"],"priorities":["高"]}},
              {"id":"v3","name":"日期","type":"列表","filter":{"planned":{"from":"2026-10-01"}}},
              {"id":"v4","name":"成員任務","type":"成員任務","filter":{}}]}
            """));
        Assert.Equal(4, views.Count);
        var tasks = new[]
        {
            Task("A", "處理中", assignees: ["me.user"], priority: "高"),
            Task("B", "待處理", "TODO", ["other"]),
            Task("C", "客戶驗證中", assignees: ["other"], priority: "高"),
        };
        Assert.Equal(["A", "C"], ViewFilter.Apply(tasks, views[0], "me.user")!.Select(t => t.Key));
        Assert.Equal(["A"], ViewFilter.Apply(tasks, views[1], "me.user")!.Select(t => t.Key));
        Assert.Null(ViewFilter.Apply(tasks, views[2], "me.user")); // 認不得的條件：交給伺服器
        Assert.Equal(3, ViewFilter.Apply(tasks, views[3], "me.user")!.Count);
    }

    [Fact]
    public void 批次修改結果()
    {
        var sent = new[] { "F2P-1", "F2P-2" };
        var results = HayakuApi.ParseBatchResults(Json("""
            {"results":[{"task":"F2P-1","ok":true,"message":"已送出變更申請"},{"task":"F2P-2","ok":false,"error":"沒有權限"}]}
            """), sent);
        Assert.Equal(("F2P-1", (string?)null, (string?)"已送出變更申請"), (results[0].Task, results[0].Error, results[0].Message));
        Assert.Equal(("F2P-2", (string?)"沒有權限"), (results[1].Task, results[1].Error));
        Assert.All(HayakuApi.ParseBatchResults(Json("{}"), sent), r => Assert.Null(r.Error));
    }
}
