using System.Text;
using System.Windows;
using System.Windows.Controls;

namespace HayakuBar.UI.Work;

/// <summary>
/// 文件、會議紀錄、空間頁面、筆記本、論壇貼文的表格：送出的是 Markdown，Hayaku 會變成表格。
/// 從 Excel 複製（Tab 分隔）貼上時自動轉成 Markdown 表格；〔▦ 表格〕插入一個空白表格。
/// </summary>
public static class MarkdownTable
{
    /// <summary>Excel 複製出來的文字（Tab 分隔、儲存格有換行時用雙引號包起來）→ Markdown 表格；不像表格回 null。</summary>
    public static string? FromTsv(string? text)
    {
        if (string.IsNullOrEmpty(text) || !text.Contains('\t'))
            return null;
        var rows = Parse(text);
        while (rows.Count > 0 && rows[^1].All(c => c.Length == 0))
            rows.RemoveAt(rows.Count - 1);
        if (rows.Count == 0 || rows.All(r => r.Count < 2))
            return null;
        var columns = rows.Max(r => r.Count);
        var sb = new StringBuilder();
        void Line(IReadOnlyList<string> cells)
        {
            sb.Append('|');
            for (var i = 0; i < columns; i++)
                sb.Append(' ').Append(Cell(i < cells.Count ? cells[i] : "")).Append(" |");
            sb.Append('\n');
        }
        Line(rows[0]);
        sb.Append('|').Append(string.Concat(Enumerable.Repeat(" --- |", columns))).Append('\n');
        foreach (var row in rows.Skip(1))
            Line(row);
        return sb.ToString().TrimEnd('\n');
    }

    /// <summary>空白表格：表頭一列＋rows 列，cols 欄。</summary>
    public static string Template(int cols = 3, int rows = 2)
    {
        var header = "|" + string.Concat(Enumerable.Range(1, cols).Select(i => $" 欄位 {i} |"));
        var line = "|" + string.Concat(Enumerable.Repeat(" --- |", cols));
        var body = string.Join("\n", Enumerable.Repeat("|" + string.Concat(Enumerable.Repeat("  |", cols)), rows));
        return $"{header}\n{line}\n{body}";
    }

    static string Cell(string s) => s.Trim().Replace("|", "\\|").Replace("\r\n", "<br>").Replace("\n", "<br>");

    /// <summary>Tab 分隔、換行分列；雙引號包起來的儲存格裡可以有換行與 Tab（"" 是一個引號）。</summary>
    static List<List<string>> Parse(string text)
    {
        var rows = new List<List<string>>();
        var row = new List<string>();
        var cell = new StringBuilder();
        var quoted = false;
        for (var i = 0; i < text.Length; i++)
        {
            var c = text[i];
            if (quoted)
            {
                if (c == '"' && i + 1 < text.Length && text[i + 1] == '"')
                {
                    cell.Append('"');
                    i++;
                }
                else if (c == '"')
                    quoted = false;
                else
                    cell.Append(c);
                continue;
            }
            switch (c)
            {
                case '"' when cell.Length == 0:
                    quoted = true;
                    break;
                case '\t':
                    row.Add(cell.ToString());
                    cell.Clear();
                    break;
                case '\r':
                    break;
                case '\n':
                    row.Add(cell.ToString());
                    cell.Clear();
                    rows.Add(row);
                    row = [];
                    break;
                default:
                    cell.Append(c);
                    break;
            }
        }
        if (cell.Length > 0 || row.Count > 0)
        {
            row.Add(cell.ToString());
            rows.Add(row);
        }
        return rows;
    }

    /// <summary>
    /// 貼上 Excel（或 Google 試算表）的儲存格時改貼成 Markdown 表格（前後空一行，才不會黏在段落裡）。
    /// 只認試算表放進剪貼簿的格式，一般有 Tab 縮排的文字（例如程式碼）照原樣貼。
    /// </summary>
    public static void Attach(TextBox box) =>
        DataObject.AddPastingHandler(box, (_, e) =>
        {
            var data = e.DataObject;
            var fromSheet = data.GetDataPresent("XML Spreadsheet") || data.GetDataPresent(DataFormats.CommaSeparatedValue)
                            || data.GetDataPresent(DataFormats.Html) && (data.GetData(DataFormats.Html) as string)?.Contains("<table", StringComparison.OrdinalIgnoreCase) == true;
            if (!fromSheet || !data.GetDataPresent(DataFormats.UnicodeText) || FromTsv(data.GetData(DataFormats.UnicodeText) as string) is not { } table)
                return;
            e.CancelCommand();
            Insert(box, table);
        });

    /// <summary>在游標的位置插入（前後補空行）。</summary>
    public static void Insert(TextBox box, string table)
    {
        var before = box.Text[..box.SelectionStart];
        var prefix = before.Length == 0 || before.EndsWith("\n\n") ? "" : before.EndsWith('\n') ? "\n" : "\n\n";
        box.SelectedText = prefix + table + "\n\n";
        box.SelectionStart += box.SelectionLength;
        box.SelectionLength = 0;
        box.Focus();
    }

    /// <summary>〔▦ 表格〕按鈕：插入空白表格。</summary>
    public static Button InsertButton(FrameworkElement owner, TextBox box)
    {
        var button = PageParts.Button(owner, "▦ 表格");
        button.ToolTip = "插入表格（Markdown）；從 Excel 複製貼上也會自動變成表格";
        button.Click += (_, _) => Insert(box, Template());
        return button;
    }
}
