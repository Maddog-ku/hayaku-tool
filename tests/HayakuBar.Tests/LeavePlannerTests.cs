using System.Text.Json;
using HayakuBar.Eip;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class LeavePlannerTests
{
    static readonly IReadOnlyList<WorkSegment> Normal = [new(new(8, 30), new(12, 30)), new(new(13, 30), new(17, 30))];

    static DaySchedule Work(DateOnly d) => new(d, Normal, 480, false, null);
    static DaySchedule Off(DateOnly d, string? holiday = null) => new(d, [], 0, false, holiday);
    static DaySchedule OnLeave(DateOnly d) => new(d, [], 0, true, null);

    static EipLeave Leave(string no, DateTime start, DateTime end, string kind = "請假", string type = "特休") =>
        new(no, "Y2798", kind, type, "", start, end, "核准", null);

    // 2026-10-29 是星期四
    static readonly DateOnly Thu = new(2026, 10, 29);
    static readonly DateOnly Fri = Thu.AddDays(1);
    static readonly DateOnly Mon = Thu.AddDays(4);

    /// <summary>週四～下下週一的班表：平日上班、週末不上班。</summary>
    static List<DaySchedule> TwoWeeks() =>
        Enumerable.Range(0, 12).Select(i => Thu.AddDays(i))
            .Select(d => d.DayOfWeek is DayOfWeek.Saturday or DayOfWeek.Sunday ? Off(d) : Work(d))
            .ToList();

    [Fact]
    public void Plan_涵蓋整個上班時段_登記整天()
    {
        var pieces = LeavePlanner.Plan(Leave("A", new(2026, 10, 30, 8, 30, 0), new(2026, 10, 30, 17, 30, 0)), LeavePlanner.Working(TwoWeeks()));

        var p = Assert.Single(pieces);
        Assert.Equal(new LeavePiece("DAY", Fri), p);
        Assert.Equal(new DateTime(2026, 10, 30), p.From);
        Assert.Equal(new DateTime(2026, 10, 31), p.To);
    }

    [Fact]
    public void Plan_只請幾小時_登記指定時段()
    {
        var p = Assert.Single(LeavePlanner.Plan(Leave("A", new(2026, 10, 30, 15, 30, 0), new(2026, 10, 30, 17, 30, 0)), LeavePlanner.Working(TwoWeeks())));

        Assert.Equal(new LeavePiece("RANGE", Fri, Start: new(15, 30), End: new(17, 30)), p);
        Assert.Equal(new DateTime(2026, 10, 30, 15, 30, 0), p.From);
        Assert.Equal(new DateTime(2026, 10, 30, 17, 30, 0), p.To);
    }

    [Fact]
    public void Plan_跨週末的整天_合併成一筆()
    {
        var pieces = LeavePlanner.Plan(Leave("A", new(2026, 10, 30, 8, 30, 0), new(2026, 11, 2, 17, 30, 0)), LeavePlanner.Working(TwoWeeks()));

        Assert.Equal([new LeavePiece("DAY", Fri, Mon)], pieces);
    }

    [Fact]
    public void Plan_第一天下午開始_拆成時段加整天()
    {
        var pieces = LeavePlanner.Plan(Leave("A", new(2026, 11, 2, 13, 30, 0), new(2026, 11, 4, 17, 30, 0)), LeavePlanner.Working(TwoWeeks()));

        Assert.Equal([
            new LeavePiece("RANGE", Mon, Start: new(13, 30), End: new(17, 30)),
            new LeavePiece("DAY", Mon.AddDays(1), Mon.AddDays(2)),
        ], pieces);
    }

    [Fact]
    public void Plan_只落在週末_不登記()
    {
        Assert.Empty(LeavePlanner.Plan(Leave("A", new(2026, 10, 31, 8, 30, 0), new(2026, 10, 31, 17, 30, 0)), LeavePlanner.Working(TwoWeeks())));
    }

    [Fact]
    public void Working_已請假的日子用平常時段_假日與範圍外週末是空的()
    {
        var days = new List<DaySchedule> { Work(Thu), OnLeave(Fri), Off(Mon, "國慶日") };
        var working = LeavePlanner.Working(days);

        Assert.Equal(Normal, working(Fri));
        Assert.Empty(working(Mon));
        Assert.Equal(Normal, working(Mon.AddDays(1))); // 範圍外的平日
        Assert.Empty(working(Thu.AddDays(2)));          // 範圍外的週六
    }

    [Fact]
    public void Effective_只留請假與補假_被銷假蓋掉的不算_同單號一次()
    {
        var rows = new[]
        {
            Leave("A", new(2026, 10, 30, 8, 30, 0), new(2026, 10, 30, 17, 30, 0)),
            Leave("A", new(2026, 10, 30, 8, 30, 0), new(2026, 10, 30, 17, 30, 0)),
            Leave("B", new(2026, 11, 2, 8, 30, 0), new(2026, 11, 2, 17, 30, 0)),
            Leave("C", new(2026, 11, 2, 8, 0, 0), new(2026, 11, 2, 18, 0, 0), kind: "銷假"),
            Leave("D", new(2026, 11, 3, 8, 30, 0), new(2026, 11, 3, 10, 30, 0), kind: "補假"),
            Leave("E", new(2026, 11, 4, 8, 30, 0), new(2026, 11, 4, 9, 0, 0), kind: "黃卡"),
        };

        Assert.Equal(["A", "D"], LeavePlanner.Effective(rows).Select(l => l.No));
    }

    [Fact]
    public void NextWorkday_週四是週五_週五跳過週末是週一_國定假日也跳過()
    {
        var days = TwoWeeks();
        Assert.Equal(Fri, LeavePlanner.NextWorkday(days, Thu));
        Assert.Equal(Mon, LeavePlanner.NextWorkday(days, Fri));

        days[days.FindIndex(d => d.Date == Mon)] = Off(Mon, "補假");
        Assert.Equal(Mon.AddDays(1), LeavePlanner.NextWorkday(days, Fri));
    }

    [Fact]
    public void NextWorkday_已經請假的工作日也算()
    {
        Assert.Equal(Fri, LeavePlanner.NextWorkday([Work(Thu), OnLeave(Fri)], Thu));
    }

    [Theory]
    [InlineData("2026-10-29 17:00", "2026-10-28", false, false)] // 還沒下班
    [InlineData("2026-10-29 17:30", "2026-10-28", false, true)]  // 下班時間到
    [InlineData("2026-10-29 16:00", "2026-10-28", true, true)]   // 提早按了下班
    [InlineData("2026-10-29 18:00", "2026-10-29", false, false)] // 今天已經同步過
    [InlineData("2026-10-29 09:00", "2026-10-27", false, true)]  // 前天以前就漏了：開機補做
    [InlineData("2026-10-29 09:00", "", false, true)]            // 從沒同步過
    public void IsDue_一天一次_下班時(string now, string last, bool clockedOut, bool due)
    {
        Assert.Equal(due, LeavePlanner.IsDue(Work(Thu), clockedOut, DateTime.Parse(now), last));
    }

    [Theory]
    [InlineData("2026-10-29 09:00", false, "2026-10-28")] // 上班前補做：算昨天的，下班時還會再做
    [InlineData("2026-10-29 17:30", false, "2026-10-29")]
    [InlineData("2026-10-29 15:00", true, "2026-10-29")]  // 提早按了下班
    public void CoveredDay_下班後才算今天(string now, bool clockedOut, string day)
    {
        Assert.Equal(DateOnly.Parse(day), LeavePlanner.CoveredDay(Work(Thu), clockedOut, DateTime.Parse(now)));
    }

    [Fact]
    public void IsDue_今天不用上班_請假放假週末都不做()
    {
        Assert.False(LeavePlanner.IsDue(Off(Thu.AddDays(2)), false, new DateTime(2026, 10, 31, 18, 0, 0), "2026-10-30"));
        Assert.False(LeavePlanner.IsDue(OnLeave(Mon), true, new DateTime(2026, 11, 2, 18, 0, 0), "2026-10-23")); // 漏了很久也一樣
        Assert.False(LeavePlanner.IsDue(Off(Fri, "國慶日"), false, new DateTime(2026, 10, 30, 18, 0, 0), ""));
    }

    [Fact]
    public void LeavePiece_參數與說明()
    {
        var day = new LeavePiece("DAY", Fri, Mon);
        Assert.Equal(new Dictionary<string, object?> { ["span"] = "DAY", ["date"] = "2026-10-30", ["endDate"] = "2026-11-02", ["note"] = "EIP A 特休" },
            day.Args("EIP A 特休"));
        Assert.Equal("10/30（週五） 到 11/2（週一） 整天", day.Describe());

        var range = new LeavePiece("RANGE", Fri, Start: new(15, 30), End: new(17, 30));
        Assert.Equal(new Dictionary<string, object?> { ["span"] = "RANGE", ["date"] = "2026-10-30", ["start"] = "15:30", ["end"] = "17:30" },
            range.Args(""));
        Assert.Equal("10/30（週五） 15:30–17:30", range.Describe());
        Assert.Equal(500, ((string)range.Args(new string('x', 600))["note"]!).Length);
    }

    [Fact]
    public void CreatedId_支援id或leave物件()
    {
        Assert.Equal("a", HayakuApi.CreatedId(JsonSerializer.SerializeToElement(new { id = "a" })));
        Assert.Equal("b", HayakuApi.CreatedId(JsonSerializer.SerializeToElement(new { leave = new { id = "b" } })));
        Assert.Null(HayakuApi.CreatedId(JsonSerializer.SerializeToElement("已登記")));
        Assert.Null(HayakuApi.CreatedId(default));
    }
}
