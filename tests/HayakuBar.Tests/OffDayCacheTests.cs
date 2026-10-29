using HayakuBar.Core;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class OffDayCacheTests
{
    static readonly IReadOnlyList<WorkSegment> Normal = [new(new(8, 30), new(17, 30))];
    static readonly DateOnly Thu = new(2026, 10, 29);

    [Fact]
    public void From_只留今天之後沒有上班時段的日子()
    {
        var days = new List<DaySchedule>
        {
            new(Thu, [], 0, true, null),                     // 今天：不記
            new(Thu.AddDays(1), Normal, 480, false, null),   // 上班
            new(Thu.AddDays(2), [], 0, false, null),         // 週六
            new(Thu.AddDays(4), [], 0, true, null),          // 請假
            new(Thu.AddDays(5), [], 0, false, "國慶日"),
            new(Thu.AddDays(6), Normal, 240, true, null),    // 請半天：還是要上班
        };

        Assert.Equal(new Dictionary<string, string>
        {
            ["2026-10-31"] = "",
            ["2026-11-02"] = "請假",
            ["2026-11-03"] = "國慶日",
        }, OffDayCache.From(days, Thu));
    }

    [Fact]
    public void Today_還原成沒有上班時段的班表()
    {
        var off = new Dictionary<string, string> { ["2026-10-29"] = "請假", ["2026-10-30"] = "國慶日", ["2026-10-31"] = "" };

        var leave = OffDayCache.Today(off, Thu)!;
        Assert.Equal(Thu, leave.Date);
        Assert.Empty(leave.Working);
        Assert.True(leave.OnLeave);
        Assert.Equal("國慶日", OffDayCache.Today(off, Thu.AddDays(1))!.Holiday);
        var weekend = OffDayCache.Today(off, Thu.AddDays(2))!;
        Assert.Empty(weekend.Working);
        Assert.False(weekend.OnLeave);
        Assert.Null(weekend.Holiday);
        Assert.Null(OffDayCache.Today(off, Thu.AddDays(3)));
    }
}
