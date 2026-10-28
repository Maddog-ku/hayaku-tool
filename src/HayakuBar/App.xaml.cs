using System.Diagnostics;
using System.Text.Json;
using System.Windows;
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
    readonly DispatcherTimer _widgetTick = new() { Interval = TimeSpan.FromSeconds(1) };

    protected override async void OnStartup(StartupEventArgs e)
    {
        base.OnStartup(e);
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
        var http = new HttpClient { Timeout = TimeSpan.FromSeconds(20) };
        http.DefaultRequestHeaders.UserAgent.ParseAdd("HayakuBar/1.0");
        var auth = new OAuthClient(http, config.BaseUrl);
        var api = new HayakuApi(new McpClient(http, auth, config.BaseUrl));
        var engine = _engine = new TimerEngine(config, state, auth, api, new ClaudeMonitor());

        _panel = new PanelWindow(engine);
        _panel.SettingsRequested += OpenSettings;

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
            OpenWeb: () => OpenUrl(config.BaseUrl),
            OpenLog: () => OpenUrl(AppPaths.Dir),
            Exit: () => Shutdown()));
        engine.Notify += message => tray.Balloon("HayakuBar", message);

        var widget = _widget = new TaskbarWidget(ParseAnchor(config.TaskbarAnchor), config.TaskbarOffset, ParseLayout(config.TaskbarLayout));
        widget.Clicked += TogglePanel;
        widget.MenuRequested += tray.ShowMenu;
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
        };
        _widgetTick.Tick += (_, _) => UpdateWidget();
        _widgetTick.Start();

        StartHookServer(config.HookPort);
        SystemEvents.PowerModeChanged += OnPowerModeChanged;
        SystemEvents.SessionSwitch += OnSessionSwitch;

        await engine.StartAsync();
        Log.Info("HayakuBar 啟動");
    }

    protected override void OnExit(ExitEventArgs e)
    {
        SystemEvents.PowerModeChanged -= OnPowerModeChanged;
        SystemEvents.SessionSwitch -= OnSessionSwitch;
        _widgetTick.Stop();
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
        if (!engine.LoggedIn)
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
                engine.Error is not null ? WidgetStatus.Error : running > 0 ? WidgetStatus.Running : WidgetStatus.Idle);
            tooltip = $"HayakuBar · 今天 {WorkMath.HoursText(worked)} / {WorkMath.HoursText(expected)} · 計時中 {running}";
        }
        _widget.Update(content);
        _tray?.SetTooltip(tooltip);
    }

    void TogglePanel()
    {
        if (_panel is null || _widget is null)
            return;
        if (_panel.IsVisible)
        {
            _panel.HidePanel();
            return;
        }
        // 點工作列時面板先因失去焦點收起，同一下點擊不要又把它打開
        if ((DateTime.Now - _panel.HiddenAt).TotalMilliseconds < 400)
            return;

        Rect anchor;
        bool alignLeft;
        if (_widget.IsEmbedded)
        {
            var r = _widget.ScreenRect;
            var tb = _widget.TaskbarRect;
            var s = _widget.Scale;
            anchor = new Rect(r.Left / s, tb.Top / s, r.Width / s, tb.Height / s);
            alignLeft = (r.Left + r.Right) / 2 < (tb.Left + tb.Right) / 2;
        }
        else
        {
            var work = SystemParameters.WorkArea;
            anchor = new Rect(work.Right - 12, work.Bottom, 0, 0);
            alignLeft = false;
        }
        _panel.ShowAt(anchor, alignLeft);
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

    void OpenSettings()
    {
        if (_engine is null)
            return;
        if (_settings is { IsLoaded: true })
        {
            _settings.Activate();
            return;
        }
        _settings = new SettingsWindow(_engine);
        _settings.Show();
    }

    void OnPowerModeChanged(object? sender, PowerModeChangedEventArgs e)
    {
        if (e.Mode == PowerModes.Resume)
            Dispatcher.InvokeAsync(() => _engine?.RefreshAsync());
    }

    void OnSessionSwitch(object? sender, SessionSwitchEventArgs e)
    {
        if (e.Reason is SessionSwitchReason.SessionUnlock or SessionSwitchReason.SessionLogon)
            Dispatcher.InvokeAsync(() => _engine?.RefreshAsync());
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
