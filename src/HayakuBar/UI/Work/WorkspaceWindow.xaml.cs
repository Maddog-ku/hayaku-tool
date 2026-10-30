using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI.Work;

/// <summary>
/// 工作台：把網頁上常用的操作搬到桌面（任務、工時月曆、工作日誌、提醒、會議、聊天、團隊、請假、統計）。
/// 左側導覽、右側一頁一個 UserControl，第一次切到才建立。
/// </summary>
public partial class WorkspaceWindow : Window
{
    readonly TimerEngine _engine;
    readonly Dictionary<string, FrameworkElement> _pages = [];
    object? _pendingArg;

    public WorkspaceWindow(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        SourceInitialized += (_, _) => ThemeManager.ApplyTitleBar(this);
        _engine.Changed += RefreshChrome;
        Closed += (_, _) => _engine.Changed -= RefreshChrome;
        PreviewKeyDown += OnPreviewKeyDown;
        // 背景同步不抓任務分頁、團隊、待辦，切到工作台時才更新（20 秒內重複切換不會重抓）
        Activated += (_, _) => _ = _engine.RefreshAllAsync(userAction: false);
        RefreshChrome();
    }

    /// <summary>頁面：tasks、projects、reviews、hours、entries、worklog、reminders、meetings、chat、team、activity、workload、leaves、eipleaves、stats。</summary>
    public void ShowPage(string page, object? arg = null)
    {
        _pendingArg = arg;
        var nav = NavPanel.Children.OfType<RadioButton>().FirstOrDefault(r => string.Equals(r.Tag as string, page, StringComparison.OrdinalIgnoreCase))
                  ?? NavPanel.Children.OfType<RadioButton>().First();
        if (nav.IsChecked == true)
            _ = ActivateAsync((string)nav.Tag);
        else
            nav.IsChecked = true;
    }

    void Nav_Checked(object sender, RoutedEventArgs e)
    {
        if (sender is RadioButton { Tag: string page })
            _ = ActivateAsync(page);
    }

    async Task ActivateAsync(string page)
    {
        if (!_pages.TryGetValue(page, out var view))
        {
            view = page switch
            {
                "hours" => new HoursPage(_engine, this),
                "entries" => new DayEntriesPage(_engine),
                "worklog" => new WorkLogPage(_engine),
                "reminders" => new RemindersPage(_engine),
                "meetings" => new MeetingsPage(_engine),
                "chat" => new ChatPage(_engine),
                "team" => new TeamPage(_engine, this),
                "leaves" => new LeavesPage(_engine),
                "eipleaves" => new EipLeavesPage(_engine),
                "stats" => new StatsPage(_engine),
                "reviews" => new ReviewsPage(_engine, this),
                "activity" => new ActivityPage(_engine, this),
                "workload" => new WorkloadPage(_engine, this),
                "projects" => new ProjectsPage(_engine, this),
                _ => new TasksPage(_engine, this),
            };
            _pages[page] = view;
        }
        PageHost.Content = view;
        var arg = _pendingArg;
        _pendingArg = null;
        try
        {
            await ((IWorkPage)view).ActivateAsync(arg);
        }
        catch (Exception ex)
        {
            Log.Error($"工作台 {page} 載入失敗", ex);
        }
    }

    void RefreshChrome()
    {
        UserText.Text = _engine.User?.DisplayName ?? "";
        var overdue = _engine.OverdueCount;
        TasksBadge.Text = overdue > 0 ? $"逾期 {overdue}" : "";
        var reviews = _engine.PendingReviews.Count;
        ReviewsBadge.Text = reviews > 0 ? reviews.ToString() : "";
    }

    void OnPreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (Keyboard.Modifiers != ModifierKeys.Control || e.Key is < Key.D0 or > Key.D9)
            return;
        var navs = NavPanel.Children.OfType<RadioButton>().ToList();
        var index = e.Key == Key.D0 ? 9 : e.Key - Key.D1;
        if (index < navs.Count)
        {
            navs[index].IsChecked = true;
            e.Handled = true;
        }
    }

    /// <summary>切到「工時紀錄」看某一天。</summary>
    public void OpenDay(DateOnly day) => ShowPage("entries", day);

    void OpenWeb_Click(object sender, RoutedEventArgs e) => Web.Open(_engine.Config.BaseUrl);
}
