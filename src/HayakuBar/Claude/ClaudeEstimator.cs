using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;
using HayakuBar.Infra;

namespace HayakuBar.Claude;

/// <summary>
/// 建立任務時請 Claude 估預計工時：在背景跑 `claude -p`（用本機 Claude Code 的登入）。
/// 不存 session、不載入使用者設定（不觸發 hook）、不開工具與 MCP，執行目錄在 HayakuBar 自己的資料夾，
/// 所以不會被監看成一段新的 Claude 工作。
/// </summary>
public static partial class ClaudeEstimator
{
    static readonly TimeSpan Timeout = TimeSpan.FromSeconds(90);

    const string Args = "-p --model haiku --no-session-persistence --tools \"\" --setting-sources \"\" --strict-mcp-config";

    [GeneratedRegex(@"\d+(?:\.\d+)?")]
    private static partial Regex Number();

    /// <summary>估預計工時（小時，0.5 為單位）；找不到 claude、逾時或回覆看不懂時回 null。</summary>
    public static async Task<double?> EstimateAsync(string title, string? project, string? prompts, CancellationToken ct)
    {
        if (FindClaude() is not { } exe)
        {
            Log.Info("估預計工時：找不到 claude 指令");
            return null;
        }
        var psi = exe.EndsWith(".cmd", StringComparison.OrdinalIgnoreCase) || exe.EndsWith(".bat", StringComparison.OrdinalIgnoreCase)
            ? new ProcessStartInfo("cmd.exe", $"/d /s /c \"\"{exe}\" {Args}\"")
            : new ProcessStartInfo(exe, Args);
        psi.RedirectStandardInput = true;
        psi.RedirectStandardOutput = true;
        psi.RedirectStandardError = true;
        psi.UseShellExecute = false;
        psi.CreateNoWindow = true;
        psi.WorkingDirectory = AppPaths.Dir;
        psi.StandardInputEncoding = new UTF8Encoding(false);
        psi.StandardOutputEncoding = Encoding.UTF8;
        psi.StandardErrorEncoding = Encoding.UTF8;
        Process? p = null;
        using var cts = CancellationTokenSource.CreateLinkedTokenSource(ct);
        cts.CancelAfter(Timeout);
        try
        {
            p = Process.Start(psi);
            if (p is null)
                return null;
            await p.StandardInput.WriteAsync(BuildPrompt(title, project, prompts).AsMemory(), cts.Token);
            p.StandardInput.Close();
            var output = p.StandardOutput.ReadToEndAsync(cts.Token);
            var error = p.StandardError.ReadToEndAsync(cts.Token);
            await p.WaitForExitAsync(cts.Token);
            var text = await output;
            if (p.ExitCode != 0)
            {
                Log.Error($"估預計工時：claude 結束碼 {p.ExitCode}：{(await error).Trim()}");
                return null;
            }
            var hours = ParseHours(text);
            Log.Info($"估預計工時：「{title}」→ {(hours is { } h ? $"{h} 小時" : $"看不懂的回覆「{text.Trim()}」")}");
            return hours;
        }
        catch (Exception ex) when (ex is System.ComponentModel.Win32Exception or OperationCanceledException or InvalidOperationException or IOException)
        {
            if (!ct.IsCancellationRequested)
                Log.Error("估預計工時失敗", ex);
            return null;
        }
        finally
        {
            try
            {
                if (p is { HasExited: false })
                    p.Kill(entireProcessTree: true);
            }
            catch (Exception ex) when (ex is InvalidOperationException or System.ComponentModel.Win32Exception)
            {
            }
            p?.Dispose();
        }
    }

    public static string BuildPrompt(string title, string? project, string? prompts)
    {
        var sb = new StringBuilder();
        sb.AppendLine("你是軟體專案的工時估算助手。依下面的工作內容，估計一位熟悉這個專案的工程師（搭配 Claude Code）完成它，");
        sb.AppendLine("包含實作、自我測試與修正，需要多少工時。");
        sb.AppendLine("只回一個數字，單位是小時，以 0.5 為單位，最少 0.5；不要任何其他文字。");
        sb.AppendLine();
        if (!string.IsNullOrWhiteSpace(project))
            sb.AppendLine($"專案：{project}");
        sb.AppendLine($"任務名稱：{title}");
        if (!string.IsNullOrWhiteSpace(prompts))
        {
            sb.AppendLine();
            sb.AppendLine("使用者給 Claude Code 的指令：");
            sb.AppendLine(prompts);
        }
        return sb.ToString();
    }

    /// <summary>回覆裡的第一個數字，四捨五入到 0.5 小時（最少 0.5）；沒有數字或大得離譜回 null。</summary>
    public static double? ParseHours(string? text)
    {
        if (string.IsNullOrWhiteSpace(text) || Number().Match(text) is not { Success: true } m
            || !double.TryParse(m.Value, NumberStyles.Float, CultureInfo.InvariantCulture, out var hours)
            || hours <= 0 || hours > 1000)
            return null;
        return Math.Max(0.5, Math.Round(hours * 2, MidpointRounding.AwayFromZero) / 2);
    }

    /// <summary>PATH 裡的 claude（原生安裝的 claude.exe 優先，其次 npm 的 claude.cmd），再看預設安裝位置。</summary>
    static string? FindClaude()
    {
        var dirs = (Environment.GetEnvironmentVariable("PATH") ?? "").Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries)
            .Append(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".local", "bin"))
            .Append(Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData), "npm"))
            .ToList();
        foreach (var name in new[] { "claude.exe", "claude.cmd" })
            foreach (var dir in dirs)
            {
                try
                {
                    var path = Path.Combine(dir.Trim('"'), name);
                    if (File.Exists(path))
                        return path;
                }
                catch (ArgumentException)
                {
                }
            }
        return null;
    }
}
