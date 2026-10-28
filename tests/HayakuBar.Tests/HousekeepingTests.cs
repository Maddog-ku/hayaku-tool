using HayakuBar.Infra;

namespace HayakuBar.Tests;

public class HousekeepingTests
{
    [Fact]
    public void CleanFiles_只刪舊截圖與過期的舊紀錄()
    {
        using var dir = new TempDir();
        var now = new DateTime(2026, 10, 28, 15, 0, 0);
        var shots = Directory.CreateDirectory(Path.Combine(dir.Path, "shots")).FullName;
        string Touch(string path, DateTime at)
        {
            File.WriteAllText(path, "x");
            File.SetLastWriteTime(path, at);
            return path;
        }
        var oldShot = Touch(Path.Combine(shots, "old.png"), now.AddDays(-4));
        var newShot = Touch(Path.Combine(shots, "new.png"), now.AddDays(-1));
        var oldLog = Touch(Path.Combine(dir.Path, "log.txt.1"), now.AddDays(-15));
        var log = Touch(Path.Combine(dir.Path, "log.txt"), now.AddDays(-30));
        var config = Touch(Path.Combine(dir.Path, "config.json"), now.AddDays(-30));

        Assert.Equal(2, Housekeeping.CleanFiles(dir.Path, now));
        Assert.False(File.Exists(oldShot));
        Assert.False(File.Exists(oldLog));
        Assert.True(File.Exists(newShot));
        Assert.True(File.Exists(log)); // 目前的紀錄、設定不會動
        Assert.True(File.Exists(config));
    }

    [Fact]
    public void CleanFiles_沒有截圖資料夾也不會出錯()
    {
        using var dir = new TempDir();
        Assert.Equal(0, Housekeeping.CleanFiles(dir.Path, DateTime.Now));
    }

    [Fact]
    public void TrimMemory_可以重複呼叫()
    {
        Housekeeping.TrimMemory();
        Housekeeping.TrimMemory();
        Assert.Contains("工作集", Housekeeping.MemoryText());
    }
}
