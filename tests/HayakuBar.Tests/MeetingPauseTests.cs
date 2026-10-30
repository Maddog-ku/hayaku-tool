using HayakuBar.Core;

namespace HayakuBar.Tests;

public class MeetingPauseTests
{
    static HashSet<string> Keys(params string[] keys) => new(keys, StringComparer.OrdinalIgnoreCase);

    [Fact]
    public void 會議本身在計時_其他加入前在跑的都要暫停()
    {
        var (meeting, others) = TimerEngine.SplitForMeeting("REVIVAL-9", Keys("F2P-1", "EG-3"), Keys("F2P-1", "EG-3", "REVIVAL-9"));
        Assert.Equal(["REVIVAL-9"], meeting);
        Assert.Equal(["EG-3", "F2P-1"], others);
    }

    [Fact]
    public void 找不到會議的key_加入後才出現的當成會議()
    {
        var (meeting, others) = TimerEngine.SplitForMeeting("REVIVAL-9", Keys("F2P-1"), Keys("F2P-1", "REVIVAL-M9"));
        Assert.Equal(["REVIVAL-M9"], meeting);
        Assert.Equal(["F2P-1"], others);
    }

    [Fact]
    public void 加入前會議就在計時_不會被暫停()
    {
        var (meeting, others) = TimerEngine.SplitForMeeting("revival-9", Keys("REVIVAL-9", "F2P-1"), Keys("REVIVAL-9", "F2P-1"));
        Assert.Equal(["REVIVAL-9"], meeting);
        Assert.Equal(["F2P-1"], others);
    }

    [Fact]
    public void 加入前沒有計時_沒有要暫停的()
    {
        var (_, others) = TimerEngine.SplitForMeeting("REVIVAL-9", Keys(), Keys("REVIVAL-9"));
        Assert.Empty(others);
    }
}
