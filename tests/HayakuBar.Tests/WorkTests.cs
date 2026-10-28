using System.Text.Json;
using HayakuBar.Mcp;
using HayakuBar.UI.Work;

namespace HayakuBar.Tests;

/// <summary>工作台用到的解析；樣本取自實際 Hayaku MCP 回應。</summary>
public class WorkParseTests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement;

    [Fact]
    public void ParseTaskSummaries_讀取路徑與日期()
    {
        var list = HayakuApi.ParseTaskSummaries(Json("""
            {"tab":"逾期","tasks":[{"task":"F2P-15","title":"事務機APP開發","project":"F2P-臉部辨識結合印表機","status":"處理中",
              "statusCategory":"IN_PROGRESS","priority":"無","plannedStart":"2026-10-21","plannedEnd":"2026-10-23","timing":true,
              "path":[{"task":"F2P-5","title":"第一階段研究"},{"task":"F2P-14","title":"機密列印"}]}]}
            """));
        var t = Assert.Single(list);
        Assert.Equal("F2P-15", t.Key);
        Assert.Equal("第一階段研究 › 機密列印", t.Path);
        Assert.Equal(new DateOnly(2026, 10, 23), t.PlannedEnd);
        Assert.True(t.Timing);
    }

    [Fact]
    public void ParseTaskDetail_讀取執行者工時與子任務()
    {
        var d = HayakuApi.ParseTaskDetail(Json("""
            {"task":"F2P-97","title":"報表頁面新增歸0按鈕","status":"客戶驗證中","statusCategory":"IN_PROGRESS","priority":"無",
             "assignees":[{"username":"harry.gu","displayName":"古信汯"}],"plannedStart":"2026-10-27","plannedEnd":"2026-10-28",
             "actualStart":"2026-10-27","actualEnd":null,"estimated":null,"logged":{"minutes":215,"display":"3.58 小時"},
             "note":null,"approvalState":"APPROVED","pendingChangeRequests":[],"path":[{"task":"F2P-5","title":"第一階段研究"}],
             "children":[{"task":"F2P-98","title":"子","status":"未開始"}]}
            """));
        Assert.Equal("古信汯", Assert.Single(d.Assignees).DisplayName);
        Assert.Equal("3.58 小時", d.LoggedText);
        Assert.Equal(215, d.LoggedMinutes);
        Assert.Null(d.EstimatedHours);
        Assert.Null(d.ActualEnd);
        Assert.Equal("F2P-98", Assert.Single(d.Children).Key);
        Assert.Equal(0, d.PendingChangeRequests);
    }

    [Theory]
    [InlineData("""{"rule":{"freq":"WEEKLY","interval":1,"weekdays":[3]},"timeOfDay":"09:00","pausedAt":null}""", "每週四 09:00")]
    [InlineData("""{"rule":{"freq":"DAILY","interval":1,"workdaysOnly":true},"timeOfDay":"08:30"}""", "每個工作日 08:30")]
    [InlineData("""{"rule":{"freq":"MONTHLY","interval":1,"monthDay":5},"timeOfDay":"10:00"}""", "每月 5 日 10:00")]
    public void RepeatText_轉成中文(string json, string expected) =>
        Assert.Equal(expected, HayakuApi.RepeatText(Json(json)));

    [Fact]
    public void ParseDate_支援常見寫法()
    {
        var today = new DateOnly(2026, 10, 28);
        Assert.Equal(today, Inputs.ParseDate("今天", today));
        Assert.Equal(today.AddDays(1), Inputs.ParseDate("明天", today));
        Assert.Equal(today.AddDays(3), Inputs.ParseDate("+3", today));
        Assert.Equal(new DateOnly(2026, 11, 5), Inputs.ParseDate("11/5", today));
        Assert.Equal(new DateOnly(2026, 11, 5), Inputs.ParseDate("2026-11-05", today));
        Assert.Null(Inputs.ParseDate("下下週", today));
        Assert.Null(Inputs.ParseDate("", today));
    }

    [Fact]
    public void ParseTime_支援冒號與四位數()
    {
        Assert.Equal(new TimeOnly(9, 30), Inputs.ParseTime("9:30"));
        Assert.Equal(new TimeOnly(14, 0), Inputs.ParseTime("1400"));
        Assert.Null(Inputs.ParseTime("25:00"));
    }
}
