using System.Windows;
using System.Windows.Controls;
using System.Windows.Media;
using HayakuBar.Mcp;

namespace HayakuBar.UI;

/// <summary>
/// Hayaku 專案圖示（名稱，例 rocket）→ emoji，顏色名稱（例 moss）→ 色塊。
/// Hayaku 有 500 多個圖示，常用的對到 emoji；對不到的用專案名稱第一個字。
/// </summary>
public static class ProjectIcons
{
    static readonly Dictionary<string, string> Glyphs = new(StringComparer.OrdinalIgnoreCase)
    {
        // 工作、工具
        ["rocket"] = "🚀", ["robot"] = "🤖", ["tool"] = "🔧", ["tools"] = "🛠", ["wrench"] = "🔧", ["hammer"] = "🔨",
        ["experiment"] = "🧪", ["flask"] = "🧪", ["bulb"] = "💡", ["idea"] = "💡", ["code"] = "💻", ["laptop"] = "💻",
        ["desktop"] = "🖥", ["mobile"] = "📱", ["phone"] = "📱", ["printer"] = "🖨", ["database"] = "🗄", ["cloud"] = "☁",
        ["server"] = "🗄", ["api"] = "🔌", ["plug"] = "🔌", ["bug"] = "🐞", ["setting"] = "⚙", ["settings"] = "⚙", ["gear"] = "⚙",
        ["folder"] = "📁", ["file"] = "📄", ["document"] = "📄", ["book"] = "📘", ["read"] = "📖", ["calendar"] = "📅",
        ["clock"] = "⏰", ["alarm"] = "⏰", ["chart"] = "📊", ["bar-chart"] = "📊", ["line-chart"] = "📈", ["pie-chart"] = "🥧",
        ["dashboard"] = "📊", ["team"] = "👥", ["user"] = "👤", ["users"] = "👥", ["message"] = "💬", ["chat"] = "💬",
        ["mail"] = "✉", ["bell"] = "🔔", ["notification"] = "🔔", ["lock"] = "🔒", ["safety"] = "🛡", ["shield"] = "🛡",
        ["key"] = "🔑", ["search"] = "🔍", ["flag"] = "🚩", ["star"] = "⭐", ["heart"] = "❤", ["fire"] = "🔥", ["thunderbolt"] = "⚡",
        ["bolt"] = "⚡", ["trophy"] = "🏆", ["crown"] = "👑", ["gift"] = "🎁", ["shop"] = "🏪", ["shopping"] = "🛒", ["cart"] = "🛒",
        ["bank"] = "🏦", ["money"] = "💰", ["dollar"] = "💲", ["wallet"] = "👛", ["credit-card"] = "💳", ["home"] = "🏠",
        ["build"] = "🏗", ["building"] = "🏢", ["office"] = "🏢", ["global"] = "🌐", ["globe"] = "🌐", ["environment"] = "🌍",
        ["compass"] = "🧭", ["map"] = "🗺", ["camera"] = "📷", ["picture"] = "🖼", ["image"] = "🖼", ["video"] = "🎬",
        ["sound"] = "🔊", ["music"] = "🎵", ["scan"] = "📠", ["qrcode"] = "🔳", ["audit"] = "📋",
        ["solution"] = "📋", ["project"] = "📂", ["container"] = "📦", ["box"] = "📦", ["inbox"] = "📥", ["send"] = "📤",
        ["link"] = "🔗", ["paper-clip"] = "📎", ["edit"] = "✏", ["pencil"] = "✏", ["highlight"] = "🖍",
        ["medicine-box"] = "💊", ["medicine"] = "💊", ["hospital"] = "🏥", ["health"] = "🩺", ["heart-pulse"] = "💓",
        ["face"] = "🙂", ["smile"] = "😊", ["eye"] = "👁", ["fingerprint"] = "🫆", ["id-card"] = "🪪", ["idcard"] = "🪪",
        // 食衣住行育樂
        ["coffee"] = "☕", ["food"] = "🍱", ["pizza"] = "🍕", ["cake"] = "🍰", ["apple"] = "🍎", ["rice"] = "🍚", ["tea"] = "🍵",
        ["beer"] = "🍺", ["shirt"] = "👕", ["car"] = "🚗", ["bus"] = "🚌", ["train"] = "🚆", ["plane"] = "✈", ["airplane"] = "✈",
        ["bicycle"] = "🚲", ["bike"] = "🚲", ["ship"] = "🚢", ["truck"] = "🚚", ["school"] = "🏫", ["graduation"] = "🎓",
        ["education"] = "🎓", ["game"] = "🎮", ["gamepad"] = "🎮", ["puzzle"] = "🧩", ["palette"] = "🎨", ["art"] = "🎨",
        ["movie"] = "🎬", ["ticket"] = "🎫", ["party"] = "🎉", ["celebration"] = "🎉",
        // 運動、動物、植物、天氣、節慶
        ["sport"] = "🏅", ["soccer"] = "⚽", ["basketball"] = "🏀", ["baseball"] = "⚾", ["tennis"] = "🎾", ["run"] = "🏃",
        ["swim"] = "🏊", ["dumbbell"] = "🏋", ["fitness"] = "🏋", ["yoga"] = "🧘", ["cat"] = "🐱", ["dog"] = "🐶", ["fish"] = "🐟",
        ["bird"] = "🐦", ["rabbit"] = "🐰", ["panda"] = "🐼", ["bear"] = "🐻", ["paw"] = "🐾", ["tree"] = "🌳", ["leaf"] = "🍃",
        ["flower"] = "🌸", ["plant"] = "🌱", ["seedling"] = "🌱", ["cactus"] = "🌵", ["sun"] = "☀", ["moon"] = "🌙",
        ["rain"] = "🌧", ["snow"] = "❄", ["snowflake"] = "❄", ["rainbow"] = "🌈", ["umbrella"] = "☂", ["wind"] = "🌬",
        ["spring"] = "🌸", ["summer"] = "🏖", ["autumn"] = "🍂", ["winter"] = "⛄", ["christmas"] = "🎄", ["lantern"] = "🏮",
        ["fireworks"] = "🎆", ["pumpkin"] = "🎃", ["red-envelope"] = "🧧", ["mooncake"] = "🥮",
    };

    /// <summary>Hayaku 的 18 種專案顏色；不在表裡的名稱用雜湊挑一個，至少同一個專案顏色固定。</summary>
    static readonly Dictionary<string, Color> Colors = new(StringComparer.OrdinalIgnoreCase)
    {
        ["moss"] = C("#6B8E4E"), ["ochre"] = C("#C8922E"), ["rose"] = C("#C5577A"), ["stone"] = C("#7D7A73"),
        ["slate"] = C("#5B6B7F"), ["sky"] = C("#3E8FD0"), ["ocean"] = C("#2E6E9E"), ["teal"] = C("#2A9D8F"),
        ["sage"] = C("#8AA586"), ["forest"] = C("#2F6B3E"), ["indigo"] = C("#4F5BB5"), ["violet"] = C("#7E57C2"),
        ["plum"] = C("#8E4585"), ["coral"] = C("#E06C5A"), ["clay"] = C("#B5654A"), ["amber"] = C("#E0A526"),
        ["sand"] = C("#C9AE7C"), ["ink"] = C("#3A3F4B"), ["red"] = C("#D04545"), ["orange"] = C("#E07B2E"),
        ["yellow"] = C("#D9B42B"), ["green"] = C("#3F9D57"), ["blue"] = C("#3B7DD8"), ["purple"] = C("#8152C7"),
        ["pink"] = C("#D85C9A"), ["gray"] = C("#808080"), ["grey"] = C("#808080"), ["brown"] = C("#8B5E3C"),
    };

    static readonly Color[] Fallback = [.. Colors.Values.Take(18)];

    static Color C(string hex) => (Color)ColorConverter.ConvertFromString(hex);

    /// <summary>圖示的 emoji；對不到時用專案名稱的第一個字（跳過「F2P-」這類代碼前綴）。</summary>
    public static string Glyph(string? icon, string name)
    {
        if (icon is not null)
        {
            if (Glyphs.TryGetValue(icon, out var g))
                return g;
            // 「rocket-outlined」「icon-rocket」這類變化
            var bare = icon.Replace("-outlined", "").Replace("-filled", "").Replace("icon-", "");
            if (Glyphs.TryGetValue(bare, out g))
                return g;
        }
        var dash = name.IndexOf('-');
        var text = dash is > 0 and < 12 && dash + 1 < name.Length ? name[(dash + 1)..] : name;
        return text.Length > 0 ? text[..1].ToUpperInvariant() : "?";
    }

    public static Color ColorOf(string? color, string key)
    {
        if (color is not null && Colors.TryGetValue(color, out var c))
            return c;
        var hash = 0;
        foreach (var ch in color ?? key)
            hash = hash * 31 + ch;
        return Fallback[Math.Abs(hash % Fallback.Length)];
    }

    /// <summary>專案圖示＋名稱（下拉選單、列表用）。</summary>
    public static FrameworkElement Badge(ProjectInfo? project, string name, double size = 18)
    {
        var panel = new StackPanel { Orientation = Orientation.Horizontal };
        panel.Children.Add(Icon(project, name, size));
        panel.Children.Add(new TextBlock { Text = name, VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(6, 0, 0, 0),
            TextTrimming = TextTrimming.CharacterEllipsis });
        return panel;
    }

    /// <summary>圓角色塊裡放 emoji 或字。</summary>
    public static FrameworkElement Icon(ProjectInfo? project, string name, double size = 18)
    {
        var color = ColorOf(project?.IconColor, project?.Key ?? name);
        var glyph = Glyph(project?.Icon, project?.Name ?? name);
        return new Border
        {
            Width = size,
            Height = size,
            CornerRadius = new CornerRadius(size / 4),
            Background = new SolidColorBrush(Color.FromArgb(0x33, color.R, color.G, color.B)),
            BorderBrush = new SolidColorBrush(color),
            BorderThickness = new Thickness(1),
            VerticalAlignment = VerticalAlignment.Center,
            Child = new TextBlock
            {
                Text = glyph,
                FontSize = size * 0.58,
                Foreground = new SolidColorBrush(color),
                FontWeight = FontWeights.SemiBold,
                HorizontalAlignment = HorizontalAlignment.Center,
                VerticalAlignment = VerticalAlignment.Center,
                FontFamily = new FontFamily("Segoe UI Emoji, Microsoft JhengHei UI"),
            },
        };
    }
}
