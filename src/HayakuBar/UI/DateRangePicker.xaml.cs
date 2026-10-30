using System.Windows;
using System.Windows.Controls;
using System.Windows.Controls.Primitives;
using System.Windows.Input;
using System.Windows.Media;
using System.Windows.Shapes;

namespace HayakuBar.UI;

/// <summary>
/// 預計開始～結束：點一下跳出並排的兩個月日曆（跟 Hayaku 網頁一樣），點第一下是開始、第二下是結束，按「套用日期」才生效。
/// 只選一天就套用＝開始和結束同一天；「清除」把兩個日期都清掉。
/// Single＝只選一天：點一下日期就直接套用。
/// </summary>
public partial class DateRangePicker : UserControl
{
    const double CellWidth = 38;
    const double CellHeight = 34;
    static readonly string[] WeekDays = ["一", "二", "三", "四", "五", "六", "日"];

    DateOnly? _start, _end;          // 已套用的
    DateOnly? _pickStart, _pickEnd;  // 日曆上正在選的
    DateOnly? _hover;
    DateOnly _month;                 // 左邊那個月的 1 號
    bool _single;

    public DateRangePicker()
    {
        InitializeComponent();
        UpdateText();
    }

    /// <summary>按了「套用日期」或「清除」（值有沒有變由呼叫端比較）。</summary>
    public event Action? Applied;

    public DateOnly? Start => _start;

    /// <summary>只選一天（例如請半天）：欄位只顯示一個日期，點一下日期就套用。</summary>
    public bool Single
    {
        get => _single;
        set
        {
            _single = value;
            if (value)
                _end = _start;
            UpdateText();
        }
    }

    public DateOnly? End => _end;

    public void SetRange(DateOnly? start, DateOnly? end)
    {
        (_start, _end) = (start, end);
        UpdateText();
    }

    static DateOnly Today => DateOnly.FromDateTime(DateTime.Today);

    void UpdateText()
    {
        // 只填了一邊時，另一邊顯示「未指定」（不是還沒選，是刻意空著）
        var oneSided = !_single && (_start is null) != (_end is null);
        Show(StartText, _start, _single ? "選擇日期" : oneSided ? "未指定" : "開始日期");
        Show(EndText, _end, oneSided ? "未指定" : "結束日期");
        Tilde.Visibility = EndText.Visibility = _single ? Visibility.Collapsed : Visibility.Visible;
    }

    static void Show(TextBlock text, DateOnly? date, string hint)
    {
        text.Text = date is { } d ? d.ToString("yyyy-MM-dd") : hint;
        text.SetResourceReference(TextBlock.ForegroundProperty, date is null ? "HintBrush" : "TextBrush");
    }

    // ───────────── 開關 ─────────────

    void Field_Click(object sender, MouseButtonEventArgs e) => Open();

    void Field_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key is Key.Enter or Key.Space or Key.Down)
        {
            e.Handled = true;
            Open();
        }
    }

    void Open()
    {
        if (CalendarPopup.IsOpen)
            return;
        (_pickStart, _pickEnd, _hover) = (_start, _end, null);
        var first = _start ?? Today;
        _month = new DateOnly(first.Year, first.Month, 1);
        Render();
        Field.SetResourceReference(Border.BorderBrushProperty, "AccentBrush");
        CalendarPopup.IsOpen = true;
    }

    void Popup_Closed(object? sender, EventArgs e) => Field.SetResourceReference(Border.BorderBrushProperty, "LineBrush");

    void Cancel_Click(object sender, RoutedEventArgs e) => CalendarPopup.IsOpen = false;

    void Apply_Click(object sender, RoutedEventArgs e)
    {
        _start = _pickStart;
        _end = _pickEnd ?? _pickStart; // 只點一天：開始和結束同一天
        CalendarPopup.IsOpen = false;
        UpdateText();
        Applied?.Invoke();
    }

    void Clear_Click(object sender, RoutedEventArgs e)
    {
        (_start, _end) = (null, null);
        CalendarPopup.IsOpen = false;
        UpdateText();
        Applied?.Invoke();
    }

    void PrevYear_Click(object sender, RoutedEventArgs e) => Move(-12);
    void PrevMonth_Click(object sender, RoutedEventArgs e) => Move(-1);
    void NextMonth_Click(object sender, RoutedEventArgs e) => Move(1);
    void NextYear_Click(object sender, RoutedEventArgs e) => Move(12);

    void Move(int months)
    {
        _month = _month.AddMonths(months);
        Render();
    }

    // ───────────── 選日期 ─────────────

    /// <summary>點一下：還沒選開始（或已選好一段）→ 重新從這天開始；已選開始 → 這天當結束（比開始早就改當開始）。</summary>
    void Pick(DateOnly day)
    {
        if (_single)
        {
            (_start, _end) = (day, day);
            CalendarPopup.IsOpen = false;
            UpdateText();
            Applied?.Invoke();
            return;
        }
        if (_pickStart is null || _pickEnd is not null || day < _pickStart)
            (_pickStart, _pickEnd) = (day, null);
        else
            _pickEnd = day;
        Render();
    }

    /// <summary>選了開始、還沒選結束時，滑鼠移到哪天就先預覽那一段。</summary>
    (DateOnly? From, DateOnly? To) Range()
    {
        if (!_single && _pickStart is { } s && _pickEnd is null && _hover is { } h && h > s)
            return (s, h);
        return (_pickStart, _pickEnd ?? _pickStart);
    }

    void Render()
    {
        var right = _month.AddMonths(1);
        LeftTitle.Text = $"{_month.Year}年  {_month.Month}月";
        RightTitle.Text = $"{right.Year}年  {right.Month}月";
        RenderMonth(LeftMonth, _month);
        RenderMonth(RightMonth, right);
        PickText.Text = _single ? "點一下日期就選好"
            : _pickStart is null ? "點一下選開始日期"
            : _pickEnd is null ? $"{_pickStart:M/d} ～ 再點結束日期（只選這天就直接套用）"
            : $"{_pickStart:M/d} ～ {_pickEnd:M/d}，共 {_pickEnd.Value.DayNumber - _pickStart.Value.DayNumber + 1} 天";
    }

    void RenderMonth(UniformGrid grid, DateOnly month)
    {
        grid.Children.Clear();
        foreach (var w in WeekDays)
        {
            var label = new TextBlock
            {
                Text = w,
                Width = CellWidth,
                Height = CellHeight - 6,
                TextAlignment = TextAlignment.Center,
                FontSize = 12,
            };
            label.SetResourceReference(TextBlock.ForegroundProperty, "MutedBrush");
            grid.Children.Add(label);
        }
        var offset = ((int)month.DayOfWeek + 6) % 7; // 週一開始
        var first = month.AddDays(-offset);
        var (from, to) = Range();
        for (var i = 0; i < 42; i++)
            grid.Children.Add(DayCell(first.AddDays(i), month, from, to));
    }

    FrameworkElement DayCell(DateOnly day, DateOnly month, DateOnly? from, DateOnly? to)
    {
        var inMonth = day.Month == month.Month;
        var isEnd = day == from || day == to;
        var inRange = from is { } f && to is { } t && day >= f && day <= t;

        var cell = new Grid { Width = CellWidth, Height = CellHeight, Background = Brushes.Transparent, Cursor = Cursors.Hand };
        if (inRange && from != to)
        {
            // 區間的底色：頭尾只畫一半，連成一條
            var band = new Border
            {
                Margin = new Thickness(day == from ? CellWidth / 2 : 0, 3, day == to ? CellWidth / 2 : 0, 3),
            };
            band.SetResourceReference(Border.BackgroundProperty, "SelectedBrush");
            cell.Children.Add(band);
        }
        if (isEnd)
        {
            var dot = new Border { Width = 30, Height = 28, CornerRadius = new CornerRadius(7) };
            dot.SetResourceReference(Border.BackgroundProperty, "TextBrush");
            cell.Children.Add(dot);
        }
        else if (day == Today && inMonth) // 另一個月補位的今天只畫底線
        {
            var ring = new Border { Width = 30, Height = 28, CornerRadius = new CornerRadius(7), BorderThickness = new Thickness(1) };
            ring.SetResourceReference(Border.BorderBrushProperty, "TextBrush");
            cell.Children.Add(ring);
        }
        var text = new TextBlock
        {
            Text = day.Day.ToString(),
            HorizontalAlignment = HorizontalAlignment.Center,
            VerticalAlignment = VerticalAlignment.Center,
            FontWeight = isEnd ? FontWeights.SemiBold : FontWeights.Normal,
            Opacity = inMonth || isEnd ? 1 : 0.4,
        };
        text.SetResourceReference(TextBlock.ForegroundProperty, isEnd ? "BgBrush" : "TextBrush");
        cell.Children.Add(text);
        if (day == Today)
        {
            // 今天：下面一條橘色短線
            var mark = new Rectangle { Width = 12, Height = 2, VerticalAlignment = VerticalAlignment.Bottom, Margin = new Thickness(0, 0, 0, 5) };
            mark.SetResourceReference(Shape.FillProperty, "AccentBrush");
            cell.Children.Add(mark);
        }

        cell.MouseLeftButtonUp += (_, _) => Pick(day);
        cell.MouseEnter += (_, _) =>
        {
            if (_pickStart is null || _pickEnd is not null || _hover == day)
                return;
            _hover = day;
            Render();
        };
        return cell;
    }
}
