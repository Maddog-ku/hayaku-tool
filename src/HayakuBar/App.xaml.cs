using System.Diagnostics;
using System.Text.Json;
using System.Windows;
using System.Windows.Media;
using System.Windows.Media.Imaging;
using System.Windows.Threading;
using HayakuBar.Auth;
using HayakuBar.Claude;
using HayakuBar.Config;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;
using HayakuBar.UI;
using Microsoft.Win32;

namespace HayakuBar;

public partial class App : Application
{
    Mutex? _mutex;
    TimerEngine? _engine;
    TaskbarWidget? _widget;
    PanelWindow? _panel;
    TrayIcon? _tray;
    MiniHttpServer? _hookServer;
    SettingsWindow? _settings;
    TimeEntriesWindow? _entries;
    UI.Work.WorkspaceWindow? _workspace;
    GlobalHotkey? _hotkey;
    HoverCard? _hoverCard;
    readonly ToastManager _toasts = new();
    readonly DispatcherTimer _widgetTick = new() { Interval = TimeSpan.FromSeconds(1) };
    readonly DispatcherTimer _hoverDelay = new() { Interval = TimeSpan.FromMilliseconds(450) };

    protected override async void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
        if (e.Args.Length > 0 && e.Args[0] == "--mcp-call")
        {
            var code = await Diagnostics.McpCallAsync(e.Args);
            Shutdown(code);
            return;
        }
        _mutex = new Mutex(true, "HayakuBar.SingleInstance", out var created);
        if (!created)
        {
            MessageBox.Show("HayakuBar 已經在執行了（看工作列）。", "HayakuBar");
            Shutdown();
            return;
        }

        DispatcherUnhandledException += (_, args) =>
        {
            Log.Error("未處理的例外", args.Exception);
            args.Handled = true;
        };
        TaskScheduler.UnobservedTaskException += (_, args) =>
        {
            Log.Error("未觀察的工作例外", args.Exception);
            args.SetObserved();
        };

        var config = AppConfig.Load();
        var state = AppState.Load();
        ThemeManager.Initialize(config.ThemeMode);
        var http = new HttpClient { Timeout = TimeSpan.FromSeconds(20) };
        http.DefaultRequestHeaders.UserAgent.ParseAdd("HayakuBar/1.0");
        var auth = new OAuthClient(http, config.BaseUrl);
        var api = new HayakuApi(new McpClient(http, auth, config.BaseUrl));
        var engine = _engine = new TimerEngine(config, state, auth, api, new ClaudeMonitor());

        _panel = new PanelWindow(engine);
        _panel.SettingsRequested += OpenSettings;
        _panel.TimeEntriesRequested += () => OpenTimeEntries(DateOnly.FromDateTime(DateTime.Today));
        _panel.WorkspaceRequested += OpenWorkspace;
        engine.TimeEntriesRequested += OpenTimeEntries;

        var tray = _tray = new TrayIcon(new TrayActions(
            TogglePanel: TogglePanel,
            Refresh: () => _ = engine.RefreshAsync(),
            Settings: OpenSettings,
            LoginOrLogout: () => _ = engine.LoggedIn ? engine.LogoutAsync() : engine.LoginAsync(),
            IsLoggedIn: () => engine.LoggedIn,
            GetAnchor: () => ParseAnchor(config.TaskbarAnchor),
            SetAnchor: SetAnchor,
            ResetOffset: () => SetOffset(0),
            GetLayout: () => ParseLayout(config.TaskbarLayout),
            SetLayout: SetLayout,
            TimeEntries: () => OpenTimeEntries(DateOnly.FromDateTime(DateTime.Today)),
            Workspace: () => OpenWorkspace("tasks", null),
            OpenWeb: () => OpenUrl(config.BaseUrl),
            OpenLog: () => OpenUrl(AppPaths.Dir),
            Exit: () => Shutdown()));
        engine.Notify += message => _toasts.Show(new ToastRequest("HayakuBar", message));
        engine.Toast += _toasts.Show;
        _toasts.Anchor = WidgetAnchor;

        var widget = _widget = new TaskbarWidget(ParseAnchor(config.TaskbarAnchor), config.TaskbarOffset, ParseLayout(config.TaskbarLayout));
        widget.Clicked += TogglePanel;
        widget.MenuRequested += tray.ShowMenu;
        widget.HoverChanged += OnWidgetHover;
        _hoverDelay.Tick += (_, _) =>
        {
            _hoverDelay.Stop();
            ShowHoverCard();
        };
        widget.OffsetCommitted += offset =>
        {
            config.TaskbarOffset = offset;
            config.Save();
        };
        widget.Start();

        engine.Changed += UpdateWidget;
        engine.ConfigApplied += () =>
        {
            widget.Anchor = ParseAnchor(config.TaskbarAnchor);
            widget.Layout = ParseLayout(config.TaskbarLayout);
            widget.Offset = config.TaskbarOffset;
            widget.Relayout();
            RegisterHotkey(showError: true);
        };
        _widgetTick.Tick += (_, _) => UpdateWidget();
        _widgetTick.Start();

        _hotkey = new GlobalHotkey();
        _hotkey.Pressed += TogglePanel;
        RegisterHotkey(showError: false);

        StartHookServer(config.HookPort);
        SystemEvents.PowerModeChanged += OnPowerModeChanged;
        SystemEvents.SessionSwitch += OnSessionSwitch;

        // --settings 或 --settings:claude 之類：啟動後直接開設定頁
        if (e.Args.FirstOrDefault(a => a.StartsWith("--settings", StringComparison.OrdinalIgnoreCase)) is { } arg)
            OpenSettings(arg.Contains(':') ? arg[(arg.IndexOf(':') + 1)..] : null);

        // --toast-test：顯示一則範例提示（檢查外觀用）
        if (e.Args.Any(a => a.Equals("--toast-test", StringComparison.OrdinalIgnoreCase)))
            _toasts.Show(new ToastRequest("會議 14:00・3 分鐘後開始", "HAYAKU專案管理系統介紹會議-3",
            [
                new ToastAction("加入並計時", () => Task.CompletedTask, Primary: true),
                new ToastAction("開啟", () => Task.CompletedTask),
            ], Sticky: true));

        // --entries：啟動後直接開工時紀錄
        if (e.Args.Any(a => a.Equals("--entries", StringComparison.OrdinalIgnoreCase)))
            _ = Dispatcher.InvokeAsync(() => OpenTimeEntries(DateOnly.FromDateTime(DateTime.Today)), DispatcherPriority.ApplicationIdle);

        // --workspace 或 --workspace:hours 之類：啟動後直接開工作台
        if (e.Args.FirstOrDefault(a => a.StartsWith("--workspace", StringComparison.OrdinalIgnoreCase)) is { } wsArg)
            _ = Dispatcher.InvokeAsync(() => OpenWorkspace(wsArg.Contains(':') ? wsArg[(wsArg.IndexOf(':') + 1)..] : "tasks", null),
                DispatcherPriority.ApplicationIdle);

        // --shot:tasks,hours,panel-today…：開啟各頁後把畫面存成 PNG（檢查外觀用，只拍這個程式自己的視窗）
        if (e.Args.FirstOrDefault(a => a.StartsWith("--shot", StringComparison.OrdinalIgnoreCase)) is { } shotArg)
            _ = ShootAsync(shotArg.Contains(':') ? shotArg[(shotArg.IndexOf(':') + 1)..] : "tasks");

        // --panel：啟動後直接打開面板
        if (e.Args.FirstOrDefault(a => a.StartsWith("--panel", StringComparison.OrdinalIgnoreCase)) is { } panelArg)
        {
            _panel.StayOpen = true;
            if (panelArg.Contains(':'))
                _panel.SelectTab(panelArg[(panelArg.IndexOf(':') + 1)..] switch
                {
                    "today" => "Today",
                    "notify" => "Notify",
                    _ => "Timer",
                });
            _ = Dispatcher.InvokeAsync(TogglePanel, DispatcherPriority.ApplicationIdle);
        }

        await engine.StartAsync();
        Log.Info("HayakuBar 啟動");
    }

    protected override void OnExit(ExitEventArgs e)
    {
        SystemEvents.PowerModeChanged -= OnPowerModeChanged;
        SystemEvents.SessionSwitch -= OnSessionSwitch;
        _widgetTick.Stop();
        _hotkey?.Dispose();
        _widget?.Dispose();
        _hookServer?.Dispose();
        _tray?.Dispose();
        _mutex?.Dispose();
        base.OnExit(e);
    }

    // ───────────── 工作列與面板 ─────────────

    void UpdateWidget()
    {
        if (_engine is not { } engine || _widget is null)
            return;
        WidgetContent content;
        string tooltip;
        if (engine.InitialSyncing)
        {
            content = new WidgetContent("同步中", "", 0, 0, false, WidgetStatus.Idle);
            tooltip = "HayakuBar · 同步中，請稍候";
        }
        else if (!engine.LoggedIn)
        {
            content = new WidgetContent(engine.LoggingIn ? "登入中" : "未登入", "", 0, 0, false,
                engine.Error is null ? WidgetStatus.Idle : WidgetStatus.Error);
            tooltip = "HayakuBar · 未登入";
        }
        else
        {
            var now = DateTime.Now;
            var worked = WorkMath.UnionMinutes(engine.Entries, now.Date, now);
            var expected = engine.Schedule?.ExpectedMinutes ?? 0;
            var running = engine.RunningKeys().Count;
            content = new WidgetContent(
                WorkMath.HoursText(worked),
                expected > 0 ? "/ " + WorkMath.HoursText(expected) : "",
                expected > 0 ? worked / expected : 0,
                running,
                engine.Claude.Repos.Any(r => r.Active),
                engine.Error is not null ? WidgetStatus.Error : running > 0 ? WidgetStatus.Running : WidgetStatus.Idle,
                engine.UnreadCount);
            tooltip = $"HayakuBar · 今天 {WorkMath.HoursText(worked)} / {WorkMath.HoursText(expected)} · 計時中 {running}";
        }
        _widget.Update(content);
        _tray?.SetTooltip(tooltip);
    }

    void TogglePanel()
    {
        if (_panel is null || _widget is null)
            return;
        if (_panel.IsOpen)
        {
            _panel.HidePanel();
            return;
        }
        // 點工作列時面板先因失去焦點收起，同一下點擊不要又把它打開
        if ((DateTime.Now - _panel.HiddenAt).TotalMilliseconds < 400)
            return;
        _hoverDelay.Stop();
        _hoverCard?.Hide();
        var (anchor, alignLeft) = WidgetAnchor();
        _panel.ShowAt(anchor, alignLeft);
    }

    /// <summary>工作列上工時的位置（DIP）；面板、摘要都從它正上方出現。</summary>
    (Rect Anchor, bool AlignLeft) WidgetAnchor()
    {
        if (_widget is { IsEmbedded: true } widget)
        {
            var r = widget.ScreenRect;
            var tb = widget.TaskbarRect;
            var s = widget.Scale;
            return (new Rect(r.Left / s, tb.Top / s, r.Width / s, tb.Height / s), (r.Left + r.Right) / 2 < (tb.Left + tb.Right) / 2);
        }
        var work = SystemParameters.WorkArea;
        return (new Rect(work.Right - 12, work.Bottom, 0, 0), false);
    }

    void OnWidgetHover(bool hovering)
    {
        if (hovering)
        {
            _hoverDelay.Stop();
            _hoverDelay.Start();
            return;
        }
        _hoverDelay.Stop();
        _hoverCard?.Hide();
    }

    void ShowHoverCard()
    {
        if (_engine is not { LoggedIn: true } engine || _panel is { IsOpen: true })
            return;
        _hoverCard ??= new HoverCard();
        var (anchor, alignLeft) = WidgetAnchor();
        _hoverCard.ShowFor(engine, anchor, alignLeft);
    }

    void RegisterHotkey(bool showError)
    {
        if (_hotkey is null || _engine is null)
            return;
        var text = _engine.Config.Hotkey;
        if (!_hotkey.Register(text) && showError)
            _toasts.Show(new ToastRequest("快捷鍵無法使用", $"「{text}」可能被其他程式占用，請在設定換一組。"));
    }

    void OpenTimeEntries(DateOnly day)
    {
        if (_engine is null)
            return;
        if (_entries is { IsLoaded: true })
        {
            _ = _entries.ShowDayAsync(day);
            _entries.Activate();
            return;
        }
        _entries = new TimeEntriesWindow(_engine, day);
        _entries.Show();
        _entries.Activate();
    }

    /// <summary>開工作台到指定頁（tasks、hours、worklog、reminders、meetings、chat、team、leaves、stats）。</summary>
    void OpenWorkspace(string page, object? arg)
    {
        if (_engine is null)
            return;
        if (_workspace is null)
        {
            _workspace = new UI.Work.WorkspaceWindow(_engine);
            _workspace.DayRequested += OpenTimeEntries;
            _workspace.Closed += (_, _) => _workspace = null;
            _workspace.Show();
        }
        if (_workspace.WindowState == WindowState.Minimized)
            _workspace.WindowState = WindowState.Normal;
        _workspace.ShowPage(page, arg);
        _workspace.Activate();
    }

    async Task ShootAsync(string pages)
    {
        var dir = Path.Combine(AppPaths.Dir, "shots");
        Directory.CreateDirectory(dir);
        while (_engine is null || _engine.InitialSyncing)
            await Task.Delay(500);
        await Task.Delay(1500);
        foreach (var page in pages.Split(',', StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
        {
            Window? target;
            if (page == "toast")
            {
                // 範例：新通知的彈窗（從下面滑上來）
                _toasts.Show(new ToastRequest("申請已審核", "丁鈺昕：F2P-101 面板新增全部列印及選擇列印按鈕",
                    [new ToastAction("標為已讀", () => Task.CompletedTask, Primary: true), new ToastAction("查看", () => Task.CompletedTask)],
                    Duration: TimeSpan.FromSeconds(30), FromBottom: true));
                await Task.Delay(1500);
                target = Windows.OfType<ToastWindow>().LastOrDefault();
                if (target is not null)
                    Log.Info($"截圖：通知彈窗位置 {target.Left:0},{target.Top:0} 寬 {target.ActualWidth:0}；工時位置 {WidgetAnchor()}");
            }
            else if (page.StartsWith("settings-", StringComparison.OrdinalIgnoreCase))
            {
                OpenSettings(page["settings-".Length..]);
                target = _settings;
            }
            else if (page.StartsWith("panel", StringComparison.OrdinalIgnoreCase) && _panel is not null)
            {
                _panel.StayOpen = true;
                _panel.SelectTab(page.Contains('-') ? char.ToUpperInvariant(page[(page.IndexOf('-') + 1)]) + page[(page.IndexOf('-') + 2)..] : "Timer");
                if (!_panel.IsOpen)
                    TogglePanel();
                target = _panel;
            }
            else
            {
                _panel?.HidePanel();
                // tasks=F2P-97、meetings=new：等號後面是要帶給那一頁的參數
                var parts = page.Split('=', 2);
                OpenWorkspace(parts[0], parts.Length > 1 ? parts[1] : null);
                target = _workspace;
            }
            await Task.Delay(4000);
            if (target is null)
                continue;
            try
            {
                var dpi = VisualTreeHelper.GetDpi(target);
                var bmp = new RenderTargetBitmap((int)(target.ActualWidth * dpi.DpiScaleX), (int)(target.ActualHeight * dpi.DpiScaleY),
                    dpi.PixelsPerInchX, dpi.PixelsPerInchY, PixelFormats.Pbgra32);
                bmp.Render(target);
                var encoder = new PngBitmapEncoder();
                encoder.Frames.Add(BitmapFrame.Create(bmp));
                await using var file = File.Create(Path.Combine(dir, $"{page}.png"));
                encoder.Save(file);
            }
            catch (Exception ex)
            {
                Log.Error($"截圖 {page} 失敗", ex);
            }
        }
        Log.Info($"截圖完成：{dir}");
    }

    void SetAnchor(TaskbarAnchor anchor)
    {
        if (_engine is null || _widget is null)
            return;
        _engine.Config.TaskbarAnchor = anchor.ToString();
        _engine.Config.TaskbarOffset = 0;
        _engine.Config.Save();
        _widget.Anchor = anchor;
        _widget.Offset = 0;
        _widget.Relayout();
    }

    void SetOffset(double offset)
    {
        if (_engine is null || _widget is null)
            return;
        _engine.Config.TaskbarOffset = offset;
        _engine.Config.Save();
        _widget.Offset = offset;
        _widget.Relayout();
    }

    void SetLayout(WidgetLayout layout)
    {
        if (_engine is null || _widget is null)
            return;
        _engine.Config.TaskbarLayout = layout.ToString();
        _engine.Config.Save();
        _widget.Layout = layout;
        _widget.Relayout();
    }

    static WidgetLayout ParseLayout(string? value) =>
        Enum.TryParse<WidgetLayout>(value, ignoreCase: true, out var layout) ? layout : WidgetLayout.TwoLine;

    static TaskbarAnchor ParseAnchor(string? value) =>
        Enum.TryParse<TaskbarAnchor>(value, ignoreCase: true, out var anchor) ? anchor : TaskbarAnchor.BeforeTray;

    // ───────────── 其他 ─────────────

    void StartHookServer(int port)
    {
        try
        {
            _hookServer = new MiniHttpServer(port, async req =>
            {
                if (req.Path == "/health")
                    return new MiniResponse(200, "text/plain", "ok");
                if (req.Method != "POST" || req.Path != "/hook")
                    return new MiniResponse(404, "text/plain", "not found");
                JsonElement payload;
                try
                {
                    using var doc = JsonDocument.Parse(req.Body);
                    payload = doc.RootElement.Clone();
                }
                catch (JsonException)
                {
                    return new MiniResponse(400, "text/plain", "bad json");
                }
                await Dispatcher.InvokeAsync(() => _engine?.OnHook(payload));
                return new MiniResponse(200, "application/json", "{}");
            });
            _hookServer.Start();
        }
        catch (Exception ex)
        {
            Log.Error($"Claude hook 接收埠 {port} 無法啟動", ex);
        }
    }

    void OpenSettings() => OpenSettings(null);

    void OpenSettings(string? page)
    {
        if (_engine is null)
            return;
        if (_settings is not { IsLoaded: true })
        {
            _settings = new SettingsWindow(_engine);
            _settings.Show();
        }
        if (page is not null)
            _settings.ShowPage(page);
        _settings.Activate();
    }

    void OnPowerModeChanged(object? sender, PowerModeChangedEventArgs e)
    {
        if (e.Mode == PowerModes.Resume)
            Dispatcher.InvokeAsync(() => _engine?.AutoRefreshAsync());
    }

    void OnSessionSwitch(object? sender, SessionSwitchEventArgs e)
    {
        if (e.Reason is SessionSwitchReason.SessionUnlock or SessionSwitchReason.SessionLogon)
            Dispatcher.InvokeAsync(() => _engine?.AutoRefreshAsync());
    }

    static void OpenUrl(string target)
    {
        try
        {
            Process.Start(new ProcessStartInfo(target) { UseShellExecute = true });
        }
        catch (Exception ex)
        {
            Log.Error($"開啟 {target} 失敗", ex);
        }
    }
}
