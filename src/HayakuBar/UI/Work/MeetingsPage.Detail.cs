using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 會議詳細（右欄）：時間、地點、主辦、重複規則、參加者與各自記到的工時；可以修改（名稱、時間、地點、參加者、會議紀錄）。
/// 會議紀錄、檔案、白板放在可展開的區塊，展開才讀，平常不多打請求。
/// </summary>
public partial class MeetingsPage
{
    string? _detailKey;
    int _detailVersion;
    MeetingDetail? _detail;

    async void Card_Click(object sender, MouseButtonEventArgs e)
    {
        if ((sender as FrameworkElement)?.Tag is not MeetingListVm vm)
            return;
        await OpenDetailAsync(vm.Item.Key);
    }

    async Task OpenDetailAsync(string key)
    {
        var version = ++_detailVersion;
        _detailKey = key;
        foreach (var row in MeetingList.ItemsSource as IEnumerable<MeetingListVm> ?? [])
            row.Selected = row.Item.Key == key;
        DetailColumn.Width = new GridLength(1.2, GridUnitType.Star);
        DetailCard.Visibility = Visibility.Visible;
        DetailHost.Children.Clear();
        DetailHost.Children.Add(Muted("讀取中…"));
        try
        {
            var detail = await _engine.Api.GetMeetingAsync(key, CancellationToken.None);
            if (version != _detailVersion)
                return;
            _detail = detail;
            RenderDetail(detail);
        }
        catch (Exception ex)
        {
            Log.Error($"讀取會議 {key} 失敗", ex);
            if (version != _detailVersion)
                return;
            DetailHost.Children.Clear();
            DetailHost.Children.Add(Header(key));
            DetailHost.Children.Add(new TextBlock { Text = $"讀不到這場會議：{ex.Message}", TextWrapping = TextWrapping.Wrap, Foreground = Brushes2.Get("ErrorBrush") });
        }
    }

    void CloseDetail()
    {
        _detailVersion++;
        _detailKey = null;
        _detail = null;
        DetailCard.Visibility = Visibility.Collapsed;
        DetailColumn.Width = new GridLength(0);
        foreach (var row in MeetingList.ItemsSource as IEnumerable<MeetingListVm> ?? [])
            row.Selected = false;
    }

    void RenderDetail(MeetingDetail d)
    {
        DetailHost.Children.Clear();
        DetailScroll.ScrollToTop();
        DetailHost.Children.Add(Header(d.Title));

        var day = DateOnly.FromDateTime(d.StartAt.LocalDateTime);
        var lines = new List<(string Label, string Value)>
        {
            ("時間", $"{Inputs.ShortDate(day)}　{d.StartAt.ToLocalTime():HH:mm}–{d.EndAt.ToLocalTime():HH:mm}"),
            ("專案", _engine.ProjectOf(d.Project)?.Name ?? d.Project),
        };
        if (!string.IsNullOrWhiteSpace(d.Location))
            lines.Add(("地點", d.Location));
        if (d.Organizer is { } organizer)
            lines.Add(("主辦", organizer));
        if (d.Recurrence is { } recurrence)
            lines.Add(("重複", recurrence));
        if (d.MyState is { } me)
            lines.Add(("我", me));
        var info = new Grid { Margin = new Thickness(0, 6, 0, 0) };
        info.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(44) });
        info.ColumnDefinitions.Add(new ColumnDefinition());
        foreach (var (label, value) in lines)
        {
            info.RowDefinitions.Add(new RowDefinition { Height = GridLength.Auto });
            var l = new TextBlock { Text = label, Foreground = Brushes2.Get("MutedBrush"), Margin = new Thickness(0, 0, 0, 4) };
            var v = new TextBlock { Text = value, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 4) };
            Grid.SetRow(l, info.RowDefinitions.Count - 1);
            Grid.SetRow(v, info.RowDefinitions.Count - 1);
            Grid.SetColumn(v, 1);
            info.Children.Add(l);
            info.Children.Add(v);
        }
        DetailHost.Children.Add(info);

        var status = new TextBlock { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 4, 0, 4), Visibility = Visibility.Collapsed };
        var edit = Button(d.CanEdit ? "✎ 修改" : "✎ 修改會議紀錄", "FlatButton");
        edit.ToolTip = d.CanEdit ? "名稱、時間、地點、參加者、會議紀錄" : "時間、參加者只有主辦或專案管理員能改；會議紀錄參加者都能改";
        var web = Button("在網頁開啟 ↗", "FlatButton");
        web.Click += (_, _) => Web.Open(_engine.MeetingUrl(d.Id));
        DetailHost.Children.Add(new WrapPanel { Margin = new Thickness(0, 8, 0, 0), Children = { edit, web } });
        DetailHost.Children.Add(status);
        var form = new StackPanel { Visibility = Visibility.Collapsed };
        DetailHost.Children.Add(form);
        edit.Click += async (_, _) =>
        {
            if (form.Visibility == Visibility.Visible)
                return;
            form.Visibility = Visibility.Visible;
            await BuildEditFormAsync(d, form, status);
        };

        DetailHost.Children.Add(Group($"參加者（{d.Attendees.Count}）"));
        foreach (var a in d.Attendees)
        {
            var row = new TextBlock { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 3) };
            row.Inlines.Add(new System.Windows.Documents.Run(a.Name));
            if (a.State.Length > 0)
                row.Inlines.Add(new System.Windows.Documents.Run($"　{a.State}")
                {
                    Foreground = Brushes2.Get(a.State == "會議中" ? "AccentBrush" : "MutedBrush"), FontSize = 12,
                });
            if (a.LoggedMinutes > 0)
                row.Inlines.Add(new System.Windows.Documents.Run($"　{WorkMath.HoursMinutes(a.LoggedMinutes)}")
                    { Foreground = Brushes2.Get("MutedBrush"), FontSize = 12 });
            DetailHost.Children.Add(row);
        }
        if (d.Attendees.Count == 0)
            DetailHost.Children.Add(Muted("沒有參加者"));

        DetailHost.Children.Add(LazySection("會議紀錄", d.Key, LoadNotesAsync));
        DetailHost.Children.Add(LazySection("檔案", d.Key, LoadFilesAsync));
        DetailHost.Children.Add(LazySection("白板", d.Key, LoadWhiteboardsAsync));
    }

    // ───────────── 修改 ─────────────

    async Task BuildEditFormAsync(MeetingDetail d, StackPanel form, TextBlock status)
    {
        form.Children.Clear();
        form.Children.Add(Group(d.CanEdit ? "修改會議" : "修改會議紀錄"));
        var title = new TextBox { Text = d.Title, Style = (Style)FindResource("HintBox"), Tag = "會議名稱" };
        var start = d.StartAt.ToLocalTime();
        var end = d.EndAt.ToLocalTime();
        var date = new TextBox { Text = start.ToString("M/d"), Style = (Style)FindResource("HintBox"), Tag = "今天、明天、11/5" };
        var from = new TextBox { Text = start.ToString("HH:mm"), Style = (Style)FindResource("HintBox"), Tag = "14:00" };
        var to = new TextBox { Text = end.ToString("HH:mm"), Style = (Style)FindResource("HintBox"), Tag = "15:00" };
        var location = new TextBox { Text = d.Location ?? "", Style = (Style)FindResource("HintBox"), Tag = "會議室、Teams 連結…（空白＝清掉）" };
        var note = new TextBox
        {
            Text = d.Note ?? "", AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = 80, MaxHeight = 260,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto, VerticalContentAlignment = VerticalAlignment.Top,
            Style = (Style)FindResource("HintBox"), Tag = "會議紀錄（Markdown）",
        };
        var attendees = new WrapPanel();
        var thisOnly = new RadioButton { Content = "只改這一場", IsChecked = true, Margin = new Thickness(0, 0, 14, 0) };
        var following = new RadioButton { Content = "這一場和之後的" };

        if (d.CanEdit)
        {
            form.Children.Add(Field("名稱", title));
            var time = new Grid { Margin = new Thickness(0, 0, 0, 8) };
            foreach (var w in new[] { 1.3, 0.12, 1.0, 0.12, 1.0 })
                time.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(w, GridUnitType.Star) });
            AddCell(time, Field("日期", date, 0), 0);
            AddCell(time, Field("開始", from, 0), 2);
            AddCell(time, Field("結束", to, 0), 4);
            form.Children.Add(time);
            form.Children.Add(Field("地點或連結", location));
            form.Children.Add(Label("參加者"));
            form.Children.Add(attendees);
            attendees.Children.Add(Muted("讀取專案成員…"));
            if (d.Recurring)
            {
                form.Children.Add(Label("這是重複會議，要改"));
                form.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 8), Children = { thisOnly, following } });
            }
        }
        form.Children.Add(Field("會議紀錄（整份取代）", note));
        var save = Button("儲存", "PrimaryButton");
        var cancel = Button("取消", "FlatButton");
        cancel.Click += (_, _) =>
        {
            form.Visibility = Visibility.Collapsed;
            form.Children.Clear();
        };
        form.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { save, cancel } });

        var checks = new List<(string Username, CheckBox Box)>();
        save.Click += async (_, _) =>
        {
            var fields = new Dictionary<string, object?>();
            if (d.CanEdit)
            {
                if (title.Text.Trim() is { Length: > 0 } t && t != d.Title)
                    fields["title"] = t;
                else if (title.Text.Trim().Length == 0)
                {
                    Inputs.Show(status, "名稱不能空白");
                    return;
                }
                if (Inputs.ParseDate(date.Text) is not { } day)
                {
                    Inputs.Show(status, "日期看不懂，請用 今天、明天、11/5 這類寫法");
                    return;
                }
                if (Inputs.ParseTime(from.Text) is not { } s || Inputs.ParseTime(to.Text) is not { } e)
                {
                    Inputs.Show(status, "開始、結束請用 HH:mm，例 14:00");
                    return;
                }
                if (e <= s)
                {
                    Inputs.Show(status, "結束要晚於開始（會議要在同一天）");
                    return;
                }
                var newStart = Inputs.Local(day, s);
                var newEnd = Inputs.Local(day, e);
                if (newStart != d.StartAt || newEnd != d.EndAt)
                {
                    fields["startAt"] = Inputs.Iso(newStart);
                    fields["endAt"] = Inputs.Iso(newEnd);
                }
                var loc = location.Text.Trim();
                if (loc != (d.Location ?? ""))
                    fields["location"] = loc.Length > 0 ? loc : null;
                var (add, remove) = AttendeeChanges(d.Attendees.Select(a => a.Username),
                    checks.Where(c => c.Box.IsChecked == true).Select(c => c.Username), checks.Count > 0);
                if (add.Count > 0)
                    fields["addAttendees"] = add;
                if (remove.Count > 0)
                    fields["removeAttendees"] = remove;
            }
            if (note.Text != (d.Note ?? ""))
                fields["note"] = note.Text.Length > 0 ? note.Text : null;
            if (fields.Count == 0)
            {
                Inputs.Show(status, "沒有修改");
                return;
            }
            if (d.Recurring && d.CanEdit)
                fields["scope"] = following.IsChecked == true ? "following" : "this";
            if (!await Inputs.Run(save, status, () => _engine.Api.UpdateMeetingAsync(d.Key, fields, CancellationToken.None)))
                return;
            Inputs.Show(ListStatus, $"已修改會議「{(fields.TryGetValue("title", out var nt) ? nt : d.Title)}」", error: false);
            await _engine.AfterExternalChangeAsync();
            await LoadAsync();
            await OpenDetailAsync(d.Key);
        };

        if (!d.CanEdit)
            return;
        try
        {
            var project = await _engine.GetProjectAsync(d.Project);
            var current = d.Attendees.Select(a => a.Username).ToHashSet(StringComparer.OrdinalIgnoreCase);
            attendees.Children.Clear();
            // 專案成員＋目前的參加者（可能已經不在專案裡）
            var people = project.Members.Select(m => (m.Username, m.DisplayName))
                .Concat(d.Attendees.Where(a => a.Username.Length > 0).Select(a => (a.Username, DisplayName: a.Name)))
                .DistinctBy(p => p.Username, StringComparer.OrdinalIgnoreCase)
                .ToList();
            foreach (var (username, name) in people)
            {
                var box = new CheckBox { Content = name, IsChecked = current.Contains(username), Margin = new Thickness(0, 0, 14, 6) };
                attendees.Children.Add(box);
                checks.Add((username, box));
            }
        }
        catch (Exception ex)
        {
            Log.Error($"讀取 {d.Project} 的成員失敗", ex);
            attendees.Children.Clear();
            attendees.Children.Add(new TextBlock { Text = $"讀不到專案成員（參加者先不改）：{ex.Message}", TextWrapping = TextWrapping.Wrap, Foreground = Brushes2.Get("ErrorBrush") });
        }
    }

    /// <summary>參加者名單的差異：要加、要拿掉的帳號；沒讀到成員清單（loaded=false）時不改參加者。</summary>
    internal static (List<string> Add, List<string> Remove) AttendeeChanges(IEnumerable<string> current, IEnumerable<string> selected, bool loaded)
    {
        if (!loaded)
            return ([], []);
        var before = current.Where(u => u.Length > 0).ToHashSet(StringComparer.OrdinalIgnoreCase);
        var after = selected.Where(u => u.Length > 0).ToHashSet(StringComparer.OrdinalIgnoreCase);
        return (after.Where(u => !before.Contains(u)).Order().ToList(), before.Where(u => !after.Contains(u)).Order().ToList());
    }

    // ───────────── 會議紀錄、檔案、白板（展開才讀） ─────────────

    async Task LoadNotesAsync(string key, StackPanel body)
    {
        var notes = await _engine.Api.MeetingNotesAsync(key, CancellationToken.None);
        body.Children.Clear();
        if (notes.Count == 0)
            body.Children.Add(Muted("還沒有會議紀錄"));
        foreach (var n in notes)
        {
            var head = n.UpdatedAt is { } at ? $"{n.Title}　{Inputs.Ago(at)}更新" : n.Title;
            body.Children.Add(new TextBlock { Text = head, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 4, 0, 4) });
            body.Children.Add(n.Markdown.Trim().Length == 0
                ? Muted("（空白）")
                : new Border
                {
                    Background = Brushes2.Get("SubtleBrush"), CornerRadius = new CornerRadius(6), Padding = new Thickness(10, 8, 10, 8), Margin = new Thickness(0, 0, 0, 8),
                    Child = new TextBox
                    {
                        Text = n.Markdown.Trim(), Style = (Style)FindResource("PlainText"), MaxHeight = 360,
                        VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
                    },
                });
        }
    }

    async Task LoadFilesAsync(string key, StackPanel body)
    {
        var files = await _engine.Api.MeetingFilesAsync(key, CancellationToken.None);
        body.Children.Clear();
        if (files.Count == 0)
            body.Children.Add(Muted("沒有檔案"));
        var id = _detail?.Id ?? key;
        foreach (var f in files)
        {
            var meta = string.Join(" · ", new[] { FileSize(f.SizeBytes), f.SourceText, f.UploadedBy, f.UploadedAt is { } at ? Inputs.Ago(at) : null }
                .Where(s => !string.IsNullOrWhiteSpace(s)));
            var row = new StackPanel { Margin = new Thickness(0, 0, 0, 6), Cursor = Cursors.Hand, Background = System.Windows.Media.Brushes.Transparent,
                ToolTip = "在網頁開啟這場會議，從「檔案」下載" };
            row.Children.Add(new TextBlock { Text = f.FileName, Foreground = Brushes2.Get("AccentBrush"), TextTrimming = TextTrimming.CharacterEllipsis });
            row.Children.Add(new TextBlock { Text = meta, Style = (Style)FindResource("CardDesc") });
            row.MouseLeftButtonUp += (_, _) => Web.Open(_engine.MeetingUrl(id));
            body.Children.Add(row);
        }
    }

    async Task LoadWhiteboardsAsync(string key, StackPanel body)
    {
        var boards = await _engine.Api.MeetingWhiteboardsAsync(key, CancellationToken.None);
        body.Children.Clear();
        if (boards.Count == 0)
            body.Children.Add(Muted("沒有白板"));
        foreach (var b in boards)
        {
            var meta = new List<string>();
            if (b.ElementCount > 0)
                meta.Add($"{b.ElementCount} 個元素");
            if (b.ImageCount > 0)
                meta.Add($"{b.ImageCount} 張圖片");
            if (b.Snapshots > 0)
                meta.Add($"{b.Snapshots} 個快照");
            if (b.UpdatedAt is { } at)
                meta.Add($"{Inputs.Ago(at)}更新");
            body.Children.Add(new TextBlock { Text = b.Title, FontWeight = FontWeights.SemiBold, Margin = new Thickness(0, 4, 0, 2) });
            body.Children.Add(new TextBlock { Text = string.Join(" · ", meta), Style = (Style)FindResource("CardDesc") });
            body.Children.Add(b.Text.Count == 0
                ? Muted("（沒有文字，只有手繪或圖形；要看圖請在網頁開啟）")
                : new TextBox { Text = string.Join("\n", b.Text), Style = (Style)FindResource("PlainText"), Margin = new Thickness(0, 4, 0, 8), MaxHeight = 240,
                    VerticalScrollBarVisibility = ScrollBarVisibility.Auto });
        }
    }

    /// <summary>可展開的區塊：第一次展開才讀，讀失敗下次展開再試。</summary>
    Expander LazySection(string title, string key, Func<string, StackPanel, Task> load)
    {
        var body = new StackPanel { Margin = new Thickness(20, 6, 0, 0) };
        var expander = new Expander
        {
            Header = new TextBlock { Text = title, FontWeight = FontWeights.SemiBold },
            Content = body,
            Margin = new Thickness(0, 12, 0, 0),
            Foreground = Brushes2.Get("TextBrush"),
        };
        var loaded = false;
        expander.Expanded += async (_, _) =>
        {
            if (loaded)
                return;
            loaded = true;
            body.Children.Clear();
            body.Children.Add(Muted("讀取中…"));
            try
            {
                await load(key, body);
            }
            catch (Exception ex)
            {
                Log.Error($"讀取會議 {key} 的{title}失敗", ex);
                loaded = false;
                body.Children.Clear();
                body.Children.Add(new TextBlock { Text = $"讀取失敗：{ex.Message}", TextWrapping = TextWrapping.Wrap, Foreground = Brushes2.Get("ErrorBrush") });
            }
        };
        return expander;
    }

    // ───────────── 小元件 ─────────────

    FrameworkElement Header(string title)
    {
        var close = new Button { Content = "✕", Style = (Style)FindResource("IconButton"), ToolTip = "關閉詳細", VerticalAlignment = VerticalAlignment.Top };
        close.Click += (_, _) => CloseDetail();
        var panel = new DockPanel();
        DockPanel.SetDock(close, Dock.Right);
        panel.Children.Add(close);
        panel.Children.Add(new TextBlock { Text = title, FontSize = 16, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
        return panel;
    }

    Button Button(string text, string style) =>
        new() { Content = text, Style = (Style)FindResource(style), Padding = new Thickness(12, 4, 12, 4), Margin = new Thickness(0, 0, 6, 6) };

    TextBlock Group(string text) => new() { Text = text, Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(0, 14, 0, 6) };

    TextBlock Label(string text) => new() { Text = text, Style = (Style)FindResource("FieldLabel") };

    FrameworkElement Field(string label, Control input, double bottom = 8) =>
        new StackPanel { Margin = new Thickness(0, 0, 0, bottom), Children = { Label(label), input } };

    static void AddCell(Grid grid, UIElement element, int column)
    {
        Grid.SetColumn(element, column);
        grid.Children.Add(element);
    }

    static TextBlock Muted(string text) =>
        new() { Text = text, Foreground = Brushes2.Get("MutedBrush"), TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 2, 0, 4) };

    /// <summary>位元組 → 「20 KB」「1.4 MB」；不知道大小回空字串。</summary>
    internal static string FileSize(long bytes) => bytes switch
    {
        <= 0 => "",
        < 1024 => $"{bytes} B",
        < 1024 * 1024 => $"{Math.Round(bytes / 1024.0)} KB",
        _ => $"{bytes / 1024.0 / 1024.0:0.#} MB",
    };
}
