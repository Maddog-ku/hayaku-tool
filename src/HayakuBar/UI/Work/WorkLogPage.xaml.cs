using System.Text;
using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>某天的工作日誌：照計時紀錄整理的任務與時數，加上自己在網頁寫的日誌。</summary>
public partial class WorkLogPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    DateOnly _day = DateOnly.FromDateTime(DateTime.Today);
    WorkLog? _log;
    ClaudeDigest? _digest;
    List<TimeEntry> _entries = [];
    int _version;

    public WorkLogPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
    }

    /// <summary>arg：日期（DateOnly），或 "draft"（今天，載入後直接整理 Claude 草稿）。</summary>
    public async Task ActivateAsync(object? arg)
    {
        if (arg is DateOnly d)
            _day = d;
        if (arg is "draft")
            _day = DateOnly.FromDateTime(DateTime.Today);
        await LoadAsync();
        if (arg is "draft" && _log is not null)
            await BuildDraftAsync();
    }

    async Task LoadAsync()
    {
        var version = ++_version;
        DateText.Text = _day.ToString("M/d（ddd）", Inputs.Zh) + (_day == DateOnly.FromDateTime(DateTime.Today) ? " 今天" : "");
        TodayButton.IsEnabled = _day != DateOnly.FromDateTime(DateTime.Today);
        Inputs.Show(StatusText, null);
        DraftSection.Visibility = Visibility.Collapsed;
        _digest = null;
        TotalText.Text = "讀取中…";
        try
        {
            var log = await _engine.Api.GetWorkLogAsync(_day, CancellationToken.None);
            if (version != _version)
                return;
            _log = log;
            Render(log);
        }
        catch (Exception ex)
        {
            Log.Error("讀取工作日誌失敗", ex);
            if (version == _version)
            {
                TotalText.Text = "—";
                Inputs.Show(StatusText, $"讀取失敗：{ex.Message}");
            }
        }
    }

    void Render(WorkLog log)
    {
        TotalText.Text = $"共 {WorkMath.HoursMinutes(log.TotalMinutes)}";
        EntryList.ItemsSource = log.Entries.Select(e => new
        {
            e.Task,
            Hours = WorkMath.HoursMinutes(e.Minutes),
            Ranges = string.Join("、", e.Ranges.Select(r => $"{r.Start.ToLocalTime():HH:mm}–{r.End.ToLocalTime():HH:mm}")),
        }).ToList();
        EmptyText.Visibility = log.Entries.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        NoteText.Text = log.Markdown;
        NoteSection.Visibility = string.IsNullOrWhiteSpace(log.Markdown) ? Visibility.Collapsed : Visibility.Visible;
        CopyButton.IsEnabled = log.Entries.Count > 0;
    }

    void Copy_Click(object sender, RoutedEventArgs e)
    {
        if (_log is null)
            return;
        var sb = new StringBuilder();
        sb.AppendLine($"{_day:yyyy/M/d} 工作日誌（共 {WorkMath.HoursMinutes(_log.TotalMinutes)}）");
        foreach (var entry in _log.Entries)
            sb.AppendLine($"- {entry.Task}　{WorkMath.HoursMinutes(entry.Minutes)}");
        if (!string.IsNullOrWhiteSpace(_log.Markdown))
            sb.AppendLine().AppendLine(_log.Markdown.Trim());
        try
        {
            Clipboard.SetText(sb.ToString());
            Inputs.Show(StatusText, "已複製到剪貼簿", error: false);
        }
        catch (Exception ex)
        {
            Inputs.Show(StatusText, $"複製失敗：{ex.Message}");
        }
    }

    // ───────────── Claude 整理 ─────────────

    async void Draft_Click(object sender, RoutedEventArgs e) => await BuildDraftAsync();

    async Task BuildDraftAsync()
    {
        if (_log is not { } log)
            return;
        var version = _version;
        var day = _day;
        DraftButton.IsEnabled = false;
        DraftButton.Content = "整理中…";
        Inputs.Show(StatusText, null);
        try
        {
            var (entries, _) = await _engine.LoadDayAsync(day);
            var digest = await _engine.BuildDigestAsync(day, log.Entries, entries);
            if (version != _version)
                return;
            _entries = entries;
            _digest = digest;
            DraftBox.Text = digest.Markdown;
            DraftInfo.Text = digest.PromptCount + digest.CommitCount == 0
                ? "這天在「專案」頁的資料夾裡沒有 Claude Code 對話或 commit，只列出計時的任務。"
                : $"依這天 {digest.PromptCount} 句 Claude Code 指令、{digest.CommitCount} 個 commit 整理；可以先改再複製或存起來。";
            NotesButton.Content = digest.Notes.Count > 0 ? $"寫進工時備註（{digest.Notes.Count} 段）" : "沒有可寫的工時備註";
            NotesButton.IsEnabled = digest.Notes.Count > 0;
            DraftSection.Visibility = Visibility.Visible;
        }
        catch (Exception ex)
        {
            Log.Error("整理 Claude 工作日誌失敗", ex);
            Inputs.Show(StatusText, $"整理失敗：{ex.Message}");
        }
        finally
        {
            DraftButton.Content = "✦ Claude 整理";
            DraftButton.IsEnabled = true;
        }
    }

    void CloseDraft_Click(object sender, RoutedEventArgs e) => DraftSection.Visibility = Visibility.Collapsed;

    void CopyDraft_Click(object sender, RoutedEventArgs e)
    {
        try
        {
            Clipboard.SetText(DraftBox.Text);
            Inputs.Show(StatusText, "草稿已複製到剪貼簿", error: false);
        }
        catch (Exception ex)
        {
            Inputs.Show(StatusText, $"複製失敗：{ex.Message}");
        }
    }

    async void CaptureDraft_Click(object sender, RoutedEventArgs e)
    {
        var text = DraftBox.Text.Trim();
        if (text.Length == 0)
            return;
        CaptureDraftButton.IsEnabled = false;
        await _engine.QuickCaptureAsync(text, todo: false);
        CaptureDraftButton.IsEnabled = true;
        Inputs.Show(StatusText, _engine.Error ?? "已存到隨手記", error: _engine.Error is not null);
    }

    async void ApplyNotes_Click(object sender, RoutedEventArgs e)
    {
        if (_digest is not { Notes.Count: > 0 } digest)
            return;
        var preview = string.Join("\n", _entries
            .Where(x => digest.Notes.ContainsKey(x.Id))
            .Select(x => $"{x.Task} {x.StartedAt?.ToLocalTime():HH:mm}–{(x.Running ? "進行中" : x.EndedAt?.ToLocalTime().ToString("HH:mm"))}：{digest.Notes[x.Id]}"));
        var ok = MessageBox.Show(Window.GetWindow(this)!, $"寫進這些計時段的備註？（已經有備註的不會動）\n\n{preview}", "寫進工時備註",
            MessageBoxButton.OKCancel, MessageBoxImage.Question);
        if (ok != MessageBoxResult.OK)
            return;
        NotesButton.IsEnabled = false;
        await _engine.SetEntryNotesAsync(digest.Notes);
        if (_engine.Error is { } error)
        {
            Inputs.Show(StatusText, error);
            NotesButton.IsEnabled = true;
            return;
        }
        NotesButton.Content = $"已寫入 {digest.Notes.Count} 段備註";
        Inputs.Show(StatusText, "已寫進工時備註，可在「工時紀錄」修改", error: false);
    }

    void Entries_Click(object sender, RoutedEventArgs e) =>
        (Window.GetWindow(this) as WorkspaceWindow)?.OpenDay(_day);

    void Web_Click(object sender, RoutedEventArgs e) => Web.Open(_engine.WebUrl("/notebook"));

    async void Prev_Click(object sender, RoutedEventArgs e)
    {
        _day = _day.AddDays(-1);
        await LoadAsync();
    }

    async void Next_Click(object sender, RoutedEventArgs e)
    {
        _day = _day.AddDays(1);
        await LoadAsync();
    }

    async void Today_Click(object sender, RoutedEventArgs e)
    {
        _day = DateOnly.FromDateTime(DateTime.Today);
        await LoadAsync();
    }
}
