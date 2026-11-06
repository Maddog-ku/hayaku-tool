using HayakuBar.Mcp;

namespace HayakuBar.Tests;

/// <summary>
/// Hayaku 1.4.2：內建的修復中、待復驗、已完成固定排在每段第一列，專案不用再自己建同名的狀態。
/// 回應有沒有列出內建那一列（1.4.0 的實際回應有列）都要算得一樣。
/// </summary>
public class Hayaku142Tests
{
    static readonly TestSubStatus[] WithBuiltIn =
    [
        new("修復中", "修復中", null, null),
        new("修復中", "待更新測試機", "更新到測試機", null),
        new("待復驗", "待復驗", null, null),
        new("待復驗", "待上正式機", "更新到正式機", null),
    ];

    static readonly TestSubStatus[] CustomOnly = WithBuiltIn.Where(s => s.Name != s.Stage).ToArray();

    [Fact]
    public void 修好了_內建修復中的下一個是第一個自訂子狀態()
    {
        foreach (var subs in new[] { WithBuiltIn, CustomOnly })
        {
            Assert.Equal("待更新測試機", TaskKinds.NextInStage(subs, "修復中", "修復中")?.Name);
            Assert.Null(TaskKinds.NextInStage(subs, "修復中", "待更新測試機")); // 最後一個：送復驗
        }
    }

    [Fact]
    public void 通過_內建待復驗的下一個是第一個自訂子狀態()
    {
        foreach (var subs in new[] { WithBuiltIn, CustomOnly })
        {
            Assert.Equal("待上正式機", TaskKinds.NextInStage(subs, "待復驗", "待復驗")?.Name);
            Assert.Null(TaskKinds.NextInStage(subs, "待復驗", "待上正式機"));
        }
    }

    [Fact]
    public void 沒有子狀態或不在這一段_沒有下一步()
    {
        Assert.Null(TaskKinds.NextInStage([], "修復中", "修復中"));
        Assert.Null(TaskKinds.NextInStage(WithBuiltIn, "修復中", "待上正式機"));
        Assert.Null(TaskKinds.NextInStage(WithBuiltIn, "修復中", "待修復"));
    }
}
