using System.Windows;
using System.Windows.Controls;
using HayakuBar.Mcp;

namespace HayakuBar.UI.Work;

/// <summary>
/// 任務詳細的〔＋ 補一段〕：在任務內容最後補一段 Markdown（append_to_task_note），原本的圖片、附件、表格都不動。
/// 按了才打 Hayaku。
/// </summary>
public partial class TasksPage
{
    TextBox? _noteAppendBox;
    Button? _noteAppendButton;
    string? _noteAppendFor;

    /// <summary>換任務時收起補一段；同一個任務重新讀取時保留打到一半的內容。</summary>
    void ResetNoteAppend(TaskDetail d)
    {
        if (_noteAppendFor == d.Key)
            return;
        _noteAppendFor = d.Key;
        NoteAppendPanel.Visibility = Visibility.Collapsed;
        if (_noteAppendBox is not null)
            _noteAppendBox.Text = "";
    }

    void NoteAppendToggle_Click(object sender, RoutedEventArgs e)
    {
        if (_noteAppendBox is null)
        {
            _noteAppendBox = PageParts.Input(this, "要補在任務內容最後的一段（Markdown，不會取代原本的內容）", multiline: true);
            _noteAppendButton = PageParts.Button(this, "補上", primary: true);
            _noteAppendButton.Click += async (_, _) => await AppendNoteAsync();
            var cancel = PageParts.Button(this, "取消");
            cancel.Click += (_, _) => NoteAppendPanel.Visibility = Visibility.Collapsed;
            NoteAppendPanel.Children.Add(_noteAppendBox);
            NoteAppendPanel.Children.Add(new StackPanel
            {
                Orientation = Orientation.Horizontal,
                Children = { _noteAppendButton, cancel, MarkdownTable.InsertButton(this, _noteAppendBox) },
            });
        }
        NoteAppendPanel.Visibility = NoteAppendPanel.Visibility == Visibility.Visible ? Visibility.Collapsed : Visibility.Visible;
        if (NoteAppendPanel.Visibility == Visibility.Visible)
            _noteAppendBox.Focus();
    }

    async Task AppendNoteAsync()
    {
        if (_detail is not { } d || _noteAppendBox is null)
            return;
        var markdown = _noteAppendBox.Text.Trim();
        if (markdown.Length == 0)
        {
            _noteAppendBox.Focus();
            return;
        }
        string? message = null;
        if (!await Inputs.Run(_noteAppendButton, DetailStatus, async () => message = await _engine.Api.AppendTaskNoteAsync(d.Key, markdown, CancellationToken.None)))
            return;
        Inputs.Show(DetailStatus, message is { Length: > 0 } ? message : $"已補在 {d.Key} 的內容最後", error: false);
        _noteAppendBox.Text = "";
        NoteAppendPanel.Visibility = Visibility.Collapsed;
        await RefreshDetailQuietlyAsync(d.Key);
    }
}
