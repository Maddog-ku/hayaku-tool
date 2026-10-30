using HayakuBar.Config;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.Core;

/// <summary>
/// Claude 在某個資料夾改程式、但對應不到任務：建議建立任務。
/// Title 是 session 第一句指令的摘要；Project、Parent 是這個資料夾上次用的專案與上層（沒有就 null）；
/// SessionFile 是 session 紀錄檔，請 Claude 估預計工時時讀開頭幾句指令的原文。
/// </summary>
public sealed record NewWorkSuggestion(string UnitPath, string UnitName, string SessionId, string? Title, string? Project, string? Parent,
    string? SessionFile = null);

/// <summary>自動建立任務：偵測到沒有任務的新工作時跳提示，建好後指定給那個 session。</summary>
public sealed partial class TimerEngine
{
    /// <summary>多久以內的改檔才算「正在改」；剛開工具時從 session 尾端讀到的舊編輯不算。</summary>
    static readonly TimeSpan EditFreshness = TimeSpan.FromMinutes(30);

    /// <summary>專案樹多久內可以直接拿來開視窗；超過 TreeFreshTime 的先顯示、再在背景重讀。</summary>
    static readonly TimeSpan TreeCacheTime = TimeSpan.FromMinutes(10);
    static readonly TimeSpan TreeFreshTime = TimeSpan.FromSeconds(30);

    /// <summary>建立任務小視窗用的專案樹快取：提示跳出時先讀好，按「建立任務」時視窗一開就有內容。</summary>
    readonly Dictionary<string, (DateTime At, Task<List<ProjectTask>> Load)> _treeCache = new(StringComparer.OrdinalIgnoreCase);

    /// <summary>「有新工作」的提示跳出、建立任務要用的資料也讀好了：App 先在背景把小視窗建好。</summary>
    public event Action<NewWorkSuggestion>? NewWorkReady;

    /// <summary>使用者按了「建立任務」：開建立任務的小視窗。</summary>
    public event Action<NewWorkSuggestion>? CreateTaskRequested;

    /// <summary>使用者按了「選既有任務」：開任務選擇視窗。</summary>
    public event Action<NewWorkSuggestion>? PickTaskRequested;

    /// <summary>
    /// 把任務指定給某個 Claude session（之後這個 session 有動作就為它計時）。
    /// startNow：馬上開始計時（標成 Claude 的），不用等下一次偵測到 Claude 在動。
    /// </summary>
    public async Task AssignSessionTaskAsync(string sessionId, string key, bool startNow = false)
    {
        key = key.Trim().ToUpperInvariant();
        _state.SessionTasks[sessionId] = key;
        _state.Save();
        _suppressed.Remove(key);
        _knownProjects.Add(KeyFinder.ProjectOf(key));
        Log.Info($"Claude session {sessionId} 指定為 {key}" + (startNow ? "，馬上開始計時" : ""));
        if (startNow)
            await Guarded(async ct =>
            {
                if (RunningKeys().Contains(key))
                    return;
                await _api.StartTimerAsync(key, ct);
                if (!IsClaudeOwned(key))
                    _state.ClaudeOwned.Add(key);
                _state.Save();
                await RefreshCoreAsync(ct);
            });
        await ClaudeTickAsync(wait: true);
    }

    /// <summary>記下這個資料夾用的專案與新任務的上層，下次建立任務時直接帶入。</summary>
    public void RememberFolder(string unitPath, string project, string? parent)
    {
        _state.FolderHints[unitPath] = new FolderHint { Project = project, Parent = parent };
        _state.Save();
    }

    /// <summary>資料夾裡的 session 對應到任務時，順便記下這個資料夾屬於哪個專案（換了專案，上次的上層就不適用）。</summary>
    void LearnFolderProjects()
    {
        var changed = false;
        foreach (var repo in _claude.Repos)
        {
            if (repo.ResolvedTask is not { } task)
                continue;
            var project = KeyFinder.ProjectOf(task);
            var hint = _state.FolderHints.GetValueOrDefault(repo.Path);
            if (string.Equals(hint?.Project, project, StringComparison.OrdinalIgnoreCase))
                continue;
            _state.FolderHints[repo.Path] = new FolderHint { Project = project };
            changed = true;
        }
        if (changed)
            _state.Save();
    }

    /// <summary>可以建立任務的專案（沒封存的）；同步時已經讀過就直接用。</summary>
    public Task<List<ProjectInfo>> CreatableProjectsAsync() =>
        Projects.Count > 0
            ? Task.FromResult(Projects.Where(p => !p.IsArchived).ToList())
            : Task.Run(() => _api.ListProjectsAsync(CancellationToken.None));

    /// <summary>新工作預設建在哪個專案：這個資料夾上次用的 → 名稱像資料夾的 → 正在計時的；都沒有就第一個。</summary>
    public ProjectInfo? GuessProject(IReadOnlyList<ProjectInfo> projects, NewWorkSuggestion suggestion)
    {
        var key = ProjectGuess.Guess(projects.Select(p => (p.Key, p.Name)).ToList(), suggestion.Project, suggestion.UnitName,
            RunningKeys().Select(KeyFinder.ProjectOf));
        return projects.FirstOrDefault(p => p.Key == key) ?? projects.FirstOrDefault();
    }

    /// <summary>
    /// 專案的全部任務（快取 10 分鐘）。在背景執行緒讀取與解析，不卡畫面；
    /// fresh：超過 30 秒的就重讀。
    /// </summary>
    public Task<List<ProjectTask>> ProjectTreeAsync(string project, bool fresh = false)
    {
        var now = DateTime.Now;
        if (_treeCache.TryGetValue(project, out var cached) && !cached.Load.IsFaulted && !cached.Load.IsCanceled
            && now - cached.At < (fresh ? TreeFreshTime : TreeCacheTime))
            return cached.Load;
        var load = Task.Run(() => _api.ListProjectTasksAsync(project, CancellationToken.None));
        _treeCache[project] = (now, load);
        return load;
    }

    /// <summary>已經讀好的專案樹（還在讀或沒讀過是 null）；Stale：超過 30 秒，顯示後要在背景重讀。</summary>
    public (List<ProjectTask> Tasks, bool Stale)? CachedProjectTree(string project) =>
        _treeCache.TryGetValue(project, out var c) && c.Load.IsCompletedSuccessfully && DateTime.Now - c.At < TreeCacheTime
            ? (c.Load.Result, DateTime.Now - c.At >= TreeFreshTime)
            : null;

    /// <summary>專案樹有變（例如剛建了任務）：下次重讀。</summary>
    public void ForgetProjectTree(string project) => _treeCache.Remove(project);

    /// <summary>提示跳出時先讀好預設專案的任務樹與狀態清單，使用者按「建立任務」時視窗一開就有內容。</summary>
    public async Task PrefetchNewWorkAsync(NewWorkSuggestion suggestion)
    {
        try
        {
            if (GuessProject(await CreatableProjectsAsync(), suggestion) is { } project)
                await Task.WhenAll(ProjectTreeAsync(project.Key), GetProjectAsync(project.Key));
            NewWorkReady?.Invoke(suggestion);
        }
        catch (Exception ex)
        {
            Log.Error("預先讀取建立任務用的專案樹失敗", ex);
        }
    }

    /// <summary>
    /// 有開「自動建立任務」的資料夾：Claude 最近在裡面改了檔、這個 session 又對應不到任務時，跳提示問一次。
    /// 只是問問題、規劃（沒改檔）不會跳；同一個 session 只問一次。
    /// </summary>
    void CheckNewWork(DateTime now)
    {
        if (!LoggedIn || _config.ClaudeOnlyDuringWork && !IsWorking(now))
            return;
        foreach (var repo in _claude.Repos)
        {
            if (!repo.AutoCreate || !repo.Active || repo.ResolvedTask is not null
                || repo.SessionId is not { } sid || repo.EditedAt is not { } edited || now - edited > EditFreshness
                || _state.AskedSessions.Contains(sid))
                continue;
            _state.AskedSessions.Add(sid);
            _state.Save();
            var hint = _state.FolderHints.GetValueOrDefault(repo.Path);
            var suggestion = new NewWorkSuggestion(repo.Path, repo.Name, sid, repo.FirstPrompt, hint?.Project, hint?.Parent, repo.SessionFile);
            Log.Info($"{repo.Name} 的 Claude session {sid} 在改程式但沒有任務，詢問要不要建立");
            Toast?.Invoke(new ToastRequest($"{repo.Name} 有新工作，要建立任務嗎？",
                repo.FirstPrompt ?? "Claude 正在改程式，但對應不到任務",
                [
                    new ToastAction("建立任務", () => Raise(CreateTaskRequested, suggestion), Primary: true),
                    new ToastAction("選既有任務", () => Raise(PickTaskRequested, suggestion)),
                    new ToastAction("不用", () => Task.CompletedTask),
                ], Sticky: true));
            _ = PrefetchNewWorkAsync(suggestion);
        }
    }

    /// <summary>
    /// 提示上的按鈕：等提示收起來（淡出約 160ms）再開視窗。
    /// 開視窗要建整個畫面，跟淡出擠在同一個時間會讓動畫頓一下。
    /// </summary>
    static Task Raise<T>(Action<T>? handler, T arg)
    {
        if (handler is not null)
            System.Windows.Application.Current.Dispatcher.InvokeAsync(async () =>
            {
                await Task.Delay(180);
                handler(arg);
            });
        return Task.CompletedTask;
    }
}
