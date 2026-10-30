using System.Text.Json;
using HayakuBar.Claude;
using HayakuBar.Config;
using HayakuBar.Core;

namespace HayakuBar.Tests;

[Collection("env")] // 會改 CLAUDE_CONFIG_DIR，不要跟其他測試平行
public class AutoCreateTests
{
    /// <summary>在暫存的 Claude 設定資料夾裡寫一個 session 檔，掃描後回傳那個監看單位。</summary>
    static RepoActivity ScanSession(string repoPath, string claudeDir, params string[] lines)
    {
        var sessionDir = Path.Combine(claudeDir, "projects", ClaudeMonitor.Encode(repoPath));
        Directory.CreateDirectory(sessionDir);
        File.WriteAllText(Path.Combine(sessionDir, "s1.jsonl"), string.Join("\n", lines) + "\n");
        var monitor = new ClaudeMonitor { IsKnownKey = k => k.StartsWith("EG-") };
        monitor.Sync([new RepoConfig { Path = ClaudeMonitor.Normalize(repoPath), AutoCreate = true }]);
        monitor.Scan();
        monitor.Resolve(DateTime.Now, 10, new Dictionary<string, string>());
        return Assert.Single(monitor.Repos);
    }

    static string User(string cwd, string text) =>
        $$"""{"type":"user","message":{"role":"user","content":{{JsonSerializer.Serialize(text)}}},"cwd":{{JsonSerializer.Serialize(cwd)}},"sessionId":"s1","timestamp":"2026-10-28T02:00:00Z"}""";

    static string ToolUse(string name, string file, string at = "2026-10-28T02:05:00Z") =>
        $$$"""{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"改一下"},{"type":"tool_use","id":"t1","name":"{{{name}}}","input":{"file_path":{{{JsonSerializer.Serialize(file)}}}}}]},"timestamp":"{{{at}}}"}""";

    static void WithClaudeDir(Action<string, string> test)
    {
        using var claudeDir = new TempDir();
        using var repoDir = new TempDir();
        var previous = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
        Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", claudeDir.Path);
        try
        {
            test(claudeDir.Path, repoDir.Path);
        }
        finally
        {
            Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", previous);
        }
    }

    [Fact]
    public void 在專案裡改檔_記下改檔時間與第一句指令() => WithClaudeDir((claude, repo) =>
    {
        var a = ScanSession(repo, claude,
            User(repo, "繼續"),
            User(repo, "列印頁面加上全部列印按鈕\n細節如下…"),
            ToolUse("Edit", Path.Combine(repo, "src", "Print.cs")));
        Assert.True(a.AutoCreate);
        Assert.Equal("列印頁面加上全部列印按鈕", a.FirstPrompt); // 「繼續」這類沒內容的跳過
        Assert.Equal(DateTimeOffset.Parse("2026-10-28T02:05:00Z").LocalDateTime, a.EditedAt);
        Assert.Null(a.ResolvedTask);
    });

    [Fact]
    public void 只是問問題_沒有改檔時間() => WithClaudeDir((claude, repo) =>
    {
        var a = ScanSession(repo, claude,
            User(repo, "這個函式在做什麼？"),
            """{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"x"}}]}}""",
            """{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"git status"}}]}}""");
        Assert.Null(a.EditedAt);
        Assert.Equal("這個函式在做什麼", a.FirstPrompt);
    });

    [Fact]
    public void 改的是專案外的檔案_例如計畫檔_不算() => WithClaudeDir((claude, repo) =>
    {
        var plan = Path.Combine(claude, "plans", "plan.md");
        var a = ScanSession(repo, claude, User(repo, "先規劃一下"), ToolUse("Write", plan));
        Assert.Null(a.EditedAt);
    });

    [Fact]
    public void 有任務編號的session_ResolvedTask有值() => WithClaudeDir((claude, repo) =>
    {
        var a = ScanSession(repo, claude, User(repo, "修 EG-7 的查詢欄位"), ToolUse("MultiEdit", Path.Combine(repo, "a.cs")));
        Assert.NotNull(a.EditedAt);
        Assert.Equal("EG-7", a.ResolvedTask); // 已有任務，引擎不會詢問
    });

    [Fact]
    public void EditTime_GitBash路徑也認得()
    {
        using var doc = JsonDocument.Parse(ToolUse("Write", "/d/repo/Print-App/src/a.ts"));
        Assert.NotNull(ClaudeMonitor.EditTime(doc.RootElement, @"D:\repo\Print-App"));
        Assert.Null(ClaudeMonitor.EditTime(doc.RootElement, @"D:\repo\Other"));
    }

    [Fact]
    public void 子專案的自動建立任務設定()
    {
        using var root = new TempDir();
        Directory.CreateDirectory(Path.Combine(root.Path, "Print-App", ".git"));
        Directory.CreateDirectory(Path.Combine(root.Path, "Api", ".git"));
        var monitor = new ClaudeMonitor();
        monitor.Sync([new RepoConfig
        {
            Path = root.Path,
            Projects = [new ProjectConfig { Name = "Print-App", AutoCreate = true }],
        }]);
        Assert.True(monitor.Repos.Single(r => r.Name == "Print-App").AutoCreate);
        Assert.False(monitor.Repos.Single(r => r.Name == "Api").AutoCreate); // 預設關閉
        Assert.False(monitor.Repos.Single(r => r.RootPath is null).AutoCreate);
    }

    [Fact]
    public void ProjectConfig_預設值不用存()
    {
        Assert.True(new ProjectConfig { Name = "a" }.IsDefault);
        Assert.False(new ProjectConfig { Name = "a", AutoCreate = true }.IsDefault);
        Assert.DoesNotContain("IsDefault", JsonSerializer.Serialize(new ProjectConfig()));
    }

    static readonly List<(string Key, string Name)> Projects =
    [
        ("F2P", "F2P 列印平台"),
        ("PRT", "Print App"),
        ("EG", "長榮 EHMS"),
    ];

    [Theory]
    [InlineData("EG", "Print-App", "EG")]          // 上次用的專案最優先
    [InlineData(null, "Print-App", "PRT")]         // 名稱相同（忽略大小寫與符號）
    [InlineData(null, "f2p", "F2P")]               // 代碼相同
    [InlineData(null, "EHMS", "EG")]               // 互相包含
    [InlineData("XX", "unknown-folder", "F2P")]    // 上次的專案已不存在 → 正在計時的
    public void ProjectGuess_依序猜專案(string? hinted, string folder, string expected) =>
        Assert.Equal(expected, ProjectGuess.Guess(Projects, hinted, folder, ["F2P"]));

    [Fact]
    public void ProjectGuess_都猜不到回null() =>
        Assert.Null(ProjectGuess.Guess(Projects, null, "random", []));

    [Fact]
    public void AppState_資料夾對應不分大小寫()
    {
        var state = JsonSerializer.Deserialize<AppState>(
            """{"FolderHints":{"D:\\repo\\Print-App":{"Project":"PRT","Parent":"PRT-3"}}}""")!;
        Assert.Equal("PRT-3", state.FolderHints[@"d:\REPO\print-app"].Parent);
    }
}

public class CreatedKeyTests
{
    static string? Key(string json, string? parent = null) =>
        HayakuBar.Mcp.HayakuApi.CreatedKey(JsonDocument.Parse(json).RootElement, parent);

    [Fact]
    public void 直接建立_task欄位() => Assert.Equal("F2P-103", Key("""{"task":"F2P-103","title":"x"}"""));

    [Fact]
    public void 要核准_包在物件裡_跳過路徑與上層() =>
        Assert.Equal("F2P-103", Key("""{"message":"已送出","parent":"F2P-5","task":{"path":[{"task":"F2P-5"}],"task":"F2P-103"}}""", "F2P-5"));

    [Fact]
    public void 要核准_包在changeRequest裡() =>
        Assert.Equal("F2P-103", Key("""{"changeRequest":{"id":"abc","type":"CREATE","task":{"key":"F2P-103"}}}"""));

    [Fact]
    public void 只有訊息文字_排除上層編號() =>
        Assert.Equal("F2P-103", Key("""{"message":"已在 F2P-5 底下建立 F2P-103，等管理者核准"}""", "F2P-5"));

    [Fact]
    public void 找不到回null() => Assert.Null(Key("""{"ok":true}"""));
}

public class EstimateTests
{
    [Theory]
    [InlineData("4", 4.0)]
    [InlineData("1.5\n", 1.5)]
    [InlineData("約 2.3 小時", 2.5)]
    [InlineData("0.2", 0.5)]
    [InlineData("3.75", 4.0)]
    public void 回覆換成以半小時為單位(string reply, double expected) => Assert.Equal(expected, ClaudeEstimator.ParseHours(reply));

    [Theory]
    [InlineData("")]
    [InlineData("不知道")]
    [InlineData("0")]
    [InlineData("5000")]
    public void 看不懂或不合理回null(string reply) => Assert.Null(ClaudeEstimator.ParseHours(reply));

    [Fact]
    public void 提示包含名稱專案與指令()
    {
        var prompt = ClaudeEstimator.BuildPrompt("列印頁面加上全部列印按鈕", "Print-App", "幫我在列印頁加按鈕");
        Assert.Contains("任務名稱：列印頁面加上全部列印按鈕", prompt);
        Assert.Contains("專案：Print-App", prompt);
        Assert.Contains("幫我在列印頁加按鈕", prompt);
    }

    [Fact]
    public void 讀session開頭的指令原文_跳過繼續與圖片標記()
    {
        static string User(string text) =>
            $$"""{"type":"user","message":{"role":"user","content":{{JsonSerializer.Serialize(text)}}},"cwd":"D:\repo","sessionId":"s1"}""";
        var file = Path.GetTempFileName();
        try
        {
            File.WriteAllText(file, string.Join("\n",
                User("[Image #1] 列印頁面加上全部列印按鈕\n按鈕放在右上角"),
                User("繼續"),
                User("順便加選擇列印"),
                User("第三句"),
                User("第四句不讀")) + "\n");
            var text = ClaudeMonitor.ReadOpeningPrompts(file);
            Assert.Equal("列印頁面加上全部列印按鈕\n按鈕放在右上角\n---\n順便加選擇列印\n---\n第三句", text);
        }
        finally
        {
            File.Delete(file);
        }
    }

    [Fact]
    public void 讀不到檔案回null() => Assert.Null(ClaudeMonitor.ReadOpeningPrompts(Path.Combine(Path.GetTempPath(), Guid.NewGuid() + ".jsonl")));
}
