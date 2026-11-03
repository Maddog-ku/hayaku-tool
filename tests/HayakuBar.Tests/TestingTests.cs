using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;
using HayakuBar.UI;
using HayakuBar.UI.Work;

namespace HayakuBar.Tests;

/// <summary>
/// Hayaku 1.3 的新工具與欄位：測試流程（類型、測試結果、三種視圖）、read_file、已完成分頁、關聯任務、托盤的會議、
/// 空間的所屬專案、工時填滿的算法、從 Excel 貼上的表格。
/// 我的任務、已完成、測試視圖的空回應取自實際回應；測試結果有內容的回應還沒有實際樣本，照工具說明寫寬鬆的解析。
/// </summary>
public class TestingTests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement;

    [Theory]
    [InlineData(null, TaskKind.Normal)]
    [InlineData("一般", TaskKind.Normal)]
    [InlineData("NORMAL", TaskKind.Normal)]
    [InlineData("測試", TaskKind.Test)]
    [InlineData("TEST", TaskKind.Test)]
    [InlineData("測試結果", TaskKind.TestResult)]
    [InlineData("TEST_RESULT", TaskKind.TestResult)]
    [InlineData("test_result", TaskKind.TestResult)]
    public void TaskKinds_Parse_代碼或中文都認得(string? value, TaskKind expected) =>
        Assert.Equal(expected, TaskKinds.Parse(value));

    [Theory]
    [InlineData("待修復", "fixing")]
    [InlineData("修復中", "fixing")]
    [InlineData("待復驗", "retest")]
    [InlineData("未開始", "testing")]
    [InlineData("測試中", "testing")]
    [InlineData("已完成", "pass")]
    [InlineData("不處理", "closed")]
    public void TaskKinds_Bucket_測試結果的狀態分組(string status, string bucket) =>
        Assert.Equal(bucket, TaskKinds.Bucket(status));

    [Fact]
    public void ParseTaskSummaries_一般任務沒有類型欄位_測試結果讀分類嚴重度輪數()
    {
        var list = HayakuApi.ParseTaskSummaries(Json("""
            {"tasks":[
              {"task":"F2P-129","title":"動態調整優化","project":"F2P-臉部辨識結合印表機","status":"處理中","statusCategory":"IN_PROGRESS",
               "priority":"無","plannedStart":"2026-11-02","plannedEnd":"2026-11-02","approvalState":"APPROVED","timing":true,
               "path":[{"task":"F2P-5","title":"第一階段"}],"childCount":0},
              {"task":"F2P-140","title":"登入頁跑版","project":"F2P","status":"待修復","statusCategory":"TODO","type":"測試結果",
               "category":"Bug","severity":"嚴重","round":3,"reporter":{"username":"qa","displayName":"小美"},
               "path":[{"task":"F2P-139","title":"第二階段測試"}],"childCount":0},
              {"task":"F2P-139","title":"第二階段測試","project":"F2P","status":"測試中","statusCategory":"IN_PROGRESS","type":"TEST"}
            ],"cursor":null,"timezone":"Asia/Taipei"}
            """));
        Assert.Equal(TaskKind.Normal, list[0].Kind);
        Assert.Equal((TaskKind.TestResult, "Bug", "嚴重", 3, "小美"),
            (list[1].Kind, list[1].Test.Category, list[1].Test.Severity, list[1].Test.Round, list[1].Test.Reporter));
        Assert.True(list[1].Test.Hot);
        Assert.Equal(TaskKind.Test, list[2].Kind);
    }

    [Fact]
    public void MyTaskTabs_測試分頁放測試任務與測試結果()
    {
        var open = new List<TaskSummary>
        {
            new("A-1", "一般", "A", "處理中", "IN_PROGRESS", "無", null, null, false, ""),
            new("A-2", "測試", "A", "測試中", "IN_PROGRESS", "無", null, null, false, "", Test: new TestInfo(TaskKind.Test, null, null, 0, null)),
            new("A-3", "問題", "A", "待修復", "TODO", "無", null, null, false, "", Test: new TestInfo(TaskKind.TestResult, "Bug", "一般", 1, null)),
        };
        var tabs = MyTaskTabs.Split(open, new DateOnly(2026, 11, 2));
        Assert.Equal(["A-2", "A-3"], tabs["test"].Select(t => t.Key));
        Assert.Contains(tabs["in_progress"], t => t.Key == "A-2"); // 其他分頁照樣列出
    }

    [Fact]
    public void TaskText_測試結果加圖示與輪數()
    {
        Assert.Equal("一般任務", TaskText.Title("一般任務", default));
        Assert.Equal("🧪 回歸測試", TaskText.Title("回歸測試", new TestInfo(TaskKind.Test, null, null, 0, null)));
        Assert.Equal("🐞 跑版", TaskText.Title("跑版", new TestInfo(TaskKind.TestResult, null, null, 1, null)));
        Assert.Equal("🐞 跑版（第 2 輪）", TaskText.Title("跑版", new TestInfo(TaskKind.TestResult, null, null, 2, null)));
        Assert.Equal("🐞 跑版（第 3 輪 🔥）", TaskText.Title("跑版", new TestInfo(TaskKind.TestResult, null, null, 3, null)));
    }

    [Fact]
    public void ParseTestRows_空的視圖_取自實際回應()
    {
        Assert.Empty(HayakuApi.ParseTestRows(Json("""{"view":"需修復任務","count":0,"truncated":false,"results":[],"timezone":"Asia/Taipei"}""")));
        Assert.Empty(HayakuApi.ParseTestRows(Json("""{"view":"需測試任務","retest":[],"toTest":[],"tests":[],"truncated":false,"timezone":"Asia/Taipei"}""")));
    }

    [Fact]
    public void ParseTestRows_需測試三組照順序_測試任務從上層讀()
    {
        var rows = HayakuApi.ParseTestRows(Json("""
            {"view":"需測試任務",
             "retest":[{"task":"F2P-141","title":"按鈕沒反應","status":"待復驗","severity":"一般","round":1,
                        "assignees":[{"username":"harry.gu","displayName":"古信汯"}],"testTask":{"task":"F2P-139","title":"第二階段測試"}}],
             "toTest":[{"task":"F2P-142","title":"列印失敗","status":"未開始","parent":"F2P-139"}],
             "tests":[{"task":"F2P-139","title":"第二階段測試","status":"測試中"}]}
            """));
        Assert.Equal(["retest", "toTest", "tests"], rows.Select(r => r.Group));
        Assert.Equal(("F2P-139", "第二階段測試", "古信汯"), (rows[0].TestTask, rows[0].TestTaskTitle, rows[0].Assignees[0]));
        Assert.Equal("F2P-139", rows[1].TestTask);
        Assert.Equal(TaskKind.TestResult, rows[1].Kind);
        Assert.Equal(TaskKind.Test, rows[2].Kind);
    }

    [Fact]
    public void ParseTestOverview_讀卡片數字_欄位同實際回應()
    {
        var o = HayakuApi.ParseTestOverview(Json("""
            {"view":"測試總覽","from":"2026-11-02","to":"2026-11-02",
             "cards":{"open":{"total":5,"critical":1,"major":2},"retest":{"total":2,"oldestDays":3},"created":{"count":7,"previous":null},
                      "closed":{"done":4,"wontfix":1},"avgRound":{"value":1.5,"round3Plus":1},"stale":2},
             "bySeverity":{"致命":1,"嚴重":2,"一般":2,"輕微":0},"byCategory":{"待判斷":{},"Bug":{},"需求變更":{}},
             "tests":[{"task":"F2P-139","title":"第二階段測試","counts":{"total":6,"passed":3,"fixing":2,"testing":1}}],
             "stale":{"total":1,"items":[{"task":"F2P-150","title":"很久沒人修","status":"待修復"}]},"projects":[],"timezone":"Asia/Taipei"}
            """));
        Assert.Equal((5, 1, 2, 2, 3, 7, 4, 1, 1.5, 1, 2), (o.Open, o.Critical, o.Major, o.Retest, o.OldestRetestDays, o.Created, o.Done, o.WontFix,
            o.AvgRound, o.Round3Plus, o.Stale));
        Assert.Equal(("致命", 1), o.BySeverity[0]);
        Assert.Equal((6, 3, 2, 1), (o.Tests[0].Total, o.Tests[0].Passed, o.Tests[0].Fixing, o.Tests[0].Testing));
        Assert.Equal("F2P-150", o.StaleItems.Single().Key);
    }

    [Fact]
    public void ParseTestOverview_測試任務的筆數讀results的byStatus_取自實際回應()
    {
        var o = HayakuApi.ParseTestOverview(Json("""
            {"view":"測試總覽","cards":{},
             "tests":[{"task":"F2P-149","title":"bug修復","status":"測試中","parent":"F2P-50 第四階段","assignees":[],"plannedEnd":null,
                       "results":{"total":5,"open":3,"byStatus":{"已完成":1,"待修復":1,"修復中":1,"待復驗":1,"不處理":1}},
                       "avgRound":1,"oldestOpenDays":null,"stale":0},
                      {"task":"F2P-146","title":"bug修復","status":"未開始","results":{"total":0,"open":0,"byStatus":{}}}],
             "stale":{"total":0,"items":[]},"timezone":"Asia/Taipei"}
            """));
        Assert.Equal((5, 1, 2, 0, 1), (o.Tests[0].Total, o.Tests[0].Passed, o.Tests[0].Fixing, o.Tests[0].Testing, o.Tests[0].Retest));
        Assert.Equal((0, 0), (o.Tests[1].Total, o.Tests[1].Passed));
    }

    [Fact]
    public void ParseTestOverview_沒有資料時平均輪數是null_取自實際回應()
    {
        var o = HayakuApi.ParseTestOverview(Json("""
            {"view":"測試總覽","from":"2026-11-02","to":"2026-11-02",
             "cards":{"open":{"total":0,"critical":0,"major":0},"retest":{"total":0,"oldestDays":null},"created":{"count":0,"previous":null},
                      "closed":{"done":0,"wontfix":0},"avgRound":{"value":null,"round3Plus":0},"stale":0},
             "bySeverity":{"致命":0,"嚴重":0,"一般":0,"輕微":0},"byCategory":{"待判斷":{},"Bug":{},"需求變更":{}},
             "tests":[],"stale":{"total":0,"items":[]},"projects":[],"timezone":"Asia/Taipei"}
            """));
        Assert.Null(o.AvgRound);
        Assert.Null(o.OldestRetestDays);
        Assert.Equal(4, o.BySeverity.Count);
        Assert.Empty(o.Tests);
    }

    [Fact]
    public void ParseFileContent_圖片回圖_文件回文字與下一段()
    {
        var image = HayakuApi.ParseFileContent(Json("""
            {"content":[{"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"},{"type":"text","text":"截圖.png（縮到 1568 px）"}]}
            """), "a1");
        Assert.NotNull(image.Image);
        Assert.Equal("image/png", image.MimeType);
        Assert.Null(image.Text);
        Assert.Equal("截圖.png（縮到 1568 px）", image.Note);

        var doc = HayakuApi.ParseFileContent(Json("""
            {"content":[{"type":"text","text":"{\"name\":\"規格.pdf\",\"text\":\"第 1 頁\\n內容\",\"nextOffset\":50000}"}]}
            """), "a2");
        Assert.Equal(("規格.pdf", "第 1 頁\n內容", 50000), (doc.Name, doc.Text, doc.NextOffset));

        var last = HayakuApi.ParseFileContent(Json("""{"structuredContent":{"text":"最後一段","nextOffset":null},"content":[]}"""), "a4");
        Assert.Equal(("最後一段", (int?)null), (last.Text, last.NextOffset));

        var plain = HayakuApi.ParseFileContent(Json("""{"content":[{"type":"text","text":"a,b\n1,2"}]}"""), "a3");
        Assert.Equal("a,b\n1,2", plain.Text);
        Assert.Null(plain.NextOffset);
    }

    [Fact]
    public void ParsePins_會議也讀得到_已取消的不能加入()
    {
        var pins = HayakuApi.ParsePins(Json("""
            {"today":"2026-11-02","pins":[
              {"task":"F2P-126","title":"任務","project":"F2P","status":"處理中","today":{"minutes":5},"running":false,"canTime":true},
              {"meeting":"REVIVAL-9","kind":"MEETING","title":"週會","project":"REVIVAL","startAt":"2026-11-02T06:00:00Z","endAt":"2026-11-02T07:00:00Z"},
              {"task":"REVIVAL-10","kind":"MEETING","title":"取消的會","startAt":"2026-11-02T08:00:00Z","endAt":"2026-11-02T09:00:00Z","status":"已取消"}
            ],"timezone":"Asia/Taipei"}
            """));
        Assert.Equal(3, pins.Count);
        Assert.False(pins[0].IsMeeting);
        Assert.True(pins[1].IsMeeting);
        Assert.Equal("REVIVAL-9", pins[1].Key);
        var start = DateTimeOffset.Parse("2026-11-02T06:00:00Z");
        Assert.False(pins[1].CanJoin(start.AddMinutes(-11)));
        Assert.True(pins[1].CanJoin(start.AddMinutes(-10)));
        Assert.True(pins[1].CanJoin(start.AddMinutes(30)));
        Assert.False(pins[1].CanJoin(start.AddMinutes(60))); // 已結束
        Assert.True(pins[2].Cancelled);
        Assert.False(pins[2].CanJoin(DateTimeOffset.Parse("2026-11-02T08:30:00Z")));
        Assert.Empty(HayakuApi.ParsePins(Json("""{"today":"2026-11-02","pins":[],"timezone":"Asia/Taipei"}""")));
    }

    [Fact]
    public void ParseSpaces_讀所屬專案與封存()
    {
        var spaces = HayakuApi.ParseSpaces(Json("""
            {"spaces":[{"key":"RULES","name":"公司規章","role":"EDITOR","pageCount":12},
                       {"key":"F2PDOC","name":"F2P 文件","role":"MEMBER","pageCount":3,"project":{"key":"F2P","name":"F2P"},"archived":true}]}
            """));
        Assert.Null(spaces[0].Project);
        Assert.Equal(("F2P", true), (spaces[1].Project, spaces[1].Archived));
    }

    [Fact]
    public void ParseDependencies_關聯任務另外放()
    {
        var deps = HayakuApi.ParseDependencies(Json("""
            {"predecessors":[{"id":"d1","task":{"task":"F2P-1","title":"前置"},"template":"finishToStart"},
                             {"kind":"related","task":{"task":"EG-3","title":"跨專案的"}}],
             "successors":[],"related":[{"task":"F2P-7","title":"相關的","status":"處理中"}],"timezone":"Asia/Taipei"}
            """));
        Assert.Equal("F2P-1", deps.Predecessors.Single().OtherKey);
        Assert.Equal(["F2P-7", "EG-3"], deps.RelatedTasks.Select(d => d.OtherKey));
        Assert.Empty(HayakuApi.ParseDependencies(Json("""{"predecessors":[],"successors":[],"timezone":"Asia/Taipei"}""")).RelatedTasks);
    }

    [Fact]
    public void ParseNotifications_測試結果與問題回報()
    {
        var list = HayakuApi.ParseNotifications(Json("""
            {"items":[
              {"latest":{"id":"n1","type":"TEST_RESULT_REOPENED","project":{"key":"F2P"},"task":{"number":140,"title":"登入頁跑版"},"readAt":null}},
              {"latest":{"id":"n2","type":"TEST_RESULT_RETEST","project":{"key":"F2P"},"task":{"number":141,"title":"按鈕沒反應"},"readAt":null}},
              {"latest":{"id":"n3","type":"FEEDBACK_NEW","payload":{"number":128,"title":"匯出失敗"},"readAt":null}}
            ]}
            """));
        Assert.True(list[0].IsReopened);
        Assert.Equal(("F2P-140", "測試結果被退回"), (list[0].TaskKey, list[0].Label));
        Assert.True(list[1].IsReadyToRetest);
        Assert.Equal("測試結果待復驗", list[1].Label);
        Assert.Equal((128, "匯出失敗"), (list[2].Feedback, list[2].Text));
        Assert.Equal(("feedback", (object?)128), TimerEngine.NotificationTarget(list[2]));
    }

    [Theory]
    [InlineData(479.6, 480, true)]   // 差不到半分鐘：算填滿
    [InlineData(479.5, 480, false)]  // 剛好差半分鐘：不算
    [InlineData(479.2, 480, false)]
    [InlineData(478.8, 480, false)]  // 7.98 ／ 8：差 1 分多，照樣標不足
    [InlineData(0, 0, true)]         // 不用上班
    public void WorkMath_IsFilled_差不到半分鐘就算填滿(double worked, double expected, bool filled)
    {
        Assert.Equal(filled, WorkMath.IsFilled(worked, expected));
        Assert.Equal(filled ? 0 : expected - worked, WorkMath.Shortfall(worked, expected), 3);
    }

    [Fact]
    public void MarkdownTable_FromTsv_Excel貼上變表格()
    {
        Assert.Equal("""
            | 名稱 | 數量 |
            | --- | --- |
            | 蘋果 | 3 |
            | a\|b | 換<br>行 |
            """.Replace("\r\n", "\n"), MarkdownTable.FromTsv("名稱\t數量\r\n蘋果\t3\r\na|b\t\"換\n行\"\r\n"));
        Assert.Null(MarkdownTable.FromTsv("沒有 tab 的一般文字"));
        Assert.Null(MarkdownTable.FromTsv(""));
    }

    [Fact]
    public void MarkdownTable_Template_空白表格()
    {
        var lines = MarkdownTable.Template(2, 1).Split('\n');
        Assert.Equal(["| 欄位 1 | 欄位 2 |", "| --- | --- |", "|  |  |"], lines);
    }
}
