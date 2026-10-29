using HayakuBar.Infra;

namespace HayakuBar.Tests;

public class HousekeepingTests
{
    static string Touch(string path, DateTime at, string text = "x")
    {
        File.WriteAllText(path, text);
        File.SetLastWriteTime(path, at);
        return path;
    }

    [Fact]
    public void CleanFiles_只刪舊截圖_設定不動()
    {
        using var dir = new TempDir();
        var now = new DateTime(2026, 10, 28, 15, 0, 0);
        var shots = Directory.CreateDirectory(Path.Combine(dir.Path, "shots")).FullName;
        var oldShot = Touch(Path.Combine(shots, "old.png"), now.AddDays(-4));
        var newShot = Touch(Path.Combine(shots, "new.png"), now.AddDays(-1));
        var config = Touch(Path.Combine(dir.Path, "config.json"), now.AddDays(-30));

        Assert.Equal(1, Housekeeping.CleanFiles(dir.Path, now));
        Assert.False(File.Exists(oldShot));
        Assert.True(File.Exists(newShot));
        Assert.True(File.Exists(config)); // 設定不會動
    }

    [Fact]
    public void CleanFiles_紀錄保留2個月()
    {
        using var dir = new TempDir();
        var now = new DateTime(2026, 10, 28, 15, 0, 0);
        var logs = Directory.CreateDirectory(Path.Combine(dir.Path, "logs")).FullName;
        var expired = Touch(Path.Combine(logs, "20260827090000.log"), now.AddDays(-61));
        var kept = Touch(Path.Combine(logs, "20260901090000.log"), now.AddDays(-57));

        Assert.Equal(1, Housekeeping.CleanFiles(dir.Path, now));
        Assert.False(File.Exists(expired));
        Assert.True(File.Exists(kept));
    }

    [Fact]
    public void CleanFiles_舊版的log_txt依第一筆時間搬進logs()
    {
        using var dir = new TempDir();
        var now = new DateTime(2026, 10, 28, 15, 0, 0);
        var legacy = Touch(Path.Combine(dir.Path, "log.txt"), now.AddHours(-1), "2026-10-20 09:00:00 [INFO] HayakuBar 啟動\n");
        var legacyOld = Touch(Path.Combine(dir.Path, "log.txt.1"), now.AddDays(-9), "2026-10-13 08:30:05 [INFO] HayakuBar 啟動\n");

        Housekeeping.CleanFiles(dir.Path, now);
        Assert.False(File.Exists(legacy));
        Assert.False(File.Exists(legacyOld));
        Assert.True(File.Exists(Path.Combine(dir.Path, "logs", "20261020090000.log")));
        Assert.True(File.Exists(Path.Combine(dir.Path, "logs", "20261013083005.log")));
    }

    [Fact]
    public void PickFile_啟動時接著寫今天的檔_換日或滿了開新檔()
    {
        using var dir = new TempDir();
        var now = new DateTime(2026, 10, 29, 9, 33, 1);
        var today = Path.Combine(dir.Path, "20261029080000.log");
        File.WriteAllText(today, "x");
        File.WriteAllText(Path.Combine(dir.Path, "20261028170000.log"), "x");

        Assert.Equal(today, Log.PickFile(dir.Path, now, reuseToday: true));
        Assert.Equal(Path.Combine(dir.Path, "20261029093301.log"), Log.PickFile(dir.Path, now, reuseToday: false));
    }

    [Fact]
    public void FreePath_同一秒重複就加編號()
    {
        using var dir = new TempDir();
        var at = new DateTime(2026, 10, 29, 9, 33, 1);
        var first = Log.FreePath(dir.Path, at);
        Assert.Equal(Path.Combine(dir.Path, "20261029093301.log"), first);
        File.WriteAllText(first, "x");
        Assert.Equal(Path.Combine(dir.Path, "20261029093301-2.log"), Log.FreePath(dir.Path, at));
    }

    [Fact]
    public void CleanFiles_刪掉寫入中斷留下的舊tmp()
    {
        using var dir = new TempDir();
        var now = new DateTime(2026, 10, 28, 15, 0, 0);
        var oldTmp = Path.Combine(dir.Path, "config.json.tmp");
        var newTmp = Path.Combine(dir.Path, "state.json.tmp");
        File.WriteAllText(oldTmp, "{}");
        File.SetLastWriteTime(oldTmp, now.AddDays(-2));
        File.WriteAllText(newTmp, "{}");
        File.SetLastWriteTime(newTmp, now.AddHours(-1));

        Assert.Equal(1, Housekeeping.CleanFiles(dir.Path, now));
        Assert.False(File.Exists(oldTmp));
        Assert.True(File.Exists(newTmp));
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
