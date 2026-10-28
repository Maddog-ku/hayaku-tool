using HayakuBar.Infra;
using Drawing = System.Drawing;
using WinForms = System.Windows.Forms;

namespace HayakuBar.UI;

public sealed record TrayActions(
    Action TogglePanel,
    Action Refresh,
    Action Settings,
    Action LoginOrLogout,
    Func<bool> IsLoggedIn,
    Func<TaskbarAnchor> GetAnchor,
    Action<TaskbarAnchor> SetAnchor,
    Action ResetOffset,
    Func<WidgetLayout> GetLayout,
    Action<WidgetLayout> SetLayout,
    Action OpenWeb,
    Action OpenLog,
    Action Exit);

/// <summary>系統匣圖示：選單（登入、工作列位置、設定、開機啟動、結束）與通知。工作列上的工時按右鍵也是這個選單。</summary>
public sealed class TrayIcon : IDisposable
{
    readonly WinForms.NotifyIcon _icon;
    readonly WinForms.ContextMenuStrip _menu;

    public TrayIcon(TrayActions actions)
    {
        _menu = new WinForms.ContextMenuStrip();
        _menu.Items.Add("開啟面板", null, (_, _) => actions.TogglePanel());
        _menu.Items.Add("立即同步", null, (_, _) => actions.Refresh());
        _menu.Items.Add(new WinForms.ToolStripSeparator());

        var position = new WinForms.ToolStripMenuItem("工作列位置");
        var anchors = new (TaskbarAnchor Anchor, string Text)[]
        {
            (TaskbarAnchor.Left, "最左邊"),
            (TaskbarAnchor.BeforeStart, "工作列圖示左側"),
            (TaskbarAnchor.AfterIcons, "工作列圖示右側"),
            (TaskbarAnchor.BeforeTray, "通知區域左側"),
        };
        var anchorItems = anchors.Select(a =>
        {
            var item = new WinForms.ToolStripMenuItem(a.Text) { Tag = a.Anchor };
            item.Click += (_, _) => actions.SetAnchor(a.Anchor);
            return item;
        }).ToList();
        position.DropDownItems.AddRange(anchorItems.ToArray<WinForms.ToolStripItem>());
        position.DropDownItems.Add(new WinForms.ToolStripSeparator());
        position.DropDownItems.Add("重設拖曳微調", null, (_, _) => actions.ResetOffset());
        position.DropDownItems.Add(new WinForms.ToolStripLabel("（可直接左右拖曳；空間不夠會自動避開圖示）") { Enabled = false });
        _menu.Items.Add(position);

        var layout = new WinForms.ToolStripMenuItem("顯示方式");
        var layoutItems = new (WidgetLayout Layout, string Text)[]
        {
            (WidgetLayout.OneLine, "一排（較寬）"),
            (WidgetLayout.TwoLine, "兩排（較窄）"),
        }.Select(l =>
        {
            var item = new WinForms.ToolStripMenuItem(l.Text) { Tag = l.Layout };
            item.Click += (_, _) => actions.SetLayout(l.Layout);
            return item;
        }).ToList();
        layout.DropDownItems.AddRange(layoutItems.ToArray<WinForms.ToolStripItem>());
        _menu.Items.Add(layout);

        var login = _menu.Items.Add("登入 Hayaku", null, (_, _) => actions.LoginOrLogout());
        _menu.Items.Add("設定…", null, (_, _) => actions.Settings());
        var startup = new WinForms.ToolStripMenuItem("開機時自動啟動") { CheckOnClick = true };
        startup.Click += (_, _) =>
        {
            try { StartupRegistration.Set(startup.Checked); }
            catch (Exception ex) { Log.Error("設定開機啟動失敗", ex); }
        };
        _menu.Items.Add(startup);
        _menu.Items.Add(new WinForms.ToolStripSeparator());
        _menu.Items.Add("開啟 Hayaku 網頁", null, (_, _) => actions.OpenWeb());
        _menu.Items.Add("開啟紀錄與設定資料夾", null, (_, _) => actions.OpenLog());
        _menu.Items.Add(new WinForms.ToolStripSeparator());
        _menu.Items.Add("結束", null, (_, _) => actions.Exit());
        _menu.Opening += (_, _) =>
        {
            login.Text = actions.IsLoggedIn() ? "登出 Hayaku" : "登入 Hayaku";
            startup.Checked = StartupRegistration.IsEnabled();
            var current = actions.GetAnchor();
            foreach (var item in anchorItems)
                item.Checked = (TaskbarAnchor)item.Tag! == current;
            var currentLayout = actions.GetLayout();
            foreach (var item in layoutItems)
                item.Checked = (WidgetLayout)item.Tag! == currentLayout;
        };

        _icon = new WinForms.NotifyIcon
        {
            Icon = CreateIcon(),
            Text = "HayakuBar",
            ContextMenuStrip = _menu,
            Visible = true,
        };
        _icon.MouseClick += (_, e) =>
        {
            if (e.Button == WinForms.MouseButtons.Left)
                actions.TogglePanel();
        };
    }

    public void ShowMenu(Drawing.Point screenPoint) => _menu.Show(screenPoint, WinForms.ToolStripDropDownDirection.AboveRight);

    public void Balloon(string title, string text) =>
        _icon.ShowBalloonTip(5000, title, text, WinForms.ToolTipIcon.Info);

    public void SetTooltip(string text) => _icon.Text = text.Length > 63 ? text[..63] : text;

    static Drawing.Icon CreateIcon()
    {
        using var bmp = new Drawing.Bitmap(32, 32);
        using (var g = Drawing.Graphics.FromImage(bmp))
        {
            g.SmoothingMode = Drawing.Drawing2D.SmoothingMode.AntiAlias;
            g.Clear(Drawing.Color.Transparent);
            using var fill = new Drawing.SolidBrush(Drawing.Color.FromArgb(217, 98, 43));
            g.FillEllipse(fill, 1, 1, 30, 30);
            using var pen = new Drawing.Pen(Drawing.Color.White, 3.2f)
            {
                StartCap = Drawing.Drawing2D.LineCap.Round,
                EndCap = Drawing.Drawing2D.LineCap.Round,
            };
            g.DrawLine(pen, 16, 16, 16, 7);
            g.DrawLine(pen, 16, 16, 23, 20);
        }
        return Drawing.Icon.FromHandle(bmp.GetHicon());
    }

    public void Dispose()
    {
        _icon.Visible = false;
        _icon.Dispose();
        _menu.Dispose();
    }
}
