using System.Windows;
using System.Windows.Controls;
using HayakuBar.Core;
using HayakuBar.Infra;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>工作台任務列（清單、樹狀、看板、專案頁）的右鍵選單。</summary>
public static class TaskMenu
{
    /// <param name="open">查看詳細。</param>
    /// <param name="delete">刪除任務（呼叫端問過再刪、更新畫面）。</param>
    public static ContextMenu Build(TimerEngine engine, string key, Action open, Func<Task> delete)
    {
        var running = engine.RunningKeys().Contains(key);
        var menu = new ContextMenu();
        MenuItem Add(string header, Action action, bool enabled = true)
        {
            var item = new MenuItem { Header = header, IsEnabled = enabled };
            item.Click += (_, _) => action();
            menu.Items.Add(item);
            return item;
        }
        void Separator() => menu.Items.Add(new Separator { Margin = new Thickness(4), Background = Brushes2.Get("LineBrush") });

        Add(running ? "❚❚  暫停計時" : "▶  開始計時", () => _ = running ? engine.PauseTaskAsync(key) : engine.StartTaskAsync(key));
        var pinned = engine.IsPinned(key);
        Add(pinned ? "📌  取消釘選" : "📌  釘到托盤", () => _ = pinned ? engine.UnpinTaskAsync(key) : engine.PinTaskAsync(key),
            pinned || engine.Pins.Count < TimerEngine.MaxPins);
        menu.Items.Add(StatusMenu(engine, key));
        Separator();
        Add("☰  查看詳細", open);
        Add("⧉  複製編號", () => KeyCopy.Copy(engine, key));
        Add("↗  在網頁開啟", () => Web.Open(engine.TaskUrl(key)));
        Separator();
        Add("🗑  刪除任務…", () => _ = delete());
        return menu;
    }

    /// <summary>改狀態：打開子選單時才讀專案的狀態清單（讀過的有暫存）。</summary>
    static MenuItem StatusMenu(TimerEngine engine, string key)
    {
        var current = engine.StatusOf(key);
        var item = new MenuItem { Header = current is null ? "⇄  改狀態" : $"⇄  改狀態（{current}）" };
        item.Items.Add(new MenuItem { Header = "讀取中…", IsEnabled = false });
        var loaded = false;
        item.SubmenuOpened += async (_, _) =>
        {
            if (loaded)
                return;
            loaded = true;
            ProjectDetail project;
            try
            {
                project = await engine.GetProjectAsync(KeyFinder.ProjectOf(key));
            }
            catch (Exception ex)
            {
                Log.Error("讀取狀態清單失敗", ex);
                item.Items.Clear();
                item.Items.Add(new MenuItem { Header = $"讀取失敗：{ex.Message}", IsEnabled = false });
                loaded = false;
                return;
            }
            item.Items.Clear();
            foreach (var status in project.Statuses)
            {
                var option = new MenuItem { Header = (status.Name == current ? "● " : "    ") + status.Name };
                option.Click += async (_, _) =>
                {
                    await engine.SetTaskStatusAsync(key, status.Name);
                    await engine.AfterExternalChangeAsync(); // 清單上的狀態跟著更新
                };
                item.Items.Add(option);
            }
        };
        return item;
    }
}
