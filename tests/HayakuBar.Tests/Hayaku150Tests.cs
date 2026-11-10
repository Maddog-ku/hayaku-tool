using System.Text.Json;
using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

/// <summary>
/// Hayaku 1.5.0：子任務範本（list_subtask_templates、apply_subtask_template）、專案頁只顯示已逾期。
/// 帳號目前沒有範本，實際回應只有 {"templates":[]}；其餘範本與套用結果的格式是照工具說明寬鬆猜的。
/// </summary>
public class Hayaku150Tests
{
    static JsonElement Json(string s) => JsonDocument.Parse(s).RootElement;

    [Fact]
    public void ParseSubtaskTemplates_空清單()
    {
        Assert.Empty(HayakuApi.ParseSubtaskTemplates(Json("""{"templates":[],"timezone":"Asia/Taipei"}""")));
    }

    [Fact]
    public void ParseSubtaskTemplates_讀名稱專案與任務數()
    {
        var list = HayakuApi.ParseSubtaskTemplates(Json("""
            {"templates":[
              {"id":12,"name":"新功能開發","project":{"key":"F2P","name":"機密列印"},"items":[
                {"title":"需求確認"},{"title":"開發","children":[{"title":"前端"},{"title":"後端"}]}]},
              {"id":"t-2","name":"上線檢查","project":null,"count":5,"description":"上正式機前"}]}
            """));
        Assert.Equal(2, list.Count);
        Assert.Equal(("12", "新功能開發", "F2P", 4), (list[0].Id, list[0].Name, list[0].Project, list[0].Count!.Value));
        Assert.Equal("新功能開發（4 個任務） · F2P", list[0].Label);
        Assert.Equal(("t-2", null, 5, "上正式機前"), (list[1].Id, list[1].Project, list[1].Count, list[1].Description));
        Assert.Equal("上線檢查（5 個任務） · 全站", list[1].Label);
    }

    [Fact]
    public void ParseTemplateApply_每個任務一行()
    {
        var r = HayakuApi.ParseTemplateApply(Json("""
            {"results":[
              {"task":"F2P-5","created":[{"title":"需求確認","plannedStart":"2026-11-12","plannedEnd":"2026-11-13"},{"title":"開發","children":[{"title":"前端"}]}]},
              {"task":"F2P-6","skipped":true,"reason":"已經有子任務"},
              {"task":"F2P-7","error":"沒有權限"}]}
            """));
        Assert.Equal(["F2P-5：3 個子任務（需求確認 11/12～11/13、開發）", "F2P-6：略過（已經有子任務）", "F2P-7：失敗：沒有權限"], r.Lines);
    }

    [Fact]
    public void ParseTemplateApply_認不出格式時留原文()
    {
        var r = HayakuApi.ParseTemplateApply(Json("""{"message":"已送出，等待核准"}"""));
        Assert.Empty(r.Lines);
        Assert.Equal("已送出，等待核准", r.Message);
        Assert.Contains("等待核准", r.Raw);
    }

    [Fact]
    public void IsOverdue_預計結束已過且沒完成()
    {
        var today = new DateOnly(2026, 11, 10);
        ProjectTask T(string category, DateOnly? end, bool isCategory = false) =>
            new("F2P-1", "x", "處理中", category, null, null, end, isCategory);
        Assert.True(TaskTree.IsOverdue(T("IN_PROGRESS", today.AddDays(-1)), today));
        Assert.False(TaskTree.IsOverdue(T("IN_PROGRESS", today), today));
        Assert.False(TaskTree.IsOverdue(T("DONE", today.AddDays(-1)), today));
        Assert.False(TaskTree.IsOverdue(T("TODO", null), today));
        Assert.False(TaskTree.IsOverdue(T("TODO", today.AddDays(-3), isCategory: true), today));
    }
}
