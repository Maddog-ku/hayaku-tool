using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using HayakuBar.Core;
using HayakuBar.Mcp;
using Drag = HayakuBar.Core.TaskSchedule.Drag;

namespace HayakuBar.UI.Work;

/// <summary>
/// 我的任務的日曆（月曆，星期一開頭）：任務從預計開始畫到預計結束，拖到別天起訖一起平移（天數不變）；
/// 沒有預計日期的列在右邊，拖到日期上就把起訖都設成那天。放不下的那天顯示「+N」，點了選要打開哪個。
/// </summary>
public partial class TasksPage
{
    static readonly string[] WeekdayNames = ["一", "二", "三", "四", "五", "六", "日"];
    const double CalendarDateRow = 20;
    const double CalendarLaneRow = 17;
    const double CalendarMoreRow = 14;

    DateOnly _calendarMonth = new(DateTime.Today.Year, DateTime.Today.Month, 1);
    Grid? _calendarWeeks;
    DateOnly _calendarFirst;
    int _calendarWeekCount;
    readonly Dictionary<DateOnly, Border> _calendarHighlights = [];

    void RenderCalendar()
    {
        CalendarView.Children.Clear();
        _calendarHighlights.Clear();
        var (first, weeks) = TaskSchedule.MonthGrid(_calendarMonth.Year, _calendarMonth.Month);
        _calendarFirst = first;
        _calendarWeekCount = weeks;
        var dated = new List<(TaskSummary Item, DateOnly Start, DateOnly End)>();
        var undated = new List<TaskSummary>();
        foreach (var t in _scheduleTasks)
        {
            var (start, end) = DatesOf(t);
            if (TaskSchedule.Span(start, end) is { } span)
                dated.Add((t, span.Start, span.End));
            else
                undated.Add(t);
        }

        var root = new DockPanel();
        var toolbar = ScheduleToolbar($"{_calendarMonth.Year} 年 {_calendarMonth.Month} 月", "上個月", "下個月",
            () => ShiftMonth(-1), () => { _calendarMonth = new(Today.Year, Today.Month, 1); RenderCalendar(); }, () => ShiftMonth(1),
            "拖到別天改預計日期（天數不變）");
        DockPanel.SetDock(toolbar, Dock.Top);
        root.Children.Add(toolbar);
        if (undated.Count > 0)
        {
            var side = UndatedPanel(undated);
            DockPanel.SetDock(side, Dock.Right);
            root.Children.Add(side);
        }

        var month = new DockPanel();
        var header = new UniformGrid7();
        for (var i = 0; i < 7; i++)
            header.Add(new TextBlock
            {
                Text = WeekdayNames[i], FontSize = 12, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 0, 0, 4),
                Foreground = Brushes2.Get(i >= 5 ? "HintBrush" : "MutedBrush"),
            }, i);
        DockPanel.SetDock(header.Grid, Dock.Top);
        month.Children.Add(header.Grid);

        // 每週能放幾列任務：看日曆實際高度
        var available = CalendarView.ActualHeight - 34 - 20; // 工具列、星期標題
        var weekHeight = available > 0 ? available / weeks : 110;
        var lanes = Math.Max(1, (int)((weekHeight - CalendarDateRow - CalendarMoreRow) / CalendarLaneRow));

        var weekGrid = new Grid { Background = Brushes.Transparent };
        weekGrid.Children.Add(new Border { BorderBrush = Brushes2.Get("LineBrush"), BorderThickness = new Thickness(1, 1, 0, 0) });
        Grid.SetRowSpan(weekGrid.Children[0], weeks);
        for (var w = 0; w < weeks; w++)
        {
            weekGrid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });
            var week = Week(first.AddDays(w * 7), dated, lanes);
            Grid.SetRow(week, w);
            weekGrid.Children.Add(week);
        }
        _calendarWeeks = weekGrid;
        month.Children.Add(weekGrid);
        root.Children.Add(month);
        CalendarView.Children.Add(root);
    }

    void ShiftMonth(int months)
    {
        _calendarMonth = _calendarMonth.AddMonths(months);
        RenderCalendar();
    }

    /// <summary>一週：日期格（不是這個月的淡一點、今天標出來）＋任務條＋放不下的「+N」。</summary>
    Grid Week(DateOnly weekStart, List<(TaskSummary Item, DateOnly Start, DateOnly End)> dated, int lanes)
    {
        var grid = new Grid();
        for (var i = 0; i < 7; i++)
            grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(CalendarDateRow) });
        for (var i = 0; i < lanes; i++)
            grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(CalendarLaneRow) });
        grid.RowDefinitions.Add(new RowDefinition { Height = new GridLength(1, GridUnitType.Star) });

        for (var i = 0; i < 7; i++)
        {
            var day = weekStart.AddDays(i);
            var inMonth = day.Month == _calendarMonth.Month;
            var cell = new Border
            {
                BorderBrush = Brushes2.Get("LineBrush"),
                BorderThickness = new Thickness(0, 0, 1, 1),
                Background = inMonth ? Brushes.Transparent : Brushes2.Get("SubtleBrush"),
            };
            Place(grid, cell, i, 0, rowSpan: lanes + 2);
            var highlight = new Border { Background = Brushes2.Get("SelectedBrush"), Visibility = Visibility.Collapsed, Margin = new Thickness(0, 0, 1, 1) };
            Place(grid, highlight, i, 0, rowSpan: lanes + 2);
            _calendarHighlights[day] = highlight;
            var isToday = day == Today;
            var number = new Border
            {
                HorizontalAlignment = HorizontalAlignment.Left,
                VerticalAlignment = VerticalAlignment.Center,
                Margin = new Thickness(4, 0, 0, 0),
                Padding = new Thickness(5, 0, 5, 0),
                CornerRadius = new CornerRadius(9),
                Background = isToday ? Brushes2.Get("AccentBrush") : Brushes.Transparent,
                Child = new TextBlock
                {
                    Text = day.Day == 1 ? $"{day.Month}/{day.Day}" : day.Day.ToString(),
                    FontSize = 12,
                    FontWeight = isToday ? FontWeights.SemiBold : FontWeights.Normal,
                    Foreground = isToday ? Brushes.White : Brushes2.Get(inMonth ? (i >= 5 ? "MutedBrush" : "TextBrush") : "HintBrush"),
                },
            };
            Place(grid, number, i, 0);
        }

        var segments = TaskSchedule.WeekLanes(dated, weekStart);
        var hidden = new List<TaskSummary>[7];
        foreach (var s in segments)
        {
            if (s.Lane >= lanes)
            {
                for (var c = s.Column; c < s.Column + s.Days; c++)
                    (hidden[c] ??= []).Add(s.Item);
                continue;
            }
            var bar = ScheduleBar(s.Item, s.FromBefore, s.ToAfter);
            bar.Margin = new Thickness(s.FromBefore ? 0 : 3, 1, s.ToAfter ? 1 : 4, 1);
            HookCalendarDrag(bar, s.Item);
            HookBarMenu(bar, s.Item);
            Place(grid, bar, s.Column, 1 + s.Lane, columnSpan: s.Days);
        }
        for (var c = 0; c < 7; c++)
        {
            if (hidden[c] is not { Count: > 0 } more)
                continue;
            var link = new TextBlock
            {
                Text = $"+{more.Count}", FontSize = 11, Foreground = Brushes2.Get("AccentBrush"), Cursor = Cursors.Hand,
                Margin = new Thickness(6, 0, 0, 0), VerticalAlignment = VerticalAlignment.Top,
                ToolTip = string.Join("\n", more.Select(t => $"{t.Key} {TaskText.Title(t)}")),
            };
            link.MouseLeftButtonUp += (_, e) =>
            {
                e.Handled = true;
                var menu = new ContextMenu { PlacementTarget = link };
                foreach (var t in more)
                {
                    var item = new MenuItem { Header = $"{t.Key}  {TaskText.Title(t)}" };
                    item.Click += (_, _) => _ = OpenTaskAsync(t.Key);
                    menu.Items.Add(item);
                }
                menu.IsOpen = true;
            };
            Place(grid, link, c, lanes + 1);
        }
        return grid;
    }

    static void Place(Grid grid, UIElement element, int column, int row, int columnSpan = 1, int rowSpan = 1)
    {
        Grid.SetColumn(element, column);
        Grid.SetRow(element, row);
        Grid.SetColumnSpan(element, columnSpan);
        Grid.SetRowSpan(element, rowSpan);
        grid.Children.Add(element);
    }

    /// <summary>右邊：沒有預計日期的任務，拖到日期上就設成那天。</summary>
    FrameworkElement UndatedPanel(List<TaskSummary> undated)
    {
        var list = new StackPanel();
        foreach (var t in undated)
        {
            var bar = ScheduleBar(t);
            bar.Height = CalendarLaneRow;
            bar.Margin = new Thickness(0, 0, 0, 4);
            HookCalendarDrag(bar, t, undatedTask: true);
            HookBarMenu(bar, t);
            list.Children.Add(bar);
        }
        var panel = new DockPanel { Width = 210, Margin = new Thickness(10, 0, 0, 0) };
        var title = new TextBlock
        {
            Text = $"沒有預計日期 {undated.Count}", FontSize = 12, Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 2),
        };
        var hint = new TextBlock
        {
            Text = "拖到日期上就設成那天", FontSize = 11, Foreground = Brushes2.Get("HintBrush"), Margin = new Thickness(0, 0, 0, 6),
        };
        DockPanel.SetDock(title, Dock.Top);
        DockPanel.SetDock(hint, Dock.Top);
        panel.Children.Add(title);
        panel.Children.Add(hint);
        panel.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = list });
        return panel;
    }

    /// <summary>滑鼠下面是哪一天（日曆外回 null）。</summary>
    DateOnly? CalendarDayAt(MouseEventArgs e)
    {
        if (_calendarWeeks is not { ActualWidth: > 0, ActualHeight: > 0 } weeks)
            return null;
        var p = e.GetPosition(weeks);
        if (p.X < 0 || p.Y < 0 || p.X >= weeks.ActualWidth || p.Y >= weeks.ActualHeight)
            return null;
        var col = (int)(p.X / (weeks.ActualWidth / 7));
        var row = (int)(p.Y / (weeks.ActualHeight / _calendarWeekCount));
        return _calendarFirst.AddDays(row * 7 + Math.Min(col, 6));
    }

    void HookCalendarDrag(Border bar, TaskSummary t, bool undatedTask = false)
    {
        bar.MouseLeftButtonDown += (_, e) =>
        {
            if (_calendarWeeks is null)
                return;
            BeginDrag(bar, e, new ScheduleDragState
            {
                Task = t, Mode = Drag.Move, Origin = e.GetPosition(CalendarView), GrabDay = CalendarDayAt(e), Undated = undatedTask,
            });
        };
        bar.MouseMove += (_, e) =>
        {
            if (_drag is not { } d || d.Task != t || !bar.IsMouseCaptured || !PastThreshold(d, e.GetPosition(CalendarView)))
                return;
            d.Started = true;
            bar.Opacity = 0.5;
            var day = CalendarDayAt(e);
            if (day is null)
                d.Preview = (null, null);
            else if (d.Undated)
                d.Preview = (day, day);
            else if (d.GrabDay is { } grab)
            {
                var (start, end) = DatesOf(t);
                d.Preview = TaskSchedule.Apply(start, end, Drag.Move, day.Value.DayNumber - grab.DayNumber);
            }
            ShowCalendarHighlight(d.Preview);
        };
        bar.MouseLeftButtonUp += (_, e) =>
        {
            if (_drag?.Task != t)
                return;
            e.Handled = true;
            ShowCalendarHighlight((null, null));
            EndDrag(bar);
        };
        bar.LostMouseCapture += (_, _) =>
        {
            if (_drag?.Task == t)
            {
                ShowCalendarHighlight((null, null));
                CancelDrag();
            }
        };
    }

    void ShowCalendarHighlight((DateOnly? Start, DateOnly? End) range)
    {
        var span = TaskSchedule.Span(range.Start, range.End);
        foreach (var (day, highlight) in _calendarHighlights)
            highlight.Visibility = span is { } s && day >= s.Start && day <= s.End ? Visibility.Visible : Visibility.Collapsed;
    }

    /// <summary>七欄平分的一列（星期標題）。</summary>
    sealed class UniformGrid7
    {
        public Grid Grid { get; } = new();

        public UniformGrid7()
        {
            for (var i = 0; i < 7; i++)
                Grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        }

        public void Add(UIElement element, int column)
        {
            Grid.SetColumn(element, column);
            Grid.Children.Add(element);
        }
    }
}
