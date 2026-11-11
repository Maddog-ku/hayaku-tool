using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;
using Drag = HayakuBar.Core.TaskSchedule.Drag;

namespace HayakuBar.UI.Work;

/// <summary>
/// 我的任務的日曆、甘特圖共用：任務條的外觀、拖拉、改預計日期（Hayaku 1.5.2）。
/// 資料跟清單同一份（目前分頁＋專案、狀態、只顯示沒有子任務的篩選），不多讀；放開滑鼠才送一次 update_task。
/// </summary>
public partial class TasksPage
{
    static readonly string[] Views = ["list", "tree", "kanban", "calendar", "gantt"];

    RadioButton ViewButton(string view) => view switch
    {
        "tree" => TreeViewButton,
        "kanban" => KanbanViewButton,
        "calendar" => CalendarViewButton,
        "gantt" => GanttViewButton,
        _ => ListViewButton,
    };

    /// <summary>剛拖完、還在等伺服器的新日期：先畫在新位置，同步完（或失敗）才拿掉。</summary>
    readonly Dictionary<string, (DateOnly? Start, DateOnly? End)> _dateOverrides = new(StringComparer.OrdinalIgnoreCase);

    List<TaskSummary> _scheduleTasks = [];
    string? _scheduleView;

    /// <summary>拖拉中的任務條。</summary>
    sealed class ScheduleDragState
    {
        public required TaskSummary Task { get; init; }
        public required Drag Mode { get; init; }
        public required Point Origin { get; init; }
        public DateOnly? GrabDay { get; init; }
        /// <summary>沒有預計日期的任務：拖到哪一天（或拉出哪一段）就設成那樣。</summary>
        public bool Undated { get; init; }
        public bool Started { get; set; }
        public (DateOnly? Start, DateOnly? End) Preview { get; set; }
    }

    ScheduleDragState? _drag;

    void InitSchedule()
    {
        CalendarView.SizeChanged += (_, e) =>
        {
            if (_scheduleView == "calendar" && Math.Abs(e.NewSize.Height - e.PreviousSize.Height) > 4)
                RenderCalendar();
        };
        GanttView.SizeChanged += (_, e) =>
        {
            if (_scheduleView == "gantt" && Math.Abs(e.NewSize.Width - e.PreviousSize.Width) > 4)
                RenderGantt();
        };
    }

    /// <summary>RenderList 畫完清單後呼叫：view 是 calendar、gantt 或 null（不是這兩種）。</summary>
    void RenderSchedule(string? view, List<TaskSummary> shown)
    {
        _scheduleView = view;
        _scheduleTasks = shown;
        CalendarView.Visibility = view == "calendar" ? Visibility.Visible : Visibility.Collapsed;
        GanttView.Visibility = view == "gantt" ? Visibility.Visible : Visibility.Collapsed;
        if (view is not null && shown.Count > 0)
            ListMessage.Text = "";
        if (_drag is { Started: true })
        {
            return; // 拖拉中不重畫（會中斷拖拉）；放開時一定會重畫
        }
        if (view == "calendar")
            RenderCalendar();
        else
            CalendarView.Children.Clear();
        if (view == "gantt")
            RenderGantt();
        else
            GanttView.Children.Clear();
    }

    void RerenderSchedule()
    {
        if (_scheduleView == "calendar")
            RenderCalendar();
        else if (_scheduleView == "gantt")
            RenderGantt();
    }

    (DateOnly? Start, DateOnly? End) DatesOf(TaskSummary t) =>
        _dateOverrides.TryGetValue(t.Key, out var o) ? o : (t.PlannedStart, t.PlannedEnd);

    static DateOnly Today => DateOnly.FromDateTime(DateTime.Today);

    static bool IsDone(TaskSummary t) => t.StatusCategory is "DONE" or "CANCELLED";

    bool IsOverdue(TaskSummary t) => !IsDone(t) && TaskSchedule.Span(DatesOf(t).Start, DatesOf(t).End) is { } span && span.End < Today;

    Color ProjectColor(TaskSummary t)
    {
        var project = _engine.ProjectOf(t.Project) ?? _engine.ProjectOf(KeyFinder.ProjectOf(t.Key));
        return ProjectIcons.ColorOf(project?.IconColor, project?.Key ?? KeyFinder.ProjectOf(t.Key));
    }

    /// <summary>任務條：專案顏色的淡底＋實線邊，逾期是紅邊，完成的變淡，計時中前面加 ●；甘特圖左邊已經有編號，條上只放名稱。</summary>
    Border ScheduleBar(TaskSummary t, bool fromBefore = false, bool toAfter = false, bool showKey = true)
    {
        var color = ProjectColor(t);
        var running = t.Timing || _engine.RunningKeys().Contains(t.Key);
        var text = new TextBlock
        {
            TextTrimming = TextTrimming.CharacterEllipsis,
            VerticalAlignment = VerticalAlignment.Center,
            FontSize = 12,
            Foreground = Brushes2.Get("TextBrush"),
        };
        if (running)
            text.Inlines.Add(new System.Windows.Documents.Run("● ") { Foreground = Brushes2.Get("RunBrush") });
        if (showKey)
            text.Inlines.Add(new System.Windows.Documents.Run(t.Key + " ") { Foreground = Brushes2.Get("MutedBrush") });
        text.Inlines.Add(new System.Windows.Documents.Run(TaskText.Title(t)));
        var (start, end) = DatesOf(t);
        return new Border
        {
            Background = new SolidColorBrush(Color.FromArgb(0x30, color.R, color.G, color.B)),
            BorderBrush = IsOverdue(t) ? Brushes2.Get("ErrorBrush") : new SolidColorBrush(color),
            BorderThickness = new Thickness(fromBefore ? 0 : 3, 1, toAfter ? 0 : 1, 1),
            CornerRadius = new CornerRadius(fromBefore ? 0 : 4, toAfter ? 0 : 4, toAfter ? 0 : 4, fromBefore ? 0 : 4),
            Padding = new Thickness(5, 0, 4, 0),
            Opacity = IsDone(t) ? 0.55 : 1,
            Cursor = Cursors.Hand,
            Child = text,
            ToolTip = $"{t.Key} {TaskText.Title(t)}\n{t.Project}・{t.Status}\n預計 {TaskSchedule.RangeText(start, end)}\n點一下查看；拖拉改預計日期",
        };
    }

    /// <summary>任務條的右鍵選單跟清單一樣。</summary>
    void HookBarMenu(FrameworkElement bar, TaskSummary t) =>
        bar.MouseRightButtonUp += (_, e) =>
        {
            e.Handled = true;
            ShowRowMenu(t.Key, TaskText.Title(t), t.ChildCount, bar);
        };

    /// <summary>開始拖（還沒移動超過幾個像素前算點一下）。</summary>
    void BeginDrag(FrameworkElement source, MouseButtonEventArgs e, ScheduleDragState state)
    {
        _drag = state;
        source.CaptureMouse();
        e.Handled = true;
    }

    /// <summary>移動超過門檻才算開始拖。</summary>
    static bool PastThreshold(ScheduleDragState d, Point now) =>
        d.Started || Math.Abs(now.X - d.Origin.X) > 4 || Math.Abs(now.Y - d.Origin.Y) > 4;

    /// <summary>放開：沒拖動算點一下（打開任務），拖了就改日期。</summary>
    void EndDrag(FrameworkElement source)
    {
        var d = _drag;
        _drag = null;
        if (source.IsMouseCaptured)
            source.ReleaseMouseCapture();
        if (d is null)
            return;
        if (!d.Started)
        {
            _ = OpenTaskAsync(d.Task.Key);
            return;
        }
        var (oldStart, oldEnd) = DatesOf(d.Task);
        if (d.Preview.Start is null && d.Preview.End is null
            || TaskSchedule.Changes(oldStart, oldEnd, d.Preview.Start, d.Preview.End).Count == 0)
        {
            RerenderSchedule();
            return;
        }
        _ = MoveDatesAsync(d.Task, d.Preview.Start, d.Preview.End);
    }

    /// <summary>拖拉被中斷（例如視窗失去焦點）：放棄，畫回原位。</summary>
    void CancelDrag()
    {
        if (_drag is null)
            return;
        _drag = null;
        RerenderSchedule();
    }

    /// <summary>
    /// 送出新的預計日期（update_task）：先畫在新位置，伺服器的說明（例如執行者送出變更申請）照樣提示；
    /// 同步完照伺服器的資料重畫，失敗就畫回原位。
    /// </summary>
    async Task MoveDatesAsync(TaskSummary t, DateOnly? start, DateOnly? end)
    {
        var (oldStart, oldEnd) = DatesOf(t);
        var fields = TaskSchedule.Changes(oldStart, oldEnd, start, end)
            .ToDictionary(kv => kv.Key, kv => (object?)Inputs.DateText(kv.Value));
        _dateOverrides[t.Key] = (start ?? oldStart, end ?? oldEnd);
        RerenderSchedule();
        try
        {
            var message = await _engine.Api.UpdateTaskAsync(t.Key, fields, CancellationToken.None);
            Log.Info($"{t.Key} 拖拉改預計日期：{TaskSchedule.RangeText(oldStart, oldEnd)} → {TaskSchedule.RangeText(start ?? oldStart, end ?? oldEnd)}");
            _engine.ShowNotice(message is { Length: > 0 } ? message : $"{t.Key} 預計 {TaskSchedule.RangeText(start ?? oldStart, end ?? oldEnd)}");
            if (_doneTasks.Any(d => d.Key.Equals(t.Key, StringComparison.OrdinalIgnoreCase)))
                _doneFetched = DateTime.MinValue; // 已完成的那份不在同步範圍，下次畫的時候重讀
            await _engine.AfterExternalChangeAsync();
        }
        catch (Exception ex)
        {
            Log.Error($"{t.Key} 改預計日期失敗", ex);
            _engine.ShowNotice($"{t.Key} 改預計日期失敗：{ex.Message}");
        }
        finally
        {
            _dateOverrides.Remove(t.Key);
            RenderList(keepSelection: true);
        }
    }

    /// <summary>日曆、甘特圖上方的一列：‹ 今天 › 與標題，右邊放說明。</summary>
    DockPanel ScheduleToolbar(string title, string prevTip, string nextTip, Action prev, Action today, Action next, string hint)
    {
        Button Nav(string content, string tip, Action action)
        {
            var b = new Button { Content = content, ToolTip = tip, Style = (Style)FindResource("FlatButton"), Padding = new Thickness(10, 3, 10, 3), Margin = new Thickness(0, 0, 4, 0) };
            b.Click += (_, _) => action();
            return b;
        }
        var bar = new DockPanel { Margin = new Thickness(0, 4, 0, 6) };
        var left = new StackPanel { Orientation = Orientation.Horizontal };
        left.Children.Add(Nav("‹", prevTip, prev));
        left.Children.Add(Nav("今天", "回到今天", today));
        left.Children.Add(Nav("›", nextTip, next));
        left.Children.Add(new TextBlock { Text = title, FontSize = 15, FontWeight = FontWeights.SemiBold, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(8, 0, 0, 0) });
        var hintText = new TextBlock { Text = hint, Foreground = Brushes2.Get("HintBrush"), FontSize = 12, VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis, Margin = new Thickness(12, 0, 0, 0) };
        DockPanel.SetDock(left, Dock.Left);
        bar.Children.Add(left);
        bar.Children.Add(hintText);
        hintText.HorizontalAlignment = HorizontalAlignment.Right;
        return bar;
    }
}
