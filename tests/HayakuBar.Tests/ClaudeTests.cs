using System.Text.Json;
using System.Text.Json.Nodes;
using HayakuBar.Claude;
using HayakuBar.Config;

namespace HayakuBar.Tests;

public sealed class TempDir : IDisposable
{
    public string Path { get; } = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "hayakubar-test-" + Guid.NewGuid().ToString("N"));
    public TempDir() => Directory.CreateDirectory(Path);
    public void Dispose()
    {
        try { Directory.Delete(Path, recursive: true); } catch { }
    }
}

[Collection("env")] // 會改 CLAUDE_CONFIG_DIR，不要跟其他測試平行
public class ClaudeMonitorTests
{
    [Fact]
    public void Encode_跟ClaudeCode的資料夾命名一致()
    {
        Assert.Equal("D--sj-hayaku-tool", ClaudeMonitor.Encode(@"D:\sj\hayaku tool"));
        Assert.Equal("C--Users-harry-gu", ClaudeMonitor.Encode(@"C:\Users\harry.gu\"));
    }

    [Fact]
    public void Scan_從session紀錄抓分支與對話中的任務()
    {
        using var claudeDir = new TempDir();
        using var repoDir = new TempDir();
        var previous = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
        Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", claudeDir.Path);
        try
        {
            var sessionDir = Path.Combine(claudeDir.Path, "projects", ClaudeMonitor.Encode(repoDir.Path));
            Directory.CreateDirectory(sessionDir);
            var cwd = JsonSerializer.Serialize(repoDir.Path);
            var lines = new[]
            {
                """{"type":"mode","mode":"normal","sessionId":"s1"}""",
                $$"""{"type":"user","message":{"role":"user","content":"幫我修 EG-7 的查詢欄位"},"gitBranch":"feature/f2p-97-reset","cwd":{{cwd}},"sessionId":"s1","entrypoint":"claude-desktop"}""",
                """{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"好的 F2P-14"}]},"gitBranch":"feature/f2p-97-reset"}""",
                """{"type":"user","isMeta":true,"message":{"role":"user","content":"EG-11 meta 不算"}}""",
            };
            File.WriteAllText(Path.Combine(sessionDir, "s1.jsonl"), string.Join("\n", lines) + "\n");

            var monitor = new ClaudeMonitor { IsKnownKey = k => k.StartsWith("EG-") || k.StartsWith("F2P-") };
            var repo = new RepoConfig { Path = ClaudeMonitor.Normalize(repoDir.Path) };
            monitor.Sync([repo]);
            monitor.Scan();
            monitor.Resolve(DateTime.Now, 10, new Dictionary<string, string>());

            var a = Assert.Single(monitor.Repos);
            Assert.Equal("s1", a.SessionId);
            Assert.Equal("feature/f2p-97-reset", a.Branch);
            Assert.True(a.Active);
            Assert.Equal("桌面版", a.SourceText);
            Assert.Equal("EG-7", a.ResolvedTask); // 對話內容優先於分支

            monitor.Resolve(DateTime.Now, 10, new Dictionary<string, string> { ["s1"] = "F2P-21" });
            Assert.Equal("F2P-21", a.ResolvedTask); // 手動指定最優先

            monitor.Resolve(DateTime.Now.AddMinutes(30), 10, new Dictionary<string, string>());
            Assert.False(a.Active); // 超過閒置時間
        }
        finally
        {
            Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", previous);
        }
    }

    [Theory]
    [InlineData("/d/repo/Print-App", @"D:\repo\Print-App")]
    [InlineData("/mnt/c/Users/x/", @"C:\Users\x")]
    [InlineData(@"D:\發表報告\", @"D:\發表報告")]
    public void Normalize_GitBash路徑也轉成Windows路徑(string input, string expected) =>
        Assert.Equal(expected, ClaudeMonitor.Normalize(input));

    static void WriteSession(string projectsDir, string dirName, string file, string cwd, string entrypoint = "cli")
    {
        var dir = Path.Combine(projectsDir, dirName);
        Directory.CreateDirectory(dir);
        var line = $$"""{"type":"user","message":{"role":"user","content":"hi"},"cwd":{{JsonSerializer.Serialize(cwd)}},"sessionId":"x","entrypoint":"{{entrypoint}}"}""";
        File.WriteAllText(Path.Combine(dir, file), line + "\n");
    }

    [Fact]
    public void Scan_子資料夾開的session算進來_撞名與相鄰repo不算()
    {
        using var claudeDir = new TempDir();
        using var work = new TempDir();
        var previous = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
        Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", claudeDir.Path);
        try
        {
            var projects = Path.Combine(claudeDir.Path, "projects");
            var repo = Directory.CreateDirectory(Path.Combine(work.Path, "卡機")).FullName;
            var other = Directory.CreateDirectory(Path.Combine(work.Path, "報告")).FullName; // 編碼後跟「卡機」同名
            var sibling = Directory.CreateDirectory(Path.Combine(work.Path, "卡機-v2")).FullName;
            var encoded = ClaudeMonitor.Encode(repo);
            Assert.Equal(encoded, ClaudeMonitor.Encode(other));

            WriteSession(projects, encoded, "other.jsonl", other);
            Thread.Sleep(30);
            WriteSession(projects, ClaudeMonitor.Encode(sibling), "sibling.jsonl", sibling);
            Thread.Sleep(30);
            var sub = Directory.CreateDirectory(Path.Combine(repo, "src")).FullName;
            WriteSession(projects, ClaudeMonitor.Encode(sub), "mine.jsonl", "/" + sub[0].ToString().ToLowerInvariant() + sub[2..].Replace('\\', '/'));
            // 最新的是撞名的那個，但它不屬於 repo
            Thread.Sleep(30);
            WriteSession(projects, encoded, "other2.jsonl", other);

            var monitor = new ClaudeMonitor();
            monitor.Sync([new RepoConfig { Path = repo, IncludeSubprojects = false }]);
            monitor.Scan();
            var a = Assert.Single(monitor.Repos);
            Assert.Equal("mine", a.SessionId);
            Assert.Equal("CLI", a.SourceText);
        }
        finally
        {
            Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", previous);
        }
    }

    [Fact]
    public void 主資料夾_自動偵測專案_各自計時_偵測不到任務就不計時()
    {
        using var claudeDir = new TempDir();
        using var work = new TempDir();
        var previous = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
        Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", claudeDir.Path);
        try
        {
            var projects = Path.Combine(claudeDir.Path, "projects");
            var root = Directory.CreateDirectory(Path.Combine(work.Path, "repo")).FullName;
            var app = Directory.CreateDirectory(Path.Combine(root, "Print-App")).FullName;
            var server = Directory.CreateDirectory(Path.Combine(root, "Print-Server")).FullName;
            Directory.CreateDirectory(Path.Combine(server, ".git"));          // git repo，沒有 session
            var plain = Directory.CreateDirectory(Path.Combine(root, "docs")).FullName; // 一般資料夾，沒有 session → 不算專案
            Directory.CreateDirectory(Path.Combine(root, "node_modules", ".git"));

            WriteSession(projects, ClaudeMonitor.Encode(app), "a.jsonl", app, "cli");
            Thread.Sleep(30);
            var appSub = Directory.CreateDirectory(Path.Combine(app, "src")).FullName;
            WriteSession(projects, ClaudeMonitor.Encode(appSub), "b.jsonl", appSub, "claude-desktop");
            WriteSession(projects, ClaudeMonitor.Encode(root), "c.jsonl", root, "cli");

            var monitor = new ClaudeMonitor();
            var detected = monitor.DetectProjects(root);
            Assert.Equal(["Print-App", "Print-Server"], detected.Select(p => p.Name).OrderBy(n => n));
            Assert.DoesNotContain(detected, p => p.Path == plain);

            monitor.Sync([new RepoConfig
            {
                Path = root,
                Projects = [new ProjectConfig { Name = "Print-Server", Enabled = false }],
            }]);
            monitor.Scan();
            monitor.Resolve(DateTime.Now, 10, new Dictionary<string, string>());

            var units = monitor.Repos.ToDictionary(u => u.Name);
            Assert.Equal(3, units.Count); // repo、Print-App、Print-Server
            // Print-App 子資料夾開的 session 算 Print-App，不重複算主資料夾
            Assert.Equal("b", units["Print-App"].SessionId);
            Assert.True(units["Print-App"].Active);
            Assert.Null(units["Print-App"].ResolvedTask); // 沒有編號、沒有分支：不計時，沒有預設任務
            Assert.Equal("c", units["repo"].SessionId);
            Assert.Null(units["repo"].ResolvedTask);
            Assert.False(units["Print-Server"].Active);
            Assert.False(units["Print-Server"].Enabled);
            Assert.Equal(root, units["Print-Server"].RootPath);
        }
        finally
        {
            Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", previous);
        }
    }

    [Fact]
    public void 主資料夾本身是git_repo時_一般子資料夾不算專案()
    {
        using var claudeDir = new TempDir();
        using var work = new TempDir();
        var previous = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
        Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", claudeDir.Path);
        try
        {
            var root = Directory.CreateDirectory(Path.Combine(work.Path, "single")).FullName;
            Directory.CreateDirectory(Path.Combine(root, ".git"));
            var src = Directory.CreateDirectory(Path.Combine(root, "src")).FullName;
            WriteSession(Path.Combine(claudeDir.Path, "projects"), ClaudeMonitor.Encode(src), "s.jsonl", src);

            var monitor = new ClaudeMonitor();
            Assert.Empty(monitor.DetectProjects(root));
            monitor.Sync([new RepoConfig { Path = root }]);
            monitor.Scan();
            Assert.Equal("s", Assert.Single(monitor.Repos).SessionId);
        }
        finally
        {
            Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", previous);
        }
    }

    [Fact]
    public void Discover_列出最近用過的資料夾()
    {
        using var claudeDir = new TempDir();
        using var work = new TempDir();
        var previous = Environment.GetEnvironmentVariable("CLAUDE_CONFIG_DIR");
        Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", claudeDir.Path);
        try
        {
            var projects = Path.Combine(claudeDir.Path, "projects");
            var a = Directory.CreateDirectory(Path.Combine(work.Path, "a")).FullName;
            var b = Directory.CreateDirectory(Path.Combine(work.Path, "b")).FullName;
            WriteSession(projects, ClaudeMonitor.Encode(a), "1.jsonl", a, "claude-desktop");
            Thread.Sleep(30);
            WriteSession(projects, ClaudeMonitor.Encode(b), "2.jsonl", b);
            WriteSession(projects, "gone", "3.jsonl", Path.Combine(work.Path, "deleted")); // 資料夾已不存在

            var found = ClaudeMonitor.Discover();
            Assert.Equal([b, a], found.Select(p => p.Path));
            Assert.Equal("桌面版", found[1].Source);
        }
        finally
        {
            Environment.SetEnvironmentVariable("CLAUDE_CONFIG_DIR", previous);
        }
    }

    [Fact]
    public void OnHook_GitBash格式的cwd也對得上()
    {
        using var repoDir = new TempDir();
        var monitor = new ClaudeMonitor();
        monitor.Sync([new RepoConfig { Path = repoDir.Path }]);
        var posix = "/" + repoDir.Path[0].ToString().ToLowerInvariant() + repoDir.Path[2..].Replace('\\', '/') + "/src";
        var payload = JsonSerializer.SerializeToElement(new { cwd = posix, hook_event_name = "UserPromptSubmit", session_id = "s" });
        Assert.True(monitor.OnHook(payload, DateTime.Now));
    }

    [Fact]
    public void OnHook_SessionEnd後就不算活動中()
    {
        using var repoDir = new TempDir();
        var monitor = new ClaudeMonitor();
        monitor.Sync([new RepoConfig { Path = repoDir.Path }]);
        var now = DateTime.Now;
        JsonElement Hook(string ev, string? prompt = null) => JsonSerializer.SerializeToElement(new
        {
            session_id = "s2",
            cwd = Path.Combine(repoDir.Path, "src"),
            hook_event_name = ev,
            prompt,
        });

        Assert.True(monitor.OnHook(Hook("UserPromptSubmit", "做 F2P-98"), now));
        monitor.Resolve(now, 10, new Dictionary<string, string>());
        var a = Assert.Single(monitor.Repos);
        Assert.True(a.Active);
        Assert.Equal("F2P-98", a.ResolvedTask);

        monitor.OnHook(Hook("SessionEnd"), now.AddSeconds(5));
        monitor.Resolve(now.AddSeconds(6), 10, new Dictionary<string, string>());
        Assert.False(a.Active);
    }

    [Fact]
    public void OnHook_不是設定的repo就忽略()
    {
        var monitor = new ClaudeMonitor();
        monitor.Sync([new RepoConfig { Path = @"C:\nowhere\repo" }]);
        var payload = JsonSerializer.SerializeToElement(new { cwd = @"C:\other", hook_event_name = "Stop" });
        Assert.False(monitor.OnHook(payload, DateTime.Now));
    }
}

public class HookInstallerTests
{
    [Fact]
    public void Install_保留既有設定_Uninstall_還原()
    {
        using var repo = new TempDir();
        var path = HookInstaller.SettingsPath(repo.Path);
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        File.WriteAllText(path, """
            {
              "permissions": { "allow": ["Bash(npm test)"] },
              "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": "echo mine" } ] } ] }
            }
            """);

        HookInstaller.Install(repo.Path, 47823);
        HookInstaller.Install(repo.Path, 47823); // 重複安裝不會重複加
        Assert.True(HookInstaller.IsInstalled(repo.Path, 47823));
        var root = JsonNode.Parse(File.ReadAllText(path))!;
        Assert.Equal("Bash(npm test)", root["permissions"]!["allow"]![0]!.GetValue<string>());
        Assert.Equal(2, root["hooks"]!["Stop"]!.AsArray().Count);
        Assert.Single(root["hooks"]!["UserPromptSubmit"]!.AsArray());
        Assert.True(File.Exists(path + ".hayakubar.bak"));

        HookInstaller.Uninstall(repo.Path, 47823);
        Assert.False(HookInstaller.IsInstalled(repo.Path, 47823));
        root = JsonNode.Parse(File.ReadAllText(path))!;
        var stop = root["hooks"]!["Stop"]!.AsArray();
        Assert.Single(stop);
        Assert.Equal("echo mine", stop[0]!["hooks"]![0]!["command"]!.GetValue<string>());
        Assert.Null(root["hooks"]!["UserPromptSubmit"]);
    }
}
