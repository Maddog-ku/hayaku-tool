using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 我的筆記本：左邊筆記清單（可搜尋頁名與內容），右邊看選中那頁的 Markdown 與待辦；
/// 可以新增一頁、在選中那頁最後附加內容。第一次切到這頁才讀，之後用快取，按重新整理才重抓。
/// </summary>
public sealed class NotesPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly TextBox _search;
    readonly ListBox _list = PageParts.List();
    readonly TextBlock _listMessage = PageParts.Muted();
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly TextBlock _title = new() { FontSize = 18, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };
    readonly TextBlock _meta = PageParts.Muted();
    readonly TextBox _content;
    readonly TextBlock _contentMessage = PageParts.Muted("左邊選一頁筆記", 13);
    readonly StackPanel _appendPanel = new() { Margin = new Thickness(0, 12, 0, 0), Visibility = Visibility.Collapsed };
    readonly TextBox _append;
    readonly Button _appendButton;
    readonly StackPanel _createPanel = new() { Margin = new Thickness(0, 0, 0, 12), Visibility = Visibility.Collapsed };
    readonly TextBox _newTitle;
    readonly TextBox _newBody;
    readonly Button _createButton;
    readonly Dictionary<string, NoteContent> _cache = [];
    Notebook? _notebook;
    string? _selected;
    bool _loaded;
    int _version;

    public NotesPage(TimerEngine engine)
    {
        _engine = engine;
        _search = PageParts.Input(this, "🔍 搜尋頁名與內容（Enter）");
        _search.Margin = new Thickness(0, 0, 0, 8);
        _search.KeyDown += async (_, e) =>
        {
            if (e.Key == Key.Enter)
                await SearchAsync();
            else if (e.Key == Key.Escape)
            {
                _search.Text = "";
                RenderNotebook();
            }
        };
        _list.SelectionChanged += async (_, _) =>
        {
            if (_list.SelectedItem is ListBoxItem { Tag: string id })
                await OpenAsync(id);
        };
        _content = PageParts.Reader(this);

        _append = PageParts.Input(this, "要加在這頁最後的內容（Markdown，不會取代原本的內容）", multiline: true);
        _appendButton = PageParts.Button(this, "附加到這頁", primary: true);
        _appendButton.Click += async (_, _) => await AppendAsync();
        _appendPanel.Children.Add(new TextBlock { Text = "附加內容", Style = (Style)FindResource("GroupLabel") });
        _appendPanel.Children.Add(_append);
        _appendPanel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { _appendButton } });

        _newTitle = PageParts.Input(this, "名稱");
        _newBody = PageParts.Input(this, "內容（Markdown，可以先空著）", multiline: true);
        _createButton = PageParts.Button(this, "建立", primary: true);
        _createButton.Click += async (_, _) => await CreateAsync();
        var cancel = PageParts.Button(this, "取消");
        cancel.Click += (_, _) => _createPanel.Visibility = Visibility.Collapsed;
        _createPanel.Children.Add(new TextBlock { Text = "新增筆記（只有你看得到）", Style = (Style)FindResource("GroupLabel"), Margin = new Thickness(2, 0, 0, 8) });
        _createPanel.Children.Add(_newTitle);
        _createPanel.Children.Add(_newBody);
        _createPanel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { _createButton, cancel } });

        var add = PageParts.Button(this, "＋ 新增筆記", primary: true);
        add.Click += (_, _) =>
        {
            _createPanel.Visibility = Visibility.Visible;
            _newTitle.Focus();
        };
        var reload = PageParts.Button(this, "⟳ 重新整理");
        reload.Click += async (_, _) => await LoadAsync();
        var web = PageParts.Button(this, "在網頁開啟 ↗");
        web.Margin = new Thickness(0);
        web.ToolTip = "整理資料夾、刪除、移動請到網頁";
        web.Click += (_, _) => Web.Open(_engine.WebUrl("/notebook"));

        var left = PageParts.Stack(new Thickness(0), _search, _listMessage, _list);
        _listMessage.Margin = new Thickness(2, 0, 0, 6);
        var reader = new StackPanel();
        reader.Children.Add(_title);
        reader.Children.Add(_meta);
        reader.Children.Add(new Border { Height = 10 });
        reader.Children.Add(_contentMessage);
        reader.Children.Add(_content);
        reader.Children.Add(_appendPanel);
        var right = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = reader };

        Content = PageParts.Stack(new Thickness(24, 18, 24, 16),
            PageParts.TitleBar(this, "筆記", add, reload, web), _status, _createPanel, PageParts.TwoColumns(left, right));
        ShowNote(null);
    }

    public Task ActivateAsync(object? arg) => _loaded ? Task.CompletedTask : LoadAsync();

    async Task LoadAsync()
    {
        var version = ++_version;
        _cache.Clear();
        Inputs.Show(_status, null);
        _listMessage.Text = "讀取中…";
        _list.ItemsSource = null;
        try
        {
            var notebook = await _engine.Api.ListNotesAsync(CancellationToken.None);
            if (version != _version)
                return;
            _notebook = notebook;
            _loaded = true;
            _search.Text = "";
            RenderNotebook(); // 上次選的那頁還在的話會選回去，選取時才讀內容
            if (_list.SelectedItem is null)
                ShowNote(null);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error("讀取筆記本失敗", ex);
            _listMessage.Text = "";
            Inputs.Show(_status, $"讀不到筆記本：{ex.Message}");
        }
    }

    /// <summary>照資料夾分組列出（沒有資料夾的放最前面），新的在前。</summary>
    void RenderNotebook()
    {
        if (_notebook is not { } nb)
            return;
        var folders = nb.Folders.ToDictionary(f => f.Id, f => f.Name);
        var rows = nb.Pages
            .OrderBy(p => p.FolderId is null ? 0 : 1)
            .ThenBy(p => p.FolderId is { } f ? folders.GetValueOrDefault(f, "") : "", StringComparer.CurrentCulture)
            .ThenByDescending(p => p.UpdatedAt ?? DateTimeOffset.MinValue)
            .Select(p => PageParts.Row(p.Title, Meta(p, folders), p.Id))
            .ToList();
        _list.ItemsSource = rows;
        _list.SelectedItem = rows.FirstOrDefault(r => r.Tag as string == _selected);
        var counts = nb.Inbox > 0 || nb.Todos > 0 ? $"隨手記 {nb.Inbox}・待辦 {nb.Todos}" : "";
        _listMessage.Text = rows.Count == 0 ? "還沒有筆記，按「＋ 新增筆記」寫第一頁" : $"{rows.Count} 頁" + (counts.Length > 0 ? $"　{counts}" : "");
    }

    static string Meta(NotePage p, Dictionary<string, string> folders)
    {
        var parts = new List<string>();
        if (p.FolderId is { } f && folders.TryGetValue(f, out var folder))
            parts.Add($"📁 {folder}");
        if (p.UpdatedAt is { } at)
            parts.Add(Inputs.Ago(at));
        if (p.TaskCodes.Count > 0)
            parts.Add(string.Join("、", p.TaskCodes));
        return string.Join("・", parts);
    }

    async Task SearchAsync()
    {
        var q = _search.Text.Trim();
        if (q.Length == 0)
        {
            RenderNotebook();
            return;
        }
        var version = ++_version;
        _listMessage.Text = "搜尋中…";
        _list.ItemsSource = null;
        try
        {
            var hits = await _engine.Api.SearchNotesAsync(q, CancellationToken.None);
            if (version != _version)
                return;
            _list.ItemsSource = hits.Select(h => PageParts.Row(h.Title, h.Snippet, h.Id)).ToList();
            _listMessage.Text = hits.Count == 0 ? $"找不到「{q}」（Esc 回到全部）" : $"「{q}」找到 {hits.Count} 頁（Esc 回到全部）";
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error("搜尋筆記失敗", ex);
            _listMessage.Text = "";
            Inputs.Show(_status, $"搜尋失敗：{ex.Message}");
        }
    }

    async Task OpenAsync(string id)
    {
        _selected = id;
        if (_cache.TryGetValue(id, out var cached))
        {
            ShowNote(cached);
            return;
        }
        var version = ++_version;
        _title.Text = _notebook?.Pages.FirstOrDefault(p => p.Id == id)?.Title ?? "";
        _meta.Text = "";
        _content.Visibility = Visibility.Collapsed;
        _contentMessage.Text = "讀取中…";
        _contentMessage.Visibility = Visibility.Visible;
        try
        {
            var note = await _engine.Api.GetNoteAsync(id, CancellationToken.None);
            _cache[id] = note;
            if (version != _version || _selected != id)
                return;
            ShowNote(note);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取筆記 {id} 失敗", ex);
            _contentMessage.Text = $"讀不到這頁：{ex.Message}";
        }
    }

    void ShowNote(NoteContent? note)
    {
        _appendPanel.Visibility = note is null ? Visibility.Collapsed : Visibility.Visible;
        if (note is null)
        {
            _title.Text = "";
            _meta.Text = "";
            _content.Visibility = Visibility.Collapsed;
            _contentMessage.Text = _notebook is { Pages.Count: 0 } ? "" : "左邊選一頁筆記";
            _contentMessage.Visibility = Visibility.Visible;
            return;
        }
        _title.Text = note.Title;
        _meta.Text = note.TaskCodes.Count > 0 ? $"相關任務：{string.Join("、", note.TaskCodes)}" : "";
        var text = note.Markdown.Trim();
        if (note.Todos.Count > 0)
            text += (text.Length > 0 ? "\n\n" : "") + "待辦\n" + string.Join("\n", note.Todos.Select(t => $"{(t.Done ? "☑" : "☐")} {t.Text}"));
        _content.Text = text;
        _content.Visibility = text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        _contentMessage.Text = "這頁還沒有內容";
        _contentMessage.Visibility = text.Length > 0 ? Visibility.Collapsed : Visibility.Visible;
    }

    async Task AppendAsync()
    {
        if (_selected is not { } id)
            return;
        var markdown = _append.Text.Trim();
        if (markdown.Length == 0)
        {
            Inputs.Show(_status, "請先填要附加的內容");
            _append.Focus();
            return;
        }
        if (!await Inputs.Run(_appendButton, _status, () => _engine.Api.AppendNoteAsync(id, markdown, CancellationToken.None), "已附加到這頁"))
            return;
        _append.Text = "";
        _cache.Remove(id);
        await OpenAsync(id);
    }

    async Task CreateAsync()
    {
        var title = _newTitle.Text.Trim();
        if (title.Length == 0)
        {
            Inputs.Show(_status, "請填名稱");
            _newTitle.Focus();
            return;
        }
        string? id = null;
        if (!await Inputs.Run(_createButton, _status, async () =>
                id = await _engine.Api.CreateNoteAsync(title, _newBody.Text.Trim(), CancellationToken.None), $"已新增「{title}」"))
            return;
        _newTitle.Text = _newBody.Text = "";
        _createPanel.Visibility = Visibility.Collapsed;
        _selected = id;
        var message = _status.Text;
        await LoadAsync();
        Inputs.Show(_status, message, error: false);
    }
}
