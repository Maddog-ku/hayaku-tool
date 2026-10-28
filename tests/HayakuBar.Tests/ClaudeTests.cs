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
            var lines = new[]
            {
                """{"type":"mode","mode":"normal","sessionId":"s1"}""",
                """{"type":"user","message":{"role":"user","content":"幫我修 EG-7 的查詢欄位"},"gitBranch":"feature/f2p-97-reset","cwd":"x","sessionId":"s1"}""",
                """{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"好的 F2P-14"}]},"gitBranch":"feature/f2p-97-reset"}""",
                """{"type":"user","isMeta":true,"message":{"role":"user","content":"EG-11 meta 不算"}}""",
            };
            File.WriteAllText(Path.Combine(sessionDir, "s1.jsonl"), string.Join("\n", lines) + "\n");

            var monitor = new ClaudeMonitor { IsKnownKey = k => k.StartsWith("EG-") || k.StartsWith("F2P-") };
            var repo = new RepoConfig { Path = repoDir.Path, DefaultTask = "F2P-5" };
            monitor.Sync([repo]);
            monitor.Scan();
            monitor.Resolve(DateTime.Now, 10, new Dictionary<string, string>());

            var a = Assert.Single(monitor.Repos);
            Assert.Equal("s1", a.SessionId);
            Assert.Equal("feature/f2p-97-reset", a.Branch);
            Assert.True(a.Active);
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

    [Fact]
    public void OnHook_SessionEnd後就不算活動中()
    {
        using var repoDir = new TempDir();
        var monitor = new ClaudeMonitor();
        monitor.Sync([new RepoConfig { Path = repoDir.Path, DefaultTask = "F2P-5" }]);
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
