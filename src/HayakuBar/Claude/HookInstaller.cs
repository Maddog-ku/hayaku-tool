using System.Text.Json;
using System.Text.Json.Nodes;

namespace HayakuBar.Claude;

/// <summary>
/// 在 repo 的 .claude/settings.local.json 加上 Claude Code hooks，事件發生時把 hook JSON 轉送給本機 HayakuBar。
/// 工具沒開時 curl 連不上，會直接 exit 0，不影響 Claude Code。
/// </summary>
public static class HookInstaller
{
    static readonly string[] Events = ["UserPromptSubmit", "Stop", "SessionEnd"];

    static readonly JsonSerializerOptions WriteOptions = new()
    {
        WriteIndented = true,
        Encoder = System.Text.Encodings.Web.JavaScriptEncoder.UnsafeRelaxedJsonEscaping,
    };

    public static string SettingsPath(string repo) => Path.Combine(repo, ".claude", "settings.local.json");

    static string Marker(int port) => $"127.0.0.1:{port}/hook";

    public static string Command(int port) =>
        $"curl -s -m 2 -X POST -H \"Content-Type: application/json\" --data-binary @- http://{Marker(port)} || exit 0";

    public static bool IsInstalled(string repo, int port)
    {
        var root = Load(SettingsPath(repo));
        return root?["hooks"] is JsonObject hooks
               && Events.All(ev => hooks[ev] is JsonArray arr && ContainsOurs(arr, port));
    }

    public static void Install(string repo, int port)
    {
        var path = SettingsPath(repo);
        var root = Load(path) ?? new JsonObject();
        if (root["hooks"] is not JsonObject hooks)
        {
            hooks = new JsonObject();
            root["hooks"] = hooks;
        }
        foreach (var ev in Events)
        {
            if (hooks[ev] is not JsonArray arr)
            {
                arr = new JsonArray();
                hooks[ev] = arr;
            }
            RemoveOurs(arr, port);
            arr.Add(new JsonObject
            {
                ["hooks"] = new JsonArray(new JsonObject
                {
                    ["type"] = "command",
                    ["command"] = Command(port),
                    ["timeout"] = 5,
                }),
            });
        }
        Save(path, root);
    }

    public static void Uninstall(string repo, int port)
    {
        var path = SettingsPath(repo);
        var root = Load(path);
        if (root?["hooks"] is not JsonObject hooks)
            return;
        foreach (var ev in Events)
        {
            if (hooks[ev] is not JsonArray arr)
                continue;
            RemoveOurs(arr, port);
            if (arr.Count == 0)
                hooks.Remove(ev);
        }
        if (hooks.Count == 0)
            root.Remove("hooks");
        Save(path, root);
    }

    static bool ContainsOurs(JsonArray groups, int port) =>
        groups.OfType<JsonObject>().Any(g => g["hooks"] is JsonArray hs && hs.OfType<JsonObject>().Any(h => IsOurs(h, port)));

    static void RemoveOurs(JsonArray groups, int port)
    {
        foreach (var group in groups.OfType<JsonObject>().ToList())
        {
            if (group["hooks"] is not JsonArray hs)
                continue;
            foreach (var h in hs.OfType<JsonObject>().Where(h => IsOurs(h, port)).ToList())
                hs.Remove(h);
            if (hs.Count == 0)
                groups.Remove(group);
        }
    }

    static bool IsOurs(JsonObject hook, int port) =>
        hook["command"]?.GetValue<string>()?.Contains(Marker(port), StringComparison.Ordinal) == true;

    static JsonObject? Load(string path)
    {
        if (!File.Exists(path))
            return null;
        var text = File.ReadAllText(path);
        if (string.IsNullOrWhiteSpace(text))
            return null;
        return JsonNode.Parse(text, documentOptions: new JsonDocumentOptions { CommentHandling = JsonCommentHandling.Skip, AllowTrailingCommas = true }) as JsonObject
               ?? throw new InvalidOperationException($"{path} 不是 JSON 物件");
    }

    static void Save(string path, JsonObject root)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(path)!);
        var backup = path + ".hayakubar.bak";
        if (File.Exists(path) && !File.Exists(backup))
            File.Copy(path, backup);
        File.WriteAllText(path, root.ToJsonString(WriteOptions) + Environment.NewLine);
    }
}
