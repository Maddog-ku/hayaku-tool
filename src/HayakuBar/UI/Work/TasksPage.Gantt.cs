using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;
using HayakuBar.Core;
using HayakuBar.Mcp;
using Drag = HayakuBar.Core.TaskSchedule.Drag;

namespace HayakuBar.UI.Work;

/// <summary>
/// 我的任務的甘特圖：左邊任務名稱（照目前的排序），右邊日期軸（週末底色、今天一條線，天數照寬度）。
/// 拖整條平移、拖兩端改預計開始或結束；沒有預計日期的任務在自己那列拉出一段就是設定起訖。
/// 不在畫面範圍的任務在邊上標日期，點了跳過去。
/// </summary>
public partial class TasksPage
{
    const double GanttNameWidth = 240;
    const double GanttRowHeight = 28;
    const double GanttMinDayWidth = 26;
    const double GanttEdge = 7; // 離兩端幾個像素內算拖端點

    DateOnly _ganttStart = TaskSchedule.WeekStart(DateOnly.FromDateTime(DateTime.Today)).AddDays(-7);
    int _ganttDays = 42;
    double _ganttDayWidth = GanttMinDayWidth;
    TextBlock? _ganttStatus;

    void RenderGantt()
    {
        GanttView.Children.Clear();
        var width = GanttView.ActualWidth - GanttNameWidth - 22; // 22：右邊捲軸
        _ganttDays = width > 0 ? Math.Clamp((int)(width / GanttMinDayWidth), 7, 120) : 42;
        _ganttDayWidth = width > 0 ? width / _ganttDays : GanttMinDayWidth;
        var end = _ganttStart.AddDays(_ganttDays - 1);

        var root = new DockPanel();
        var toolbar = ScheduleToolbar($"{TaskSchedule.Md(_ganttStart)} – {TaskSchedule.Md(end)}", "往前一週", "往後一週",
            () => ShiftGantt(-7), () => { _ganttStart = TaskSchedule.WeekStart(Today).AddDays(-7); RenderGantt(); }, () => ShiftGantt(7),
            "拖整條平移，拖兩端改開始／結束；沒有日期的在那列拉出一段");
        _ganttStatus = (TextBlock)((StackPanel)toolbar.Children[0]).Children[^1];
        DockPanel.SetDock(toolbar, Dock.Top);
        root.Children.Add(toolbar);

        var header = GanttHeader();
        DockPanel.SetDock(header, Dock.Top);
        root.Children.Add(header);

        var rows = new StackPanel();
        foreach (var t in _scheduleTasks)
            rows.Children.Add(GanttRow(t));
        root.Children.Add(new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Visible, Content = rows });
        GanttView.Children.Add(root);
    }

    void ShiftGantt(int days)
    {
        _ganttStart = _ganttStart.AddDays(days);
        RenderGantt();
    }

    double GanttX(DateOnly day) => (day.DayNumber - _ganttStart.DayNumber) * _ganttDayWidth;

    /// <summary>日期標題：月份（每月第一天或最左邊）、日期與星期。</summary>
    Grid GanttHeader()
    {
        var grid = new Grid { Height = 36, Margin = new Thickness(0, 0, 22, 0) };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(GanttNameWidth) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(new TextBlock { Text = "任務", FontSize = 12, Foreground = Brushes2.Get("MutedBrush"), VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(4, 0, 0, 4) });
        var canvas = new Canvas { ClipToBounds = true };
        Grid.SetColumn(canvas, 1);
        for (var i = 0; i < _ganttDays; i++)
        {
            var day = _ganttStart.AddDays(i);
            var weekend = day.DayOfWeek is DayOfWeek.Saturday or DayOfWeek.Sunday;
            var isToday = day == Today;
            if (i == 0 || day.Day == 1)
            {
                var monthText = new TextBlock { Text = $"{day.Year}/{day.Month}", FontSize = 11, Foreground = Brushes2.Get("MutedBrush") };
                Canvas.SetLeft(monthText, i * _ganttDayWidth + 2);
                Canvas.SetTop(monthText, 0);
                canvas.Children.Add(monthText);
            }
            var label = new TextBlock
            {
                Text = $"{day.Day}\n{WeekdayNames[((int)day.DayOfWeek + 6) % 7]}",
                FontSize = 10,
                LineHeight = 11,
                TextAlignment = TextAlignment.Center,
                Width = _ganttDayWidth,
                FontWeight = isToday ? FontWeights.Bold : FontWeights.Normal,
                Foreground = isToday ? Brushes2.Get("AccentBrush") : Brushes2.Get(weekend ? "HintBrush" : "MutedBrush"),
            };
            Canvas.SetLeft(label, i * _ganttDayWidth);
            Canvas.SetTop(label, 13);
            canvas.Children.Add(label);
        }
        grid.Children.Add(canvas);
        return new Grid { Children = { new Border { BorderBrush = Brushes2.Get("LineBrush"), BorderThickness = new Thickness(0, 0, 0, 1), Child = grid } } };
    }

    /// <summary>一列：任務名稱＋日期軸上的條。</summary>
    Grid GanttRow(TaskSummary t)
    {
        var row = new Grid { Height = GanttRowHeight };
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(GanttNameWidth) });
        row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        row.Children.Add(new Border { BorderBrush = Brushes2.Get("LineBrush"), BorderThickness = new Thickness(0, 0, 0, 1), Opacity = 0.6 });
        Grid.SetColumnSpan(row.Children[0], 2);

        var name = new TextBlock
        {
            VerticalAlignment = VerticalAlignment.Center, TextTrimming = TextTrimming.CharacterEllipsis, Margin = new Thickness(4, 0, 8, 0),
            Cursor = Cursors.Hand, FontSize = 12, Opacity = IsDone(t) ? 0.55 : 1,
            ToolTip = $"{t.Key} {TaskText.Title(t)}\n{(t.Path.Length > 0 ? $"{t.Project} › {t.Path}" : t.Project)}・{t.Status}",
        };
        name.Inlines.Add(new System.Windows.Documents.Run(t.Key + " ") { Foreground = Brushes2.Get("MutedBrush") });
        name.Inlines.Add(new System.Windows.Documents.Run(TaskText.Title(t)));
        name.MouseLeftButtonUp += (_, _) => _ = OpenTaskAsync(t.Key);
        HookBarMenu(name, t);
        row.Children.Add(name);

        var canvas = new Canvas { ClipToBounds = true, Background = Brushes.Transparent };
        Grid.SetColumn(canvas, 1);
        for (var i = 0; i < _ganttDays; i++)
        {
            var day = _ganttStart.AddDays(i);
            if (day.DayOfWeek is DayOfWeek.Saturday or DayOfWeek.Sunday)
                canvas.Children.Add(At(new Rectangle { Width = _ganttDayWidth, Height = GanttRowHeight, Fill = Brushes2.Get("SubtleBrush") }, i * _ganttDayWidth, 0));
        }
        var todayIndex = Today.DayNumber - _ganttStart.DayNumber;
        if (todayIndex >= 0 && todayIndex < _ganttDays)
            canvas.Children.Add(At(new Rectangle { Width = 2, Height = GanttRowHeight, Fill = Brushes2.Get("AccentBrush"), Opacity = 0.7 },
                (todayIndex + 0.5) * _ganttDayWidth - 1, 0));
        row.Children.Add(canvas);

        var (start, end) = DatesOf(t);
        if (TaskSchedule.Span(start, end) is not { } span)
        {
            HookGanttDraw(canvas, t);
            canvas.ToolTip = "沒有預計日期：在這列拉出一段就是預計開始到結束";
            canvas.Cursor = Cursors.Cross;
            return row;
        }
        var last = _ganttStart.AddDays(_ganttDays - 1);
        if (span.End < _ganttStart || span.Start > last)
        {
            // 不在畫面範圍：在邊上標日期，點了跳到那裡
            var before = span.End < _ganttStart;
            var jump = new TextBlock
            {
                Text = before ? $"‹ {TaskSchedule.RangeText(start, end)}" : $"{TaskSchedule.RangeText(start, end)} ›",
                FontSize = 11, Foreground = Brushes2.Get(IsOverdue(t) ? "ErrorBrush" : "MutedBrush"), Cursor = Cursors.Hand,
                ToolTip = "跳到這個任務的日期",
            };
            jump.MouseLeftButtonUp += (_, e) =>
            {
                e.Handled = true;
                _ganttStart = TaskSchedule.WeekStart(span.Start).AddDays(-7);
                RenderGantt();
            };
            jump.Loaded += (_, _) => Canvas.SetLeft(jump, before ? 4 : Math.Max(4, canvas.ActualWidth - jump.ActualWidth - 4));
            canvas.Children.Add(At(jump, 4, 6));
            return row;
        }
        var bar = ScheduleBar(t, span.Start < _ganttStart, span.End > last, showKey: false);
        bar.Height = GanttRowHeight - 8;
        LayoutGanttBar(bar, span.Start, span.End);
        HookGanttDrag(bar, canvas, t);
        HookBarMenu(bar, t);
        canvas.Children.Add(bar);
        return row;
    }

    static UIElement At(UIElement element, double left, double top)
    {
        Canvas.SetLeft(element, left);
        Canvas.SetTop(element, top);
        return element;
    }

    void LayoutGanttBar(FrameworkElement bar, DateOnly start, DateOnly end)
    {
        var left = GanttX(start);
        Canvas.SetLeft(bar, left + 1);
        Canvas.SetTop(bar, 4);
        bar.Width = Math.Max(_ganttDayWidth - 2, GanttX(end.AddDays(1)) - left - 2);
    }

    Drag GanttDragMode(Border bar, MouseEventArgs e)
    {
        var x = e.GetPosition(bar).X;
        return x <= GanttEdge ? Drag.Start : x >= bar.ActualWidth - GanttEdge ? Drag.End : Drag.Move;
    }

    void HookGanttDrag(Border bar, Canvas canvas, TaskSummary t)
    {
        bar.MouseLeftButtonDown += (_, e) =>
            BeginDrag(bar, e, new ScheduleDragState { Task = t, Mode = GanttDragMode(bar, e), Origin = e.GetPosition(canvas) });
        bar.MouseMove += (_, e) =>
        {
            if (_drag is not { } d || d.Task != t || !bar.IsMouseCaptured)
            {
                bar.Cursor = GanttDragMode(bar, e) == Drag.Move ? Cursors.Hand : Cursors.SizeWE; // 移到兩端提示可以拉
                return;
            }
            var p = e.GetPosition(canvas);
            if (!PastThreshold(d, p))
                return;
            d.Started = true;
            var (start, end) = DatesOf(t);
            d.Preview = TaskSchedule.Apply(start, end, d.Mode, (int)Math.Round((p.X - d.Origin.X) / _ganttDayWidth));
            if (TaskSchedule.Span(d.Preview.Start, d.Preview.End) is { } s)
            {
                LayoutGanttBar(bar, s.Start, s.End);
                ShowGanttStatus(t, d.Preview);
            }
        };
        bar.MouseLeftButtonUp += (_, e) =>
        {
            if (_drag?.Task != t)
                return;
            e.Handled = true;
            EndDrag(bar);
        };
        bar.LostMouseCapture += (_, _) =>
        {
            if (_drag?.Task == t)
                CancelDrag();
        };
    }

    /// <summary>沒有預計日期的任務：在自己那列按住拉出一段（只點一下就是那一天）。</summary>
    void HookGanttDraw(Canvas canvas, TaskSummary t)
    {
        Border? preview = null;
        DateOnly DayAt(MouseEventArgs e) =>
            _ganttStart.AddDays(Math.Clamp((int)(e.GetPosition(canvas).X / _ganttDayWidth), 0, _ganttDays - 1));
        canvas.MouseLeftButtonDown += (_, e) =>
        {
            var day = DayAt(e);
            BeginDrag(canvas, e, new ScheduleDragState { Task = t, Mode = Drag.Move, Origin = e.GetPosition(canvas), GrabDay = day, Undated = true });
            _drag!.Started = true; // 點一下也算設定那一天，不是打開任務
            _drag.Preview = (day, day);
            preview = ScheduleBar(t, showKey: false);
            preview.Height = GanttRowHeight - 8;
            preview.Opacity = 0.6;
            LayoutGanttBar(preview, day, day);
            canvas.Children.Add(preview);
            ShowGanttStatus(t, _drag.Preview);
        };
        canvas.MouseMove += (_, e) =>
        {
            if (_drag is not { } d || d.Task != t || !canvas.IsMouseCaptured || d.GrabDay is not { } grab || preview is null)
                return;
            var (s, en) = TaskSchedule.Place(grab, DayAt(e));
            d.Preview = (s, en);
            LayoutGanttBar(preview, s, en);
            ShowGanttStatus(t, d.Preview);
        };
        canvas.MouseLeftButtonUp += (_, e) =>
        {
            if (_drag?.Task != t)
                return;
            e.Handled = true;
            EndDrag(canvas);
        };
        canvas.LostMouseCapture += (_, _) =>
        {
            if (_drag?.Task == t)
                CancelDrag();
        };
    }

    void ShowGanttStatus(TaskSummary t, (DateOnly? Start, DateOnly? End) range)
    {
        if (_ganttStatus is not null)
            _ganttStatus.Text = $"{t.Key}：{TaskSchedule.RangeText(range.Start, range.End)}";
    }
}
