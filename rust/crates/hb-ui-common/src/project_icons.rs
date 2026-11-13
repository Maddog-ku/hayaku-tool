//! 對應 C# `UI/ProjectIcons.cs`：Hayaku 專案圖示（名稱，例 rocket）→ emoji，顏色名稱（例 moss）→ 色塊。
//! Hayaku 有 500 多個圖示，常用的對到 emoji；對不到的用專案名稱第一個字。
//!
//! 畫面上用 `@hbc` 的 `ProjectIcon`（圓角色塊裡放 emoji 或字）／`ProjectBadge`（圖示＋名稱），
//! 資料用 [`ProjectIcons::icon`] 算出 [`ProjectIconData`]（glyph 與顏色）。

use hb_api::ProjectInfo;

/// 圖示名稱 → emoji（比對不分大小寫）。
const GLYPHS: &[(&str, &str)] = &[
    // 工作、工具
    ("rocket", "🚀"),
    ("robot", "🤖"),
    ("tool", "🔧"),
    ("tools", "🛠"),
    ("wrench", "🔧"),
    ("hammer", "🔨"),
    ("experiment", "🧪"),
    ("flask", "🧪"),
    ("bulb", "💡"),
    ("idea", "💡"),
    ("code", "💻"),
    ("laptop", "💻"),
    ("desktop", "🖥"),
    ("mobile", "📱"),
    ("phone", "📱"),
    ("printer", "🖨"),
    ("database", "🗄"),
    ("cloud", "☁"),
    ("server", "🗄"),
    ("api", "🔌"),
    ("plug", "🔌"),
    ("bug", "🐞"),
    ("setting", "⚙"),
    ("settings", "⚙"),
    ("gear", "⚙"),
    ("folder", "📁"),
    ("file", "📄"),
    ("document", "📄"),
    ("book", "📘"),
    ("read", "📖"),
    ("calendar", "📅"),
    ("clock", "⏰"),
    ("alarm", "⏰"),
    ("chart", "📊"),
    ("bar-chart", "📊"),
    ("line-chart", "📈"),
    ("pie-chart", "🥧"),
    ("dashboard", "📊"),
    ("team", "👥"),
    ("user", "👤"),
    ("users", "👥"),
    ("message", "💬"),
    ("chat", "💬"),
    ("mail", "✉"),
    ("bell", "🔔"),
    ("notification", "🔔"),
    ("lock", "🔒"),
    ("safety", "🛡"),
    ("shield", "🛡"),
    ("key", "🔑"),
    ("search", "🔍"),
    ("flag", "🚩"),
    ("star", "⭐"),
    ("heart", "❤"),
    ("fire", "🔥"),
    ("thunderbolt", "⚡"),
    ("bolt", "⚡"),
    ("trophy", "🏆"),
    ("crown", "👑"),
    ("gift", "🎁"),
    ("shop", "🏪"),
    ("shopping", "🛒"),
    ("cart", "🛒"),
    ("bank", "🏦"),
    ("money", "💰"),
    ("dollar", "💲"),
    ("wallet", "👛"),
    ("credit-card", "💳"),
    ("home", "🏠"),
    ("build", "🏗"),
    ("building", "🏢"),
    ("office", "🏢"),
    ("global", "🌐"),
    ("globe", "🌐"),
    ("environment", "🌍"),
    ("compass", "🧭"),
    ("map", "🗺"),
    ("camera", "📷"),
    ("picture", "🖼"),
    ("image", "🖼"),
    ("video", "🎬"),
    ("sound", "🔊"),
    ("music", "🎵"),
    ("scan", "📠"),
    ("qrcode", "🔳"),
    ("audit", "📋"),
    ("solution", "📋"),
    ("project", "📂"),
    ("container", "📦"),
    ("box", "📦"),
    ("inbox", "📥"),
    ("send", "📤"),
    ("link", "🔗"),
    ("paper-clip", "📎"),
    ("edit", "✏"),
    ("pencil", "✏"),
    ("highlight", "🖍"),
    ("medicine-box", "💊"),
    ("medicine", "💊"),
    ("hospital", "🏥"),
    ("health", "🩺"),
    ("heart-pulse", "💓"),
    ("face", "🙂"),
    ("smile", "😊"),
    ("eye", "👁"),
    ("fingerprint", "🫆"),
    ("id-card", "🪪"),
    ("idcard", "🪪"),
    // 食衣住行育樂
    ("coffee", "☕"),
    ("food", "🍱"),
    ("pizza", "🍕"),
    ("cake", "🍰"),
    ("apple", "🍎"),
    ("rice", "🍚"),
    ("tea", "🍵"),
    ("beer", "🍺"),
    ("shirt", "👕"),
    ("car", "🚗"),
    ("bus", "🚌"),
    ("train", "🚆"),
    ("plane", "✈"),
    ("airplane", "✈"),
    ("bicycle", "🚲"),
    ("bike", "🚲"),
    ("ship", "🚢"),
    ("truck", "🚚"),
    ("school", "🏫"),
    ("graduation", "🎓"),
    ("education", "🎓"),
    ("game", "🎮"),
    ("gamepad", "🎮"),
    ("puzzle", "🧩"),
    ("palette", "🎨"),
    ("art", "🎨"),
    ("movie", "🎬"),
    ("ticket", "🎫"),
    ("party", "🎉"),
    ("celebration", "🎉"),
    // 運動、動物、植物、天氣、節慶
    ("sport", "🏅"),
    ("soccer", "⚽"),
    ("basketball", "🏀"),
    ("baseball", "⚾"),
    ("tennis", "🎾"),
    ("run", "🏃"),
    ("swim", "🏊"),
    ("dumbbell", "🏋"),
    ("fitness", "🏋"),
    ("yoga", "🧘"),
    ("cat", "🐱"),
    ("dog", "🐶"),
    ("fish", "🐟"),
    ("bird", "🐦"),
    ("rabbit", "🐰"),
    ("panda", "🐼"),
    ("bear", "🐻"),
    ("paw", "🐾"),
    ("tree", "🌳"),
    ("leaf", "🍃"),
    ("flower", "🌸"),
    ("plant", "🌱"),
    ("seedling", "🌱"),
    ("cactus", "🌵"),
    ("sun", "☀"),
    ("moon", "🌙"),
    ("rain", "🌧"),
    ("snow", "❄"),
    ("snowflake", "❄"),
    ("rainbow", "🌈"),
    ("umbrella", "☂"),
    ("wind", "🌬"),
    ("spring", "🌸"),
    ("summer", "🏖"),
    ("autumn", "🍂"),
    ("winter", "⛄"),
    ("christmas", "🎄"),
    ("lantern", "🏮"),
    ("fireworks", "🎆"),
    ("pumpkin", "🎃"),
    ("red-envelope", "🧧"),
    ("mooncake", "🥮"),
];

/// Hayaku 的 18 種專案顏色（前 18 個是雜湊挑選的備用色，順序跟 C# 一樣）；不在表裡的名稱用雜湊挑一個，至少同一個專案顏色固定。
const COLORS: &[(&str, u32)] = &[
    ("moss", 0x6B8E4E),
    ("ochre", 0xC8922E),
    ("rose", 0xC5577A),
    ("stone", 0x7D7A73),
    ("slate", 0x5B6B7F),
    ("sky", 0x3E8FD0),
    ("ocean", 0x2E6E9E),
    ("teal", 0x2A9D8F),
    ("sage", 0x8AA586),
    ("forest", 0x2F6B3E),
    ("indigo", 0x4F5BB5),
    ("violet", 0x7E57C2),
    ("plum", 0x8E4585),
    ("coral", 0xE06C5A),
    ("clay", 0xB5654A),
    ("amber", 0xE0A526),
    ("sand", 0xC9AE7C),
    ("ink", 0x3A3F4B),
    ("red", 0xD04545),
    ("orange", 0xE07B2E),
    ("yellow", 0xD9B42B),
    ("green", 0x3F9D57),
    ("blue", 0x3B7DD8),
    ("purple", 0x8152C7),
    ("pink", 0xD85C9A),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("brown", 0x8B5E3C),
];

const FALLBACK_COUNT: usize = 18;

/// 不透明的 RGB 顏色（C# `Color`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Rgb {
    const fn from_hex(hex: u32) -> Self {
        Self { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 }
    }

    pub fn to_slint(self) -> slint::Color {
        slint::Color::from_rgb_u8(self.r, self.g, self.b)
    }
}

/// 專案圖示的內容：色塊裡的 emoji 或字、顏色（底色是同色 0x33 透明度、框與字是原色）。
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectIconData {
    pub glyph: String,
    pub color: Rgb,
}

pub struct ProjectIcons;

fn lookup_glyph(icon: &str) -> Option<&'static str> {
    GLYPHS.iter().find(|(k, _)| k.eq_ignore_ascii_case(icon)).map(|(_, g)| *g)
}

impl ProjectIcons {
    /// 圖示的 emoji；對不到時用專案名稱的第一個字（跳過「F2P-」這類代碼前綴）。
    pub fn glyph(icon: Option<&str>, name: &str) -> String {
        if let Some(icon) = icon {
            if let Some(g) = lookup_glyph(icon) {
                return g.to_owned();
            }
            // 「rocket-outlined」「icon-rocket」這類變化
            let bare = icon.replace("-outlined", "").replace("-filled", "").replace("icon-", "");
            if let Some(g) = lookup_glyph(&bare) {
                return g.to_owned();
            }
        }
        // C# 的 IndexOf／Length 以 UTF-16 計
        let name_len: usize = name.encode_utf16().count();
        let dash = name.find('-').map(|b| name[..b].encode_utf16().count());
        let text = match (dash, name.find('-')) {
            (Some(d), Some(b)) if d > 0 && d < 12 && d + 1 < name_len => &name[b + 1..],
            _ => name,
        };
        match text.chars().next() {
            Some(c) => c.to_uppercase().collect(),
            None => "?".to_owned(),
        }
    }

    pub fn color_of(color: Option<&str>, key: &str) -> Rgb {
        if let Some(c) = color
            && let Some((_, hex)) = COLORS.iter().find(|(k, _)| k.eq_ignore_ascii_case(c))
        {
            return Rgb::from_hex(*hex);
        }
        let mut hash: i32 = 0;
        for ch in color.unwrap_or(key).encode_utf16() {
            hash = hash.wrapping_mul(31).wrapping_add(ch as i32);
        }
        let index = (hash % FALLBACK_COUNT as i32).unsigned_abs() as usize;
        Rgb::from_hex(COLORS[index].1)
    }

    /// 圓角色塊裡放 emoji 或字（C# `Icon`）：顏色依專案的 IconColor（沒有用專案編號／名稱雜湊），字依專案圖示。
    pub fn icon(project: Option<&ProjectInfo>, name: &str) -> ProjectIconData {
        let color =
            Self::color_of(project.and_then(|p| p.icon_color.as_deref()), project.map_or(name, |p| p.key.as_str()));
        let glyph = Self::glyph(project.and_then(|p| p.icon.as_deref()), project.map_or(name, |p| p.name.as_str()));
        ProjectIconData { glyph, color }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_圖示對到_emoji_不分大小寫與變化() {
        assert_eq!("🚀", ProjectIcons::glyph(Some("rocket"), "X"));
        assert_eq!("🚀", ProjectIcons::glyph(Some("Rocket"), "X"));
        assert_eq!("🚀", ProjectIcons::glyph(Some("rocket-outlined"), "X"));
        assert_eq!("🚀", ProjectIcons::glyph(Some("icon-rocket"), "X"));
    }

    #[test]
    fn glyph_對不到用名稱第一個字_跳過代碼前綴() {
        assert_eq!("臉", ProjectIcons::glyph(Some("unknown-icon"), "F2P-臉部辨識結合印表機"));
        assert_eq!("H", ProjectIcons::glyph(None, "hayaku"));
        assert_eq!("?", ProjectIcons::glyph(None, ""));
        // 「-」在最前面或結尾不算前綴
        assert_eq!("-", ProjectIcons::glyph(None, "-abc"));
        assert_eq!("A", ProjectIcons::glyph(None, "abc-"));
    }

    #[test]
    fn color_of_表裡的顏色與雜湊備用色固定() {
        assert_eq!(Rgb { r: 0x6B, g: 0x8E, b: 0x4E }, ProjectIcons::color_of(Some("Moss"), "X"));
        let a = ProjectIcons::color_of(None, "F2P");
        assert_eq!(a, ProjectIcons::color_of(None, "F2P"));
        // 備用色只從前 18 個挑
        let first18: Vec<Rgb> = COLORS[..18].iter().map(|(_, h)| Rgb::from_hex(*h)).collect();
        for key in ["F2P", "REVIVAL", "EG", "很長很長的專案名稱 abc", ""] {
            assert!(first18.contains(&ProjectIcons::color_of(None, key)));
        }
        // C#：hash = "AB" → 65*31+66 = 2081，2081 % 18 = 11 → violet
        assert_eq!(Rgb::from_hex(0x7E57C2), ProjectIcons::color_of(None, "AB"));
    }
}
