using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 團隊空間：選空間 → 左邊頁面樹（可全文搜尋）→ 右邊看那頁的 Markdown；可以新增頁、在頁面最後附加內容、
/// 編輯（改名、整頁換內容）、移到別的上層、刪除（連子頁，30 天內空間管理員可以在網頁還原）。
/// 檔案庫頁列出資料夾與檔案，點檔案拿下載連結（1 小時有效）用瀏覽器開。
/// 第一次切到這頁才讀；頁面樹、頁面內容快取到按重新整理或換空間。
/// </summary>
public sealed class SpacesPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly ComboBox _space = new() { Width = 260, Margin = new Thickness(0, 0, 8, 0) };
    readonly TextBox _search;
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly TextBlock _leftMessage = PageParts.Muted();
    readonly TreeView _tree = new() { BorderThickness = new Thickness(0), Background = System.Windows.Media.Brushes.Transparent };
    readonly ListBox _hits = PageParts.List();
    readonly TextBlock _title = new() { FontSize = 18, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };
    readonly TextBlock _meta = PageParts.Muted();
    readonly TextBlock _rightMessage = PageParts.Muted("左邊選一頁", 13);
    readonly TextBox _content;
    readonly StackPanel _files = new() { Visibility = Visibility.Collapsed };
    readonly ListBox _fileList = PageParts.List();
    readonly Button _up;
    readonly StackPanel _actions = new() { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 12, 0, 0) };
    readonly StackPanel _composer = new() { Margin = new Thickness(0, 10, 0, 0), Visibility = Visibility.Collapsed };
    readonly TextBlock _composerLabel = new() { FontWeight = FontWeights.SemiBold, Margin = new Thickness(2, 0, 0, 6) };
    readonly TextBox _composeTitle;
    readonly TextBox _composeBody;
    readonly Button _composeButton;
    readonly Dictionary<string, List<SpaceNode>> _trees = new(StringComparer.OrdinalIgnoreCase);
    readonly Dictionary<string, SpacePageContent> _pages = [];
    readonly Stack<(string? FolderId, string Name)> _folders = new();
    List<SpaceInfo> _spaces = [];
    readonly Button _newTop;

    /// <summary>選著的空間；專案封存了是唯讀（不能新增頁、附加內容）。</summary>
    SpaceInfo? CurrentSpace => _spaces.FirstOrDefault(s => s.Key == SpaceKey);

    bool Writable => CurrentSpace is not { Archived: true };
    SpaceNode? _node;
    ComposeMode _mode;
    bool _loaded;
    bool _building;
    int _version;

    string? SpaceKey => (_space.SelectedItem as ComboBoxItem)?.Tag as string;

    public SpacesPage(TimerEngine engine)
    {
        _engine = engine;
        _space.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await LoadTreeAsync(refresh: false);
        };
        _search = PageParts.Input(this, "🔍 搜尋這個空間的頁名與內容（Enter）");
        _search.Width = 300;
        _search.Margin = new Thickness(0);
        _search.KeyDown += async (_, e) =>
        {
            if (e.Key == Key.Enter)
                await SearchAsync();
            else if (e.Key == Key.Escape)
            {
                _search.Text = "";
                ShowTree();
            }
        };
        _tree.SelectedItemChanged += async (_, _) =>
        {
            if (_tree.SelectedItem is TreeViewItem { Tag: SpaceNode node })
                await OpenAsync(node);
        };
        _hits.Visibility = Visibility.Collapsed;
        _hits.SelectionChanged += async (_, _) =>
        {
            if (_hits.SelectedItem is ListBoxItem { Tag: KnowledgeHit hit })
                await OpenHitAsync(hit);
        };
        _content = PageParts.Reader(this);

        _up = PageParts.Button(this, "↑ 上一層");
        _up.Click += async (_, _) =>
        {
            if (_folders.Count > 0)
                _folders.Pop();
            await LoadFilesAsync();
        };
        _fileList.MouseDoubleClick += async (_, _) => await OpenFileAsync(download: false);
        _fileList.KeyDown += async (_, e) =>
        {
            if (e.Key == Key.Enter)
                await OpenFileAsync(download: false);
        };
        var download = PageParts.Button(this, "⬇ 下載選取的檔案");
        download.ToolTip = "用瀏覽器下載原檔（連結 1 小時有效）";
        download.Click += async (_, _) => await OpenFileAsync(download: true);
        var fileHint = PageParts.Muted("點兩下資料夾進去、點兩下檔案直接看內容（圖片、PDF、Word、Excel、文字檔）。上傳、刪除請到網頁。", 11);
        fileHint.Margin = new Thickness(0, 0, 0, 6);
        _files.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Margin = new Thickness(0, 0, 0, 6), Children = { _up, download } });
        _files.Children.Add(fileHint);
        _files.Children.Add(_fileList);

        var child = PageParts.Button(this, "＋ 新增子頁");
        child.Click += (_, _) => Compose(ComposeMode.Child);
        var append = PageParts.Button(this, "附加內容");
        append.Click += (_, _) => Compose(ComposeMode.Append);
        var edit = PageParts.Button(this, "✎ 編輯");
        edit.ToolTip = "改頁名、整頁換成新的內容（換掉前網頁會自動存一版）";
        edit.Click += (_, _) => Compose(ComposeMode.Edit);
        var move = PageParts.Button(this, "移動…");
        move.Click += async (_, _) => await MoveAsync();
        var delete = PageParts.Button(this, "刪除…");
        delete.Click += async (_, _) => await DeleteAsync();
        _actions.Children.Add(child);
        _actions.Children.Add(append);
        _actions.Children.Add(edit);
        _actions.Children.Add(move);
        _actions.Children.Add(delete);
        _actions.Visibility = Visibility.Collapsed;

        _composeTitle = PageParts.Input(this, "名稱");
        _composeBody = PageParts.Input(this, "內容（Markdown）", multiline: true);
        _composeButton = PageParts.Button(this, "送出", primary: true);
        _composeButton.Click += async (_, _) => await SubmitAsync();
        var cancel = PageParts.Button(this, "取消");
        cancel.Click += (_, _) => _composer.Visibility = Visibility.Collapsed;
        var warn = PageParts.Muted("團隊空間是公司的正式文件，大家都看得到；送出前請確認內容。", 11);
        warn.SetResourceReference(TextBlock.ForegroundProperty, "WarnBrush");
        warn.Margin = new Thickness(2, 0, 0, 6);
        _composer.Children.Add(_composerLabel);
        _composer.Children.Add(warn);
        _composer.Children.Add(_composeTitle);
        _composer.Children.Add(_composeBody);
        _composer.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { _composeButton, cancel, MarkdownTable.InsertButton(this, _composeBody) } });

        var newTop = _newTop = PageParts.Button(this, "＋ 新頁", primary: true);
        newTop.ToolTip = "在這個空間的最上層新增一頁";
        newTop.Click += (_, _) =>
        {
            _node = _node is { IsFiles: true } ? null : _node;
            Compose(ComposeMode.Child, topLevel: true);
        };
        var reload = PageParts.Button(this, "⟳ 重新整理");
        reload.Click += async (_, _) => await LoadSpacesAsync();
        var web = PageParts.Button(this, "在網頁開啟 ↗");
        web.Margin = new Thickness(0);
        web.ToolTip = "上傳、刪除檔案、還原刪掉的頁面請到網頁";
        web.Click += (_, _) => Web.Open(_engine.WebUrl(SpaceKey is { } key ? $"/spaces/{Uri.EscapeDataString(key)}" : "/spaces"));

        var filters = new WrapPanel { Margin = new Thickness(0, 0, 0, 12) };
        filters.Children.Add(_space);
        filters.Children.Add(_search);

        _leftMessage.Margin = new Thickness(2, 0, 0, 6);
        var leftHost = new Grid();
        leftHost.Children.Add(_tree);
        leftHost.Children.Add(_hits);
        var left = PageParts.Stack(new Thickness(0), _leftMessage, leftHost);
        var reader = new StackPanel();
        reader.Children.Add(_title);
        reader.Children.Add(_meta);
        reader.Children.Add(new Border { Height = 10 });
        reader.Children.Add(_rightMessage);
        reader.Children.Add(_content);
        reader.Children.Add(_files);
        reader.Children.Add(_actions);
        reader.Children.Add(_composer);
        var right = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = reader };

        Content = PageParts.Stack(new Thickness(24, 18, 24, 16),
            PageParts.TitleBar(this, "團隊空間", newTop, reload, web), filters, _status, PageParts.TwoColumns(left, right));
        ShowEmpty();
    }

    public Task ActivateAsync(object? arg) => _loaded ? Task.CompletedTask : LoadSpacesAsync();

    /// <summary>重新整理：重抓空間清單，清掉頁面樹與內容的快取。</summary>
    async Task LoadSpacesAsync()
    {
        var version = ++_version;
        _trees.Clear();
        _pages.Clear();
        Inputs.Show(_status, null);
        _leftMessage.Text = "讀取中…";
        try
        {
            var spaces = await _engine.Api.ListSpacesAsync(CancellationToken.None);
            if (version != _version)
                return;
            _spaces = spaces;
            _loaded = true;
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error("讀取團隊空間失敗", ex);
            _leftMessage.Text = "";
            Inputs.Show(_status, $"讀不到團隊空間：{ex.Message}");
            return;
        }
        var selected = SpaceKey;
        _building = true;
        // 專案裡開的空間標專案代碼（跟網頁側邊欄一樣），專案封存了的標唯讀
        var items = _spaces.Select(s => new ComboBoxItem
        {
            Content = (s.Project is { } p ? $"[{p}] " : "") + (s.PageCount is { } n ? $"{s.Name}（{n} 頁）" : s.Name) + (s.Archived ? "・唯讀" : ""),
            Tag = s.Key,
            ToolTip = string.Join("・", new[]
            {
                s.Key, s.Role is { } role ? $"我的角色：{role}" : null, s.Project is { } proj ? $"專案 {proj} 的空間（成員和權限跟著專案）" : null,
                s.Archived ? "專案已封存，只能看" : null,
            }.Where(x => x is not null)),
        }).ToList();
        _space.ItemsSource = items;
        _space.SelectedItem = items.FirstOrDefault(i => i.Tag as string == selected) ?? items.FirstOrDefault();
        _building = false;
        if (items.Count == 0)
        {
            _leftMessage.Text = "你還沒有看得到的團隊空間";
            _tree.Items.Clear();
            ShowEmpty();
            return;
        }
        await LoadTreeAsync(refresh: true);
    }

    async Task LoadTreeAsync(bool refresh)
    {
        if (SpaceKey is not { } space)
            return;
        _newTop.Visibility = Writable ? Visibility.Visible : Visibility.Collapsed;
        _search.Text = "";
        ShowTree();
        if (!refresh && _trees.TryGetValue(space, out var cached))
        {
            RenderTree(cached);
            return;
        }
        var version = ++_version;
        _leftMessage.Text = "讀取中…";
        _tree.Items.Clear();
        try
        {
            var nodes = await _engine.Api.SpaceTreeAsync(space, CancellationToken.None);
            if (version != _version || SpaceKey != space)
                return;
            _trees[space] = nodes;
            RenderTree(nodes);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取團隊空間 {space} 的頁面樹失敗", ex);
            _leftMessage.Text = "";
            Inputs.Show(_status, $"讀不到頁面：{ex.Message}");
        }
    }

    void RenderTree(List<SpaceNode> nodes)
    {
        _tree.Items.Clear();
        var byParent = nodes.ToLookup(n => n.ParentId is { } p && nodes.Any(x => x.Id == p) ? p : null);
        TreeViewItem Item(SpaceNode n, int depth)
        {
            var item = new TreeViewItem
            {
                Header = new TextBlock { Text = (n.IsFiles ? "🗂 " : "") + n.Title, TextTrimming = TextTrimming.CharacterEllipsis, Margin = new Thickness(0, 2, 0, 2) },
                Tag = n, IsExpanded = depth == 0,
            };
            foreach (var c in byParent[n.Id])
                item.Items.Add(Item(c, depth + 1));
            return item;
        }
        foreach (var root in byParent[null])
            _tree.Items.Add(Item(root, 0));
        _leftMessage.Text = nodes.Count == 0 ? "這個空間還沒有頁面，按「＋ 新頁」新增" : $"{nodes.Count} 頁";
        if (_node is null || !nodes.Any(n => n.Id == _node.Id))
            ShowEmpty();
    }

    void ShowTree()
    {
        _tree.Visibility = Visibility.Visible;
        _hits.Visibility = Visibility.Collapsed;
        if (SpaceKey is { } space && _trees.TryGetValue(space, out var nodes))
            _leftMessage.Text = nodes.Count == 0 ? "這個空間還沒有頁面，按「＋ 新頁」新增" : $"{nodes.Count} 頁";
    }

    async Task SearchAsync()
    {
        var q = _search.Text.Trim();
        if (q.Length == 0)
        {
            ShowTree();
            return;
        }
        var version = ++_version;
        _tree.Visibility = Visibility.Collapsed;
        _hits.Visibility = Visibility.Visible;
        _hits.ItemsSource = null;
        _leftMessage.Text = "搜尋中…";
        try
        {
            var hits = await _engine.Api.SearchSpacesAsync(q, SpaceKey, CancellationToken.None);
            if (version != _version)
                return;
            _hits.ItemsSource = hits.Select(h => PageParts.Row(h.Title, h.Snippet, h)).ToList();
            _leftMessage.Text = hits.Count == 0 ? $"找不到「{q}」（Esc 回到頁面樹）" : $"「{q}」找到 {hits.Count} 頁（Esc 回到頁面樹）";
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error("搜尋團隊空間失敗", ex);
            _leftMessage.Text = "";
            Inputs.Show(_status, $"搜尋失敗：{ex.Message}");
        }
    }

    Task OpenHitAsync(KnowledgeHit hit)
    {
        var space = hit.Space ?? SpaceKey;
        if (space is null)
            return Task.CompletedTask;
        var node = SpaceKey is { } current && _trees.TryGetValue(current, out var nodes) ? nodes.FirstOrDefault(n => n.Id == hit.Id) : null;
        return OpenAsync(node ?? new SpaceNode(hit.Id, hit.Title, null, null, null), space);
    }

    async Task OpenAsync(SpaceNode node, string? space = null)
    {
        space ??= SpaceKey;
        if (space is null)
            return;
        _node = node;
        _composer.Visibility = Visibility.Collapsed;
        _title.Text = node.Title;
        if (node.IsFiles)
        {
            _folders.Clear();
            await LoadFilesAsync();
            return;
        }
        _files.Visibility = Visibility.Collapsed;
        if (_pages.TryGetValue(node.Id, out var cached))
        {
            ShowPage(cached);
            return;
        }
        var version = ++_version;
        _meta.Text = "";
        _content.Visibility = Visibility.Collapsed;
        _actions.Visibility = Visibility.Collapsed;
        _rightMessage.Text = "讀取中…";
        _rightMessage.Visibility = Visibility.Visible;
        try
        {
            var page = await _engine.Api.GetSpacePageAsync(space, node.Id, CancellationToken.None);
            _pages[node.Id] = page;
            if (version != _version || _node?.Id != node.Id)
                return;
            ShowPage(page);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取團隊空間頁面 {node.Id} 失敗", ex);
            _rightMessage.Text = $"讀不到這頁：{ex.Message}";
        }
    }

    void ShowPage(SpacePageContent page)
    {
        _title.Text = page.Title;
        var meta = new List<string>();
        if (page.Path.Count > 0)
            meta.Add(string.Join(" › ", page.Path));
        if (page.Owner is { } owner)
            meta.Add($"負責人 {owner}");
        if (page.UpdatedAt is { } at)
            meta.Add($"{Inputs.Ago(at)}更新");
        _meta.Text = string.Join("・", meta);
        _content.Text = page.Markdown.Trim();
        _content.Visibility = _content.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        _rightMessage.Text = "這頁還沒有內容";
        _rightMessage.Visibility = _content.Text.Length > 0 ? Visibility.Collapsed : Visibility.Visible;
        _actions.Visibility = Writable ? Visibility.Visible : Visibility.Collapsed;
        if (!Writable)
            _meta.Text += (_meta.Text.Length > 0 ? "・" : "") + "專案已封存，只能看";
    }

    void ShowEmpty()
    {
        _node = null;
        _title.Text = "";
        _meta.Text = "";
        _content.Visibility = Visibility.Collapsed;
        _files.Visibility = Visibility.Collapsed;
        _actions.Visibility = Visibility.Collapsed;
        _rightMessage.Text = _spaces.Count == 0 && _loaded ? "" : "左邊選一頁";
        _rightMessage.Visibility = Visibility.Visible;
    }

    // ───────────── 檔案庫 ─────────────

    async Task LoadFilesAsync()
    {
        if (_node is not { IsFiles: true } node || SpaceKey is not { } space)
            return;
        var version = ++_version;
        var folderId = _folders.Count > 0 ? _folders.Peek().FolderId : null;
        _content.Visibility = Visibility.Collapsed;
        _actions.Visibility = Visibility.Collapsed;
        _files.Visibility = Visibility.Visible;
        _up.Visibility = _folders.Count > 0 ? Visibility.Visible : Visibility.Collapsed;
        _meta.Text = _folders.Count > 0 ? "檔案庫 › " + string.Join(" › ", _folders.Reverse().Select(f => f.Name)) : "檔案庫";
        _fileList.ItemsSource = null;
        _rightMessage.Text = "讀取中…";
        _rightMessage.Visibility = Visibility.Visible;
        try
        {
            var files = await _engine.Api.ListSpaceFilesAsync(space, node.Id, folderId, CancellationToken.None);
            if (version != _version || _node?.Id != node.Id)
                return;
            _fileList.ItemsSource = files.Select(f => PageParts.Row((f.IsFolder ? "📁 " : "📄 ") + f.Name, FileMeta(f), f)).ToList();
            _rightMessage.Text = files.Count == 0 ? "這裡沒有檔案" : "";
            _rightMessage.Visibility = files.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取檔案庫 {node.Id} 失敗", ex);
            _rightMessage.Text = $"讀不到檔案：{ex.Message}";
        }
    }

    static string FileMeta(SpaceFile f) => string.Join("・", new[]
    {
        PageParts.Size(f.Size), f.Uploader ?? "", f.At is { } at ? Inputs.Ago(at) : "",
    }.Where(s => s.Length > 0));

    /// <summary>資料夾：進去；檔案：直接看內容（read_file），download 才拿下載連結用瀏覽器下載。</summary>
    async Task OpenFileAsync(bool download)
    {
        if (_fileList.SelectedItem is not ListBoxItem { Tag: SpaceFile file })
        {
            if (download)
                Inputs.Show(_status, "先在清單選一個檔案");
            return;
        }
        if (file.IsFolder)
        {
            if (download)
                return;
            _folders.Push((file.Id, file.Name));
            await LoadFilesAsync();
            return;
        }
        if (!download)
        {
            FilePreviewWindow.Show(_engine, file.Id, file.Name, Window.GetWindow(this));
            return;
        }
        string? link = null;
        if (await Inputs.Run(null, _status, async () => link = await _engine.Api.SpaceFileLinkAsync(file.Id, CancellationToken.None)))
        {
            if (link is null)
                Inputs.Show(_status, "拿不到下載連結，請到網頁下載");
            else
                Web.Open(link);
        }
    }

    // ───────────── 新增頁、附加內容、編輯 ─────────────

    enum ComposeMode { Child, Append, Edit }

    void Compose(ComposeMode mode, bool topLevel = false)
    {
        if (SpaceKey is null)
            return;
        if (topLevel)
            _node = null;
        SpacePageContent? page = null;
        if (mode == ComposeMode.Edit && (_node is null || !_pages.TryGetValue(_node.Id, out page)))
            return;
        _mode = mode;
        _composerLabel.Text = mode switch
        {
            ComposeMode.Append => $"附加到「{_node?.Title}」最後",
            ComposeMode.Edit => $"編輯「{_node?.Title}」",
            _ => _node is null ? "在最上層新增一頁" : $"在「{_node.Title}」底下新增一頁",
        };
        _composeTitle.Visibility = mode == ComposeMode.Append ? Visibility.Collapsed : Visibility.Visible;
        _composeBody.Tag = mode switch
        {
            ComposeMode.Append => "要加在這頁最後的內容（Markdown，不會取代原本的內容）",
            ComposeMode.Edit => "整頁的內容（Markdown，會取代原本的內容）",
            _ => "內容（Markdown，可以先空著）",
        };
        if (page is not null)
        {
            _composeTitle.Text = page.Title;
            _composeBody.Text = page.Markdown.Trim();
        }
        _composer.Visibility = Visibility.Visible;
        if (mode == ComposeMode.Append)
            _composeBody.Focus();
        else
            _composeTitle.Focus();
    }

    async Task SubmitAsync()
    {
        if (SpaceKey is not { } space)
            return;
        var title = _composeTitle.Text.Trim();
        var body = _composeBody.Text.Trim();
        if (_mode != ComposeMode.Append && title.Length == 0)
        {
            Inputs.Show(_status, "請填名稱");
            _composeTitle.Focus();
            return;
        }
        if (_mode == ComposeMode.Append && (body.Length == 0 || _node is null))
        {
            Inputs.Show(_status, "請先填要附加的內容");
            _composeBody.Focus();
            return;
        }
        var parent = _node;
        var mode = _mode;
        bool ok;
        switch (mode)
        {
            case ComposeMode.Child:
                ok = await Inputs.Run(_composeButton, _status, () => _engine.Api.CreateSpacePageAsync(space, parent?.Id, title, body, CancellationToken.None),
                    $"已新增「{title}」");
                break;
            case ComposeMode.Append:
                ok = await Inputs.Run(_composeButton, _status, () => _engine.Api.AppendSpacePageAsync(space, parent!.Id, body, CancellationToken.None),
                    "已附加到這頁");
                break;
            default:
                if (parent is null || !_pages.TryGetValue(parent.Id, out var old))
                    return;
                // 只送有改的：沒改名不送 title、內容沒變不送 markdown（免得多存一版）
                var newTitle = title == old.Title ? null : title;
                var newBody = body == old.Markdown.Trim() ? null : body;
                if (newTitle is null && newBody is null)
                {
                    _composer.Visibility = Visibility.Collapsed;
                    return;
                }
                ok = await Inputs.Run(_composeButton, _status,
                    () => _engine.Api.UpdateSpacePageAsync(space, parent.Id, newTitle, newBody, CancellationToken.None), "已儲存");
                break;
        }
        if (!ok)
            return;
        _composeTitle.Text = _composeBody.Text = "";
        _composer.Visibility = Visibility.Collapsed;
        var message = _status.Text;
        if (mode == ComposeMode.Child || mode == ComposeMode.Edit && title != parent?.Title)
            await LoadTreeAsync(refresh: true);
        if (mode != ComposeMode.Child && parent is not null)
        {
            _pages.Remove(parent.Id);
            await OpenAsync(parent with { Title = mode == ComposeMode.Edit ? title : parent.Title });
        }
        Inputs.Show(_status, message, error: false);
    }

    // ───────────── 移動、刪除 ─────────────

    /// <summary>選新的上層（同一個空間、不能放進自己或子頁底下），移過去排在最後。</summary>
    async Task MoveAsync()
    {
        if (SpaceKey is not { } space || _node is not { } node || !_trees.TryGetValue(space, out var nodes))
            return;
        var blocked = new HashSet<string> { node.Id };
        bool added;
        do
        {
            added = false;
            foreach (var n in nodes)
                if (n.ParentId is { } p && blocked.Contains(p) && blocked.Add(n.Id))
                    added = true;
        } while (added);
        var byId = nodes.ToDictionary(n => n.Id);
        int Depth(SpaceNode n) => n.ParentId is { } p && byId.TryGetValue(p, out var parent) ? Depth(parent) + 1 : 0;
        var choices = new List<(string? Id, string Text)> { (null, "（最上層）") };
        choices.AddRange(nodes.Where(n => !blocked.Contains(n.Id) && !n.IsFiles)
            .Select(n => ((string?)n.Id, new string(' ', Depth(n) * 3) + n.Title)));
        var current = node.ParentId is { } cp && byId.ContainsKey(cp) ? cp : null;
        var picker = new SpaceMoveWindow(node.Title, choices, current) { Owner = Window.GetWindow(this) };
        if (picker.ShowDialog() != true || picker.Selected == current)
            return;
        var target = picker.Selected;
        if (await Inputs.Run(null, _status, () => _engine.Api.MoveSpacePageAsync(space, node.Id, target, CancellationToken.None), $"已移動「{node.Title}」"))
        {
            var message = _status.Text;
            await LoadTreeAsync(refresh: true);
            Inputs.Show(_status, message, error: false);
        }
    }

    async Task DeleteAsync()
    {
        if (SpaceKey is not { } space || _node is not { } node)
            return;
        var children = _trees.TryGetValue(space, out var nodes) ? nodes.Count(n => n.ParentId == node.Id) : 0;
        var ok = MessageBox.Show(Window.GetWindow(this),
            $"刪除「{node.Title}」？" + (children > 0 ? "底下的子頁會一起刪除。" : "") + "\n30 天內空間管理員可以在網頁還原。",
            "刪除頁面", MessageBoxButton.OKCancel, MessageBoxImage.Warning);
        if (ok != MessageBoxResult.OK)
            return;
        if (await Inputs.Run(null, _status, () => _engine.Api.DeleteSpacePageAsync(space, node.Id, CancellationToken.None), $"已刪除「{node.Title}」"))
        {
            var message = _status.Text;
            _pages.Remove(node.Id);
            ShowEmpty();
            await LoadTreeAsync(refresh: true);
            Inputs.Show(_status, message, error: false);
        }
    }
}

/// <summary>移動團隊空間頁面：選新的上層（排在最後）。</summary>
sealed class SpaceMoveWindow : SmallDialog
{
    readonly ListBox _list = new() { Height = 280, Margin = new Thickness(0, 0, 0, 12) };

    public string? Selected { get; private set; }

    public SpaceMoveWindow(string title, IReadOnlyList<(string? Id, string Text)> choices, string? current) : base("移動頁面")
    {
        Body.Children.Add(Label($"把「{title}」移到哪一頁底下？（排在最後）"));
        foreach (var c in choices)
            _list.Items.Add(new ListBoxItem { Content = c.Text + (c.Id == current ? "　· 目前位置" : ""), Tag = c.Id });
        _list.SelectedIndex = Math.Max(0, choices.ToList().FindIndex(c => c.Id == current));
        _list.MouseDoubleClick += (_, _) => Done();
        Body.Children.Add(_list);
        AddButton("移動", primary: true, () =>
        {
            Done();
            return Task.CompletedTask;
        });
        AddButton("取消", primary: false, cancel: true);
    }

    void Done()
    {
        if (_list.SelectedItem is not ListBoxItem item)
            return;
        Selected = item.Tag as string;
        DialogResult = true;
    }
}
