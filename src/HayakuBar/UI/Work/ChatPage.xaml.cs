using System.Windows;
using System.Windows.Controls;
using System.Windows.Input;
using System.Windows.Threading;
using HayakuBar.Core;
using HayakuBar.Infra;

namespace HayakuBar.UI.Work;

/// <summary>一個對話：專案頻道（Project+Channel）或任務聊天（Task）。</summary>
public sealed record Conversation(string Title, string Subtitle, string? Task, string? Project, string? Channel);

/// <summary>聊天：專案頻道與任務聊天，讀訊息、送訊息；開著時每 20 秒更新。</summary>
public partial class ChatPage : UserControl, IWorkPage
{
    readonly TimerEngine _engine;
    readonly DispatcherTimer _poll = new() { Interval = TimeSpan.FromSeconds(20) };
    Conversation? _current;
    bool _channelsLoaded;
    int _version;

    public ChatPage(TimerEngine engine)
    {
        _engine = engine;
        InitializeComponent();
        _poll.Tick += async (_, _) =>
        {
            if (IsVisible && _current is not null)
                await LoadMessagesAsync(scrollToEnd: false);
        };
        Loaded += (_, _) => _poll.Start();
        Unloaded += (_, _) => _poll.Stop();
    }

    /// <summary>arg：任務編號 → 直接打開那個任務的聊天。</summary>
    public async Task ActivateAsync(object? arg)
    {
        if (!_channelsLoaded)
            await LoadChannelsAsync();
        if (arg is string key && KeyFinder.Parse(key) is { } k)
            await OpenAsync(TaskConversation(k));
    }

    Conversation TaskConversation(string key) =>
        new($"{key} {_engine.TitleOf(key)}".Trim(), "任務聊天", key, null, null);

    async Task LoadChannelsAsync()
    {
        ChannelEmpty.Text = "讀取中…";
        ChannelEmpty.Visibility = Visibility.Visible;
        try
        {
            var list = new List<Conversation>();
            foreach (var project in await _engine.Api.ListProjectsAsync(CancellationToken.None))
            {
                foreach (var channel in await _engine.Api.ListChannelsAsync(project.Key, CancellationToken.None))
                    list.Add(new Conversation($"# {channel}", project.Name, null, project.Key, channel));
            }
            ChannelList.ItemsSource = list;
            ChannelEmpty.Visibility = list.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            ChannelEmpty.Text = "沒有可以看的頻道";
            _channelsLoaded = true;
        }
        catch (Exception ex)
        {
            Log.Error("讀取頻道失敗", ex);
            ChannelEmpty.Text = $"讀取失敗：{ex.Message}";
        }
    }

    async void ChannelList_SelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (ChannelList.SelectedItem is Conversation c)
            await OpenAsync(c);
    }

    async void TaskInput_KeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter)
            return;
        e.Handled = true;
        if (KeyFinder.Parse(TaskInput.Text) is not { } key)
        {
            Inputs.Show(StatusText, "請輸入任務編號，例 F2P-97");
            return;
        }
        ChannelList.SelectedItem = null;
        await OpenAsync(TaskConversation(key));
    }

    async Task OpenAsync(Conversation c)
    {
        _current = c;
        ConversationTitle.Text = c.Title;
        ConversationSubtitle.Text = c.Subtitle;
        MessageInput.IsEnabled = SendButton.IsEnabled = ReloadButton.IsEnabled = WebButton.IsEnabled = true;
        MessageInput.Tag = c.Task is not null ? $"傳訊息到 {c.Task}（可用 @帳號），Enter 送出、Shift+Enter 換行"
            : $"傳到 {c.Title}（可用 @帳號），Enter 送出、Shift+Enter 換行";
        MessageList.ItemsSource = null;
        await LoadMessagesAsync(scrollToEnd: true);
        MessageInput.Focus();
    }

    async Task LoadMessagesAsync(bool scrollToEnd)
    {
        if (_current is not { } c)
            return;
        var version = ++_version;
        if (MessageList.ItemsSource is null)
        {
            MessageEmpty.Text = "讀取中…";
            MessageEmpty.Visibility = Visibility.Visible;
        }
        try
        {
            var messages = await _engine.Api.ListChatAsync(c.Task, c.Project, c.Channel, CancellationToken.None);
            if (version != _version || _current != c)
                return;
            var atEnd = MessageScroll.VerticalOffset >= MessageScroll.ScrollableHeight - 4;
            MessageList.ItemsSource = TasksPage.ChatRows(messages);
            MessageEmpty.Text = "還沒有訊息，說點什麼吧";
            MessageEmpty.Visibility = messages.Count == 0 ? Visibility.Visible : Visibility.Collapsed;
            if (scrollToEnd || atEnd)
                await Dispatcher.InvokeAsync(MessageScroll.ScrollToEnd, DispatcherPriority.Loaded);
        }
        catch (Exception ex)
        {
            Log.Error("讀取聊天失敗", ex);
            if (version == _version)
            {
                MessageEmpty.Text = $"讀取失敗：{ex.Message}";
                MessageEmpty.Visibility = Visibility.Visible;
            }
        }
    }

    async void MessageInput_PreviewKeyDown(object sender, KeyEventArgs e)
    {
        if (e.Key != Key.Enter || Keyboard.Modifiers.HasFlag(ModifierKeys.Shift))
            return;
        e.Handled = true;
        await SendAsync();
    }

    async void Send_Click(object sender, RoutedEventArgs e) => await SendAsync();

    async Task SendAsync()
    {
        var body = MessageInput.Text.Trim();
        if (_current is not { } c || body.Length == 0)
            return;
        if (await Inputs.Run(SendButton, StatusText, () => _engine.Api.SendChatAsync(c.Task, c.Project, c.Channel, body, CancellationToken.None)))
        {
            MessageInput.Text = "";
            await LoadMessagesAsync(scrollToEnd: true);
        }
    }

    async void Reload_Click(object sender, RoutedEventArgs e) => await LoadMessagesAsync(scrollToEnd: false);

    void Web_Click(object sender, RoutedEventArgs e)
    {
        if (_current is null)
            return;
        Web.Open(_current.Task is { } task ? _engine.TaskUrl(task) : _engine.WebUrl("/chat"));
    }
}
