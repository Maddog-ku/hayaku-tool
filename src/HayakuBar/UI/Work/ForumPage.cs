using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 論壇：選專案 → 頻道（論壇）→ 左邊貼文清單（置頂在最上面，可只看未解決）→ 右邊貼文內容與回應；可以回覆、發新文。
/// 第一次切到這頁才讀；頻道清單、貼文清單、貼文內容快取到按重新整理或換選擇。
/// 頻道清單分不出聊天頻道和論壇，選到聊天頻道時伺服器回「找不到這個論壇」，就提示到「聊天」頁。
/// </summary>
public sealed class ForumPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly ComboBox _project = new() { Width = 240, Margin = new Thickness(0, 0, 8, 0) };
    readonly ComboBox _channel = new() { Width = 180, Margin = new Thickness(0, 0, 8, 0) };
    readonly CheckBox _unsolved = new() { Content = "只看未解決", VerticalAlignment = VerticalAlignment.Center };
    readonly TextBlock _status = new() { TextWrapping = TextWrapping.Wrap, Margin = new Thickness(0, 0, 0, 8), Visibility = Visibility.Collapsed };
    readonly TextBlock _leftMessage = PageParts.Muted();
    readonly ListBox _posts = PageParts.List();
    readonly TextBlock _title = new() { FontSize = 18, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap };
    readonly TextBlock _meta = PageParts.Muted();
    readonly TextBlock _rightMessage = PageParts.Muted("左邊選一篇貼文", 13);
    readonly TextBox _body;
    readonly StackPanel _replies = new();
    readonly StackPanel _replyPanel = new() { Margin = new Thickness(0, 12, 0, 0), Visibility = Visibility.Collapsed };
    readonly TextBox _reply;
    readonly Button _replyButton;
    readonly StackPanel _createPanel = new() { Margin = new Thickness(0, 0, 0, 12), Visibility = Visibility.Collapsed };
    readonly TextBlock _createLabel = new() { FontWeight = FontWeights.SemiBold, Margin = new Thickness(2, 0, 0, 8) };
    readonly TextBox _newTitle;
    readonly TextBox _newBody;
    readonly Button _createButton;
    readonly Dictionary<string, List<string>> _channels = new(StringComparer.OrdinalIgnoreCase);
    readonly Dictionary<string, string> _lastChannel = new(StringComparer.OrdinalIgnoreCase);
    readonly Dictionary<string, List<ForumPost>> _postCache = [];
    readonly Dictionary<string, ForumThread> _threads = [];
    ForumPost? _post;
    bool _loaded;
    bool _building;
    int _version;

    string? ProjectKey => (_project.SelectedItem as ComboBoxItem)?.Tag as string;
    string? Channel => _channel.SelectedItem as string;

    public ForumPage(TimerEngine engine)
    {
        _engine = engine;
        _project.SelectionChanged += async (_, _) =>
        {
            if (!_building)
                await LoadChannelsAsync(refresh: false);
        };
        _channel.SelectionChanged += async (_, _) =>
        {
            if (_building)
                return;
            if (ProjectKey is { } p && Channel is { } c)
                _lastChannel[p] = c;
            await LoadPostsAsync(refresh: false);
        };
        _unsolved.Click += async (_, _) => await LoadPostsAsync(refresh: false);
        _posts.SelectionChanged += async (_, _) =>
        {
            if (_posts.SelectedItem is ListBoxItem { Tag: ForumPost post })
                await OpenAsync(post);
        };
        _body = PageParts.Reader(this);

        _reply = PageParts.Input(this, "回覆這篇（Markdown，可用 @帳號）", multiline: true);
        _replyButton = PageParts.Button(this, "回覆", primary: true);
        _replyButton.Click += async (_, _) => await ReplyAsync();
        _replyPanel.Children.Add(_reply);
        _replyPanel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { _replyButton } });

        _newTitle = PageParts.Input(this, "標題（1～120 字）");
        _newTitle.MaxLength = 120;
        _newBody = PageParts.Input(this, "內文（Markdown，可以空著、可用 @帳號）", multiline: true);
        _createButton = PageParts.Button(this, "發文", primary: true);
        _createButton.Click += async (_, _) => await CreateAsync();
        var cancel = PageParts.Button(this, "取消");
        cancel.Click += (_, _) => _createPanel.Visibility = Visibility.Collapsed;
        _createPanel.Children.Add(_createLabel);
        _createPanel.Children.Add(_newTitle);
        _createPanel.Children.Add(_newBody);
        _createPanel.Children.Add(new StackPanel { Orientation = Orientation.Horizontal, Children = { _createButton, cancel } });

        var add = PageParts.Button(this, "＋ 發文", primary: true);
        add.Click += (_, _) =>
        {
            if (Channel is null)
                return;
            _createLabel.Text = $"在「{Channel}」發文";
            _createPanel.Visibility = Visibility.Visible;
            _newTitle.Focus();
        };
        var reload = PageParts.Button(this, "⟳ 重新整理");
        reload.Click += async (_, _) => await RefreshAsync();
        var web = PageParts.Button(this, "在網頁開啟 ↗");
        web.Margin = new Thickness(0);
        web.ToolTip = "標籤、標成已解決、置頂、編輯請到網頁";
        web.Click += (_, _) => Web.Open(_engine.WebUrl(ProjectKey is { } key ? $"/projects/{Uri.EscapeDataString(key)}" : "/"));

        var filters = new WrapPanel { Margin = new Thickness(0, 0, 0, 12) };
        filters.Children.Add(_project);
        filters.Children.Add(_channel);
        filters.Children.Add(_unsolved);

        _leftMessage.Margin = new Thickness(2, 0, 0, 6);
        var left = PageParts.Stack(new Thickness(0), _leftMessage, _posts);
        var reader = new StackPanel();
        reader.Children.Add(_title);
        reader.Children.Add(_meta);
        reader.Children.Add(new Border { Height = 10 });
        reader.Children.Add(_rightMessage);
        reader.Children.Add(_body);
        reader.Children.Add(_replies);
        reader.Children.Add(_replyPanel);
        var right = new ScrollViewer { VerticalScrollBarVisibility = ScrollBarVisibility.Auto, Content = reader };

        Content = PageParts.Stack(new Thickness(24, 18, 24, 16),
            PageParts.TitleBar(this, "論壇", add, reload, web), filters, _status, _createPanel, PageParts.TwoColumns(left, right, 320));
        ShowThread(null);
    }

    /// <summary>arg：專案代碼（例如從論壇通知點過來）時切到那個專案。</summary>
    public Task ActivateAsync(object? arg)
    {
        var wanted = arg as string;
        if (_loaded && (wanted is null || string.Equals(wanted, ProjectKey, StringComparison.OrdinalIgnoreCase)))
            return Task.CompletedTask;
        _loaded = true;
        RenderProjects(wanted);
        return LoadChannelsAsync(refresh: false);
    }

    void RenderProjects(string? wanted = null)
    {
        var selected = wanted ?? ProjectKey;
        _building = true;
        var items = _engine.Projects.Where(p => !p.IsArchived)
            .Select(p => new ComboBoxItem { Content = ProjectIcons.Badge(p, p.Name, 16), Tag = p.Key })
            .ToList();
        _project.ItemsSource = items;
        _project.SelectedItem = items.FirstOrDefault(i => string.Equals(i.Tag as string, selected, StringComparison.OrdinalIgnoreCase)) ?? items.FirstOrDefault();
        _building = false;
        if (items.Count == 0)
            _leftMessage.Text = "沒有看得到的專案";
    }

    /// <summary>重新整理：清掉快取，重抓目前專案的頻道與貼文。</summary>
    Task RefreshAsync()
    {
        _channels.Clear();
        _postCache.Clear();
        _threads.Clear();
        RenderProjects();
        return LoadChannelsAsync(refresh: true);
    }

    async Task LoadChannelsAsync(bool refresh)
    {
        if (ProjectKey is not { } project)
            return;
        if (refresh || !_channels.TryGetValue(project, out var channels))
        {
            var version = ++_version;
            Inputs.Show(_status, null);
            _leftMessage.Text = "讀取中…";
            _posts.ItemsSource = null;
            try
            {
                channels = await _engine.Api.ListChannelsAsync(project, CancellationToken.None);
                if (version != _version || ProjectKey != project)
                    return;
                _channels[project] = channels;
            }
            catch (Exception ex)
            {
                if (version != _version)
                    return;
                Log.Error($"讀取 {project} 的頻道失敗", ex);
                _leftMessage.Text = "";
                Inputs.Show(_status, $"讀不到頻道：{ex.Message}");
                return;
            }
        }
        _building = true;
        _channel.ItemsSource = channels;
        // 上次在這個專案選的；沒有就挑名稱像論壇的，再沒有就第一個
        _channel.SelectedItem = _lastChannel.TryGetValue(project, out var last) && channels.Contains(last) ? last
            : channels.FirstOrDefault(c => c.Contains("論壇") || c.Contains("forum", StringComparison.OrdinalIgnoreCase)) ?? channels.FirstOrDefault();
        _building = false;
        if (channels.Count == 0)
        {
            _leftMessage.Text = "這個專案沒有頻道";
            _posts.ItemsSource = null;
            ShowThread(null);
            return;
        }
        await LoadPostsAsync(refresh);
    }

    async Task LoadPostsAsync(bool refresh)
    {
        if (ProjectKey is not { } project || Channel is not { } channel)
            return;
        var unsolved = _unsolved.IsChecked == true;
        var cacheKey = $"{project}\n{channel}\n{unsolved}";
        if (!refresh && _postCache.TryGetValue(cacheKey, out var cached))
        {
            RenderPosts(cached);
            return;
        }
        var version = ++_version;
        Inputs.Show(_status, null);
        _leftMessage.Text = "讀取中…";
        _posts.ItemsSource = null;
        try
        {
            var posts = await _engine.Api.ListForumPostsAsync(project, channel, unsolved, CancellationToken.None);
            if (version != _version || ProjectKey != project || Channel != channel)
                return;
            _postCache[cacheKey] = posts;
            RenderPosts(posts);
        }
        catch (McpException ex) when (ex.Message.Contains("NOT_FOUND") || ex.Message.Contains("找不到這個論壇"))
        {
            if (version != _version)
                return;
            _leftMessage.Text = $"「{channel}」是聊天頻道，不是論壇；聊天請到「聊天」頁";
            ShowThread(null);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取 {project} {channel} 的貼文失敗", ex);
            _leftMessage.Text = "";
            Inputs.Show(_status, $"讀不到貼文：{ex.Message}");
        }
    }

    void RenderPosts(List<ForumPost> posts)
    {
        var rows = posts.Select(p => PageParts.Row((p.Pinned ? "📌 " : "") + (p.Solved ? "✓ " : "") + p.Title, PostMeta(p), p)).ToList();
        _posts.ItemsSource = rows;
        _posts.SelectedItem = rows.FirstOrDefault(r => (r.Tag as ForumPost)?.Id == _post?.Id);
        _leftMessage.Text = posts.Count == 0
            ? _unsolved.IsChecked == true ? "沒有未解決的貼文" : "還沒有貼文，按「＋ 發文」"
            : $"{posts.Count} 篇";
        if (_posts.SelectedItem is null)
            ShowThread(null);
    }

    static string PostMeta(ForumPost p)
    {
        var parts = new List<string> { p.Author, p.Replies > 0 ? $"{p.Replies} 則回應" : "還沒有回應" };
        if ((p.LastActivityAt ?? p.CreatedAt) is { } at)
            parts.Add(Inputs.Ago(at));
        if (p.Tags.Count > 0)
            parts.Add(string.Join(" ", p.Tags.Select(t => $"#{t}")));
        return string.Join("・", parts.Where(s => s.Length > 0));
    }

    async Task OpenAsync(ForumPost post, bool refresh = false)
    {
        if (ProjectKey is not { } project || Channel is not { } channel)
            return;
        _post = post;
        if (!refresh && _threads.TryGetValue(post.Id, out var cached))
        {
            ShowThread(cached);
            return;
        }
        var version = ++_version;
        _title.Text = post.Title;
        _meta.Text = "";
        _body.Visibility = Visibility.Collapsed;
        _replies.Children.Clear();
        _replyPanel.Visibility = Visibility.Collapsed;
        _rightMessage.Text = "讀取中…";
        _rightMessage.Visibility = Visibility.Visible;
        try
        {
            var thread = await _engine.Api.GetForumPostAsync(project, channel, post.Id, CancellationToken.None);
            _threads[post.Id] = thread;
            if (version != _version || _post?.Id != post.Id)
                return;
            ShowThread(thread);
        }
        catch (Exception ex)
        {
            if (version != _version)
                return;
            Log.Error($"讀取貼文 {post.Id} 失敗", ex);
            _rightMessage.Text = $"讀不到這篇：{ex.Message}";
        }
    }

    void ShowThread(ForumThread? thread)
    {
        _replies.Children.Clear();
        if (thread is null || _post is null)
        {
            if (thread is null)
                _post = null;
            _title.Text = "";
            _meta.Text = "";
            _body.Visibility = Visibility.Collapsed;
            _replyPanel.Visibility = Visibility.Collapsed;
            _rightMessage.Text = "左邊選一篇貼文";
            _rightMessage.Visibility = Visibility.Visible;
            return;
        }
        _title.Text = (thread.Solved ? "✓ " : "") + (thread.Title.Length > 0 ? thread.Title : _post.Title);
        var meta = new List<string>();
        if (thread.Root is { } root)
        {
            meta.Add(root.Author);
            if (root.CreatedAt is { } at)
                meta.Add(Inputs.Ago(at));
        }
        if (thread.Solved)
            meta.Add("已解決");
        if (thread.Tags.Count > 0)
            meta.Add(string.Join(" ", thread.Tags.Select(t => $"#{t}")));
        _meta.Text = string.Join("・", meta);
        _body.Text = Clean(thread.Root?.Body ?? "");
        _body.Visibility = _body.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        _rightMessage.Visibility = Visibility.Collapsed;
        _replies.Children.Add(new TextBlock
        {
            Text = thread.Replies.Count > 0 ? $"回應（{thread.Replies.Count}）" : "還沒有回應", Style = (Style)FindResource("GroupLabel"),
        });
        foreach (var r in thread.Replies)
        {
            var card = new StackPanel();
            card.Children.Add(PageParts.Muted($"{r.Author}・{(r.CreatedAt is { } at ? Inputs.Ago(at) : "")}", 11));
            var body = PageParts.Reader(this);
            body.Text = r.Deleted ? "（回應已刪除）" : Clean(r.Body);
            body.Margin = new Thickness(0, 2, 0, 0);
            card.Children.Add(body);
            var border = new Border { CornerRadius = new CornerRadius(6), Padding = new Thickness(10, 8, 10, 8), Margin = new Thickness(0, 0, 0, 6), Child = card };
            border.SetResourceReference(Border.BackgroundProperty, "SubtleBrush");
            _replies.Children.Add(border);
        }
        _replyPanel.Visibility = Visibility.Visible;
    }

    /// <summary>Markdown 的強制換行（行尾反斜線）照一般換行顯示。</summary>
    static string Clean(string markdown) => markdown.Replace("\\\n", "\n").Trim();

    async Task ReplyAsync()
    {
        if (ProjectKey is not { } project || Channel is not { } channel || _post is not { } post)
            return;
        var body = _reply.Text.Trim();
        if (body.Length == 0)
        {
            Inputs.Show(_status, "請先填回覆內容");
            _reply.Focus();
            return;
        }
        if (!await Inputs.Run(_replyButton, _status, () => _engine.Api.ReplyForumPostAsync(project, channel, post.Id, body, CancellationToken.None), "已回覆"))
            return;
        _reply.Text = "";
        _postCache.Clear(); // 回應數、最新活動變了
        await OpenAsync(post, refresh: true);
    }

    async Task CreateAsync()
    {
        if (ProjectKey is not { } project || Channel is not { } channel)
            return;
        var title = _newTitle.Text.Trim();
        if (title.Length == 0)
        {
            Inputs.Show(_status, "請填標題");
            _newTitle.Focus();
            return;
        }
        if (!await Inputs.Run(_createButton, _status, () => _engine.Api.CreateForumPostAsync(project, channel, title, _newBody.Text.Trim(), CancellationToken.None),
                $"已發文「{title}」"))
            return;
        _newTitle.Text = _newBody.Text = "";
        _createPanel.Visibility = Visibility.Collapsed;
        var message = _status.Text;
        await LoadPostsAsync(refresh: true);
        Inputs.Show(_status, message, error: false);
    }
}
