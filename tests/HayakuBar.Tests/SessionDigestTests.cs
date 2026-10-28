using System.Text.Json;
using HayakuBar.Claude;
using HayakuBar.Mcp;

namespace HayakuBar.Tests;

public class SessionDigestTests
{
    static readonly DateOnly Day = new(2026, 10, 28);

    static string Line(string type, string text, DateTime at, string cwd, string? branch = null, bool meta = false, bool sidechain = false) =>
        JsonSerializer.Serialize(new Dictionary<string, object?>
        {
            ["type"] = type,
            ["cwd"] = cwd,
            ["gitBranch"] = branch,
            ["isMeta"] = meta,
            ["isSidechain"] = sidechain,
            ["timestamp"] = new DateTimeOffset(at).ToUniversalTime().ToString("O"),
            ["message"] = new { role = type, content = text },
        });

    static RepoActivity Unit(string path) => new(path);

    [Theory]
    [InlineData("修正列印按鈕位置。\n細節在下面", "修正列印按鈕位置")]
    [InlineData("  continue ", null)]
    [InlineData("繼續", null)]
    [InlineData("/clear", null)]
    [InlineData("<command-name>/model</command-name>", null)]
    [InlineData("[Request interrupted by user]", null)]
    [InlineData("commit 並Push", null)]
    [InlineData("直接開始實作值到完成", null)]
    [InlineData("[Image #8]", null)]
    [InlineData("[Image #9] 不要有全部歸零按鈕", "不要有全部歸零按鈕")]
    [InlineData("版本號用2.1.28，傳真接收放在部門那邊，然後 commit 並 push", "版本號用2.1.28，傳真接收放在部門那邊，然後 commit 並 push")]
    public void Summarize_取第一行並略過沒內容的回覆(string prompt, string? expected) =>
        Assert.Equal(expected, SessionDigest.Summarize(prompt));

    [Fact]
    public void Summarize_太長會截斷() =>
        Assert.Equal(61, SessionDigest.Summarize(new string('字', 100))!.Length);

    [Fact]
    public void Parse_依對話編號與分支歸屬且只取當天()
    {
        var at = Day.ToDateTime(new TimeOnly(10, 0));
        var lines = new[]
        {
            Line("user", "昨天的事", at.AddDays(-1), @"D:\repo\app"),
            Line("user", "先看一下登入流程", at, @"D:\repo\app", branch: "feature/F2P-12-login"),
            Line("assistant", "好的", at.AddMinutes(1), @"D:\repo\app"),
            Line("user", "F2P-97 修正列印按鈕", at.AddMinutes(5), @"D:\repo\app"),
            Line("user", "再調整間距", at.AddMinutes(9), @"D:\repo\app"),
            Line("user", "系統訊息", at.AddMinutes(10), @"D:\repo\app", meta: true),
            Line("user", "子代理的指令", at.AddMinutes(11), @"D:\repo\app", sidechain: true),
            Line("user", "別的資料夾", at.AddMinutes(12), @"E:\other"),
        };
        var prompts = SessionDigest.Parse(lines, Day, [Unit(@"D:\repo"), Unit(@"D:\repo\app")], null, _ => true);
        Assert.Equal(["先看一下登入流程", "F2P-97 修正列印按鈕", "再調整間距"], prompts.Select(p => p.Summary));
        Assert.Equal(["F2P-12", "F2P-97", "F2P-97"], prompts.Select(p => p.Task));
        Assert.All(prompts, p => Assert.Equal(@"D:\repo\app", p.Folder));
    }

    [Fact]
    public void Parse_手動指定的任務優先()
    {
        var lines = new[] { Line("user", "F2P-97 做這個", Day.ToDateTime(new TimeOnly(9, 0)), @"D:\repo") };
        var prompts = SessionDigest.Parse(lines, Day, [Unit(@"D:\repo")], "EG-7", _ => true);
        Assert.Equal("EG-7", Assert.Single(prompts).Task);
    }

    [Fact]
    public void NotesFor_只填沒有備註且時間對得上的計時段()
    {
        var day = Day.ToDateTime(TimeOnly.MinValue);
        TimeEntry Entry(string id, string task, double from, double to, string? note = null) =>
            new(id, task, task, new DateTimeOffset(day.AddHours(from)), new DateTimeOffset(day.AddHours(to)), null, false, null, note);
        var entries = new[]
        {
            Entry("a", "F2P-97", 9, 11),
            Entry("b", "F2P-97", 13, 14, note: "已經寫了"),
            Entry("c", "F2P-12", 9, 11),
            Entry("d", "F2P-97", 15, 16),
        };
        var prompts = new[]
        {
            new ClaudePrompt(day.AddHours(9.5), @"D:\repo", "F2P-97", "修正列印"),
            new ClaudePrompt(day.AddHours(10), @"D:\repo", "F2P-97", "修正列印"),
            new ClaudePrompt(day.AddHours(13.5), @"D:\repo", "F2P-97", "不會寫進已有備註的"),
        };
        var commits = new[] { new GitCommit(day.AddHours(10.5), @"D:\repo", "abc123", "fix: 列印按鈕", "F2P-97") };
        var notes = SessionDigest.NotesFor(entries, prompts, commits, day.AddHours(20));
        Assert.Equal("修正列印；fix: 列印按鈕", Assert.Single(notes, n => n.Key == "a").Value);
        Assert.Single(notes);
    }

    [Fact]
    public void Markdown_依任務分組並附上時數與commit()
    {
        var day = Day.ToDateTime(TimeOnly.MinValue);
        var prompts = new[]
        {
            new ClaudePrompt(day.AddHours(9), @"D:\repo", "F2P-97", "修正列印"),
            new ClaudePrompt(day.AddHours(10), @"D:\repo", null, "看一下環境"),
        };
        var commits = new[] { new GitCommit(day.AddHours(11), @"D:\repo", "abc123", "fix: 列印按鈕", "F2P-97") };
        var log = new[] { new WorkLogEntry("F2P-97", 130, []) };
        var md = SessionDigest.Markdown(Day, prompts, commits, log, k => k == "F2P-97" ? "面板列印" : null);
        Assert.Contains("## F2P-97 面板列印（2 小時 10 分）\n- 修正列印\n- commit abc123：fix: 列印按鈕".Replace("\n", Environment.NewLine), md);
        Assert.Contains("## 其他（沒對應到任務）" + Environment.NewLine + "- 看一下環境", md);
        Assert.True(md.IndexOf("F2P-97", StringComparison.Ordinal) < md.IndexOf("其他", StringComparison.Ordinal));
    }
}
