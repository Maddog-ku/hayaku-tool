using HayakuBar.Core;

namespace HayakuBar.Tests;

public class SyncPolicyTests
{
    static readonly DateOnly Today = new(2026, 10, 29);
    static readonly DateTime Now = new(2026, 10, 29, 10, 0, 0);

    static bool Need(bool background = true, string[]? running = null, string[]? known = null,
        DateOnly? fetchedDay = null, double minutesAgo = 1) =>
        SyncPolicy.NeedEntries(background, running ?? ["a"], known ?? ["a"], Today, fetchedDay ?? Today, Now, Now.AddMinutes(-minutesAgo));

    [Fact]
    public void 背景輪詢_計時沒變_不重抓列表() => Assert.False(Need());

    [Fact]
    public void 動作後或使用者要求_一定重抓() => Assert.True(Need(background: false));

    [Fact]
    public void 有計時開始或停止_重抓()
    {
        Assert.True(Need(running: ["a", "b"]));
        Assert.True(Need(running: []));
        Assert.True(Need(running: ["b"]));
    }

    [Fact]
    public void 換日_重抓() => Assert.True(Need(fetchedDay: Today.AddDays(-1)));

    [Fact]
    public void 超過5分鐘_重抓() => Assert.True(Need(minutesAgo: 5));

    [Theory]
    [InlineData(1, 1)]
    [InlineData(2, 2)]
    [InlineData(3, 5)]
    [InlineData(10, 5)]
    public void 失敗退避_1_2_5分鐘(int failures, int minutes) =>
        Assert.Equal(TimeSpan.FromMinutes(minutes), SyncPolicy.BackoffDelay(failures));
}
