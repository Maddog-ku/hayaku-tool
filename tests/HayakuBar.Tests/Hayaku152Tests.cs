using HayakuBar.Core;
using Drag = HayakuBar.Core.TaskSchedule.Drag;

namespace HayakuBar.Tests;

public class Hayaku152Tests
{
    static DateOnly D(int month, int day) => new(2026, month, day);

    [Fact]
    public void 平移_起訖一起移_天數不變()
    {
        Assert.Equal((D(11, 14), D(11, 17)), TaskSchedule.Apply(D(11, 12), D(11, 15), Drag.Move, 2));
        Assert.Equal((D(11, 9), D(11, 12)), TaskSchedule.Apply(D(11, 12), D(11, 15), Drag.Move, -3));
    }

    [Fact]
    public void 平移_沒填的欄位維持沒填()
    {
        Assert.Equal(((DateOnly?)null, (DateOnly?)D(11, 16)), TaskSchedule.Apply(null, D(11, 15), Drag.Move, 1));
        Assert.Equal(((DateOnly?)D(11, 13), (DateOnly?)null), TaskSchedule.Apply(D(11, 12), null, Drag.Move, 1));
    }

    [Fact]
    public void 拖開始那端_不會越過結束()
    {
        Assert.Equal((D(11, 10), D(11, 15)), TaskSchedule.Apply(D(11, 12), D(11, 15), Drag.Start, -2));
        Assert.Equal((D(11, 15), D(11, 15)), TaskSchedule.Apply(D(11, 12), D(11, 15), Drag.Start, 10));
    }

    [Fact]
    public void 拖結束那端_不會早於開始()
    {
        Assert.Equal((D(11, 12), D(11, 20)), TaskSchedule.Apply(D(11, 12), D(11, 15), Drag.End, 5));
        Assert.Equal((D(11, 12), D(11, 12)), TaskSchedule.Apply(D(11, 12), D(11, 15), Drag.End, -10));
    }

    [Fact]
    public void 只有結束的任務_往前拉開始那端會補上開始()
    {
        Assert.Equal(((DateOnly?)D(11, 13), (DateOnly?)D(11, 15)), TaskSchedule.Apply(null, D(11, 15), Drag.Start, -2));
        // 往後拉擋在結束那天，等於沒動，不要平白補一個開始
        Assert.Equal(((DateOnly?)null, (DateOnly?)D(11, 15)), TaskSchedule.Apply(null, D(11, 15), Drag.Start, 3));
    }

    [Fact]
    public void 沒有日期的任務拖拉不變_放到日期上才設定()
    {
        Assert.Equal(((DateOnly?)null, (DateOnly?)null), TaskSchedule.Apply(null, null, Drag.Move, 3));
        Assert.Equal((D(11, 10), D(11, 14)), TaskSchedule.Place(D(11, 14), D(11, 10)));
    }

    [Fact]
    public void 送出的欄位只放有變的()
    {
        var fields = TaskSchedule.Changes(D(11, 12), D(11, 15), D(11, 12), D(11, 18));
        Assert.Equal(["plannedEnd"], fields.Keys);
        Assert.Equal(D(11, 18), fields["plannedEnd"]);
        Assert.Empty(TaskSchedule.Changes(D(11, 12), D(11, 15), D(11, 12), D(11, 15)));
        Assert.Equal(["plannedStart", "plannedEnd"], TaskSchedule.Changes(null, null, D(11, 3), D(11, 3)).Keys);
    }

    [Fact]
    public void 月曆從1號那週的星期一開始()
    {
        // 2026/11/1 是星期日 → 從 10/26（星期一）開始，到 11/30（星期一）那週，共 6 週
        Assert.Equal((D(10, 26), 6), TaskSchedule.MonthGrid(2026, 11));
        // 2027/2/1 是星期一，剛好 4 週
        Assert.Equal((new DateOnly(2027, 2, 1), 4), TaskSchedule.MonthGrid(2027, 2));
    }

    [Fact]
    public void 一週排列_不重疊的放同一列_跨週的標出來()
    {
        var week = D(11, 9); // 星期一
        var lanes = TaskSchedule.WeekLanes(
        [
            ("A", D(11, 9), D(11, 10)),
            ("B", D(11, 11), D(11, 11)),  // 跟 A 不重疊 → 第 0 列
            ("C", D(11, 10), D(11, 12)),  // 跟 A、B 都重疊 → 第 1 列
            ("D", D(11, 5), D(11, 20)),   // 前後週都還有，最早開始 → 第 0 列
            ("E", D(11, 30), D(11, 30)),  // 不在這週
        ], week);
        var byItem = lanes.ToDictionary(s => s.Item);
        Assert.False(byItem.ContainsKey("E"));
        Assert.Equal(new TaskSchedule.Segment<string>("D", 0, 7, 0, true, true), byItem["D"]);
        Assert.Equal(1, byItem["A"].Lane);
        Assert.Equal(1, byItem["B"].Lane);
        Assert.Equal(2, byItem["C"].Lane);
        Assert.Equal((1, 3), (byItem["C"].Column, byItem["C"].Days));
    }

    [Fact]
    public void 只有一端的任務當作一天()
    {
        Assert.Equal((D(11, 5), D(11, 5)), TaskSchedule.Span(null, D(11, 5)));
        Assert.Equal((D(11, 5), D(11, 5)), TaskSchedule.Span(D(11, 5), null));
        Assert.Null(TaskSchedule.Span(null, null));
        Assert.Equal("11/5 結束（開始未指定）", TaskSchedule.RangeText(null, D(11, 5)));
    }
}
