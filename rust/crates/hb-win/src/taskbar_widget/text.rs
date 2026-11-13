//! 工作列小工具的文字：字型載入、逐字選字型（數字 Space Grotesk → Inter → 微軟正黑體 → Segoe UI）、
//! 量寬度、把字形畫進 tiny-skia 的 Pixmap（premultiplied）。

use std::sync::OnceLock;

use ab_glyph::{Font, FontArc, FontVec, GlyphId, PxScale, point};
use tiny_skia::{Pixmap, PremultipliedColorU8};

static SG_BOLD: &[u8] = include_bytes!("../../../hb-ui-kit/ui/fonts/SpaceGrotesk-Bold.ttf");
static SG_MEDIUM: &[u8] = include_bytes!("../../../hb-ui-kit/ui/fonts/SpaceGrotesk-Medium.ttf");
static INTER_MEDIUM: &[u8] = include_bytes!("../../../hb-ui-kit/ui/fonts/Inter-Medium.ttf");
static INTER_SEMIBOLD: &[u8] = include_bytes!("../../../hb-ui-kit/ui/fonts/Inter-SemiBold.ttf");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Weight {
    Medium,
    Bold,
}

struct Fonts {
    medium: Vec<FontArc>,
    bold: Vec<FontArc>,
}

fn system_font(file: &str, index: u32) -> Option<FontArc> {
    let dir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
    let path = std::path::Path::new(&dir).join("Fonts").join(file);
    let data = std::fs::read(&path).ok()?;
    match FontVec::try_from_vec_and_index(data, index) {
        Ok(f) => Some(FontArc::new(f)),
        Err(e) => {
            log::error!("讀取字型 {} 失敗：{e}", path.display());
            None
        }
    }
}

fn embedded(data: &'static [u8]) -> Option<FontArc> {
    FontArc::try_from_slice(data).ok()
}

fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| {
        // 中文用系統的微軟正黑體（msjh.ttc 第 0 個是 Microsoft JhengHei）；讀不到就退回 Segoe UI
        let cjk = system_font("msjh.ttc", 0);
        let cjk_bold = system_font("msjhbd.ttc", 0).or_else(|| cjk.clone());
        let segoe = system_font("segoeui.ttf", 0);
        let segoe_bold = system_font("segoeuib.ttf", 0).or_else(|| segoe.clone());
        if cjk.is_none() {
            log::info!("找不到微軟正黑體，中文改用 Segoe UI");
        }
        let medium = [embedded(SG_MEDIUM), embedded(INTER_MEDIUM), cjk, segoe].into_iter().flatten().collect();
        let bold = [embedded(SG_BOLD), embedded(INTER_SEMIBOLD), cjk_bold, segoe_bold].into_iter().flatten().collect();
        Fonts { medium, bold }
    })
}

fn chain(weight: Weight) -> &'static [FontArc] {
    match weight {
        Weight::Medium => &fonts().medium,
        Weight::Bold => &fonts().bold,
    }
}

/// 一個字：用哪個字型、字形編號、前進寬度（px）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct ShapedChar {
    font: usize,
    id: GlyphId,
    pub advance: f32,
}

fn em_factor(font: &FontArc, size: f32) -> f32 {
    size / font.units_per_em().unwrap_or(1000.0)
}

/// 一個字的字形與寬度（size 是 em 大小，px）。
pub(crate) fn shape_char(ch: char, size: f32, weight: Weight) -> ShapedChar {
    let fonts = chain(weight);
    let found = fonts.iter().enumerate().find(|(_, f)| f.glyph_id(ch).0 != 0);
    match found {
        Some((i, f)) => {
            let id = f.glyph_id(ch);
            ShapedChar { font: i, id, advance: f.h_advance_unscaled(id) * em_factor(f, size) }
        }
        None => {
            // 每個字型都沒有：用第一個字型的 .notdef（空白寬度）
            let advance = fonts.first().map_or(size * 0.5, |f| f.h_advance_unscaled(GlyphId(0)) * em_factor(f, size));
            ShapedChar { font: 0, id: GlyphId(0), advance }
        }
    }
}

pub(crate) fn shape(text: &str, size: f32, weight: Weight) -> Vec<ShapedChar> {
    text.chars().map(|c| shape_char(c, size, weight)).collect()
}

pub(crate) fn width(chars: &[ShapedChar]) -> f32 {
    chars.iter().map(|c| c.advance).sum()
}

/// 大寫字母高度（px）：用來把數字垂直置中。
pub(crate) fn cap_height(size: f32, weight: Weight) -> f32 {
    let Some(font) = chain(weight).first() else {
        return size * 0.7;
    };
    let id = font.glyph_id('H');
    let scale = PxScale::from(size * font.height_unscaled() / font.units_per_em().unwrap_or(1000.0));
    match font.outline_glyph(id.with_scale_and_position(scale, point(0.0, 0.0))) {
        Some(g) => {
            let b = g.px_bounds();
            -b.min.y
        }
        None => size * 0.7,
    }
}

/// 直接（非 premultiplied）的 RGBA，0～1。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::rgba(r, g, b, 255)
    }

    #[allow(clippy::self_named_constructors)]
    pub const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
    }

    pub fn with_alpha(self, a: f32) -> Self {
        Self { a: self.a * a, ..self }
    }

    pub fn to_skia(self) -> tiny_skia::Color {
        tiny_skia::Color::from_rgba(self.r, self.g, self.b, self.a.clamp(0.0, 1.0)).unwrap_or(tiny_skia::Color::BLACK)
    }
}

/// 把一個像素用 source-over 疊上去（color 是直接色，coverage 0～1）。
fn blend(pixel: &mut PremultipliedColorU8, color: Rgba, coverage: f32) {
    let a = (color.a * coverage).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let mix = |src: f32, dst: u8| (src * a * 255.0 + dst as f32 * inv).round().clamp(0.0, 255.0);
    let na = (a * 255.0 + pixel.alpha() as f32 * inv).round().clamp(0.0, 255.0);
    let nr = mix(color.r, pixel.red()).min(na);
    let ng = mix(color.g, pixel.green()).min(na);
    let nb = mix(color.b, pixel.blue()).min(na);
    if let Some(p) = PremultipliedColorU8::from_rgba(nr as u8, ng as u8, nb as u8, na as u8) {
        *pixel = p;
    }
}

/// 畫一個字。`baseline` 是基線 y；`clip` 是可見的 y 範圍（數字滾輪的格子）；`gamma` < 1 讓淺色字在深底上不會太細。
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_char(
    pixmap: &mut Pixmap,
    c: &ShapedChar,
    size: f32,
    weight: Weight,
    x: f32,
    baseline: f32,
    color: Rgba,
    clip: Option<(f32, f32)>,
    gamma: f32,
) {
    if color.a <= 0.0 || c.id.0 == 0 {
        return;
    }
    let Some(font) = chain(weight).get(c.font) else {
        return;
    };
    let scale = PxScale::from(size * font.height_unscaled() / font.units_per_em().unwrap_or(1000.0));
    let Some(outline) = font.outline_glyph(c.id.with_scale_and_position(scale, point(x, baseline))) else {
        return;
    };
    let bounds = outline.px_bounds();
    let (w, h) = (pixmap.width() as i32, pixmap.height() as i32);
    let (clip_top, clip_bottom) = clip.unwrap_or((f32::MIN, f32::MAX));
    let pixels = pixmap.pixels_mut();
    outline.draw(|gx, gy, cov| {
        let px = bounds.min.x as i32 + gx as i32;
        let py = bounds.min.y as i32 + gy as i32;
        if px < 0 || py < 0 || px >= w || py >= h {
            return;
        }
        // 格子邊緣半像素也算部分覆蓋，滑動時邊緣比較柔
        let top = (clip_top - py as f32).clamp(0.0, 1.0);
        let bottom = (py as f32 + 1.0 - clip_bottom).clamp(0.0, 1.0);
        let visible = (1.0 - top - bottom).max(0.0);
        if visible <= 0.0 {
            return;
        }
        let cov = cov.clamp(0.0, 1.0).powf(gamma) * visible;
        blend(&mut pixels[(py * w + px) as usize], color, cov);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 字型都載得到() {
        assert!(chain(Weight::Bold).len() >= 2);
        assert!(chain(Weight::Medium).len() >= 2);
        // 數字與 h 用 Space Grotesk（第 0 個）
        for ch in "0123456789.h/+".chars() {
            assert_eq!(shape_char(ch, 14.0, Weight::Bold).font, 0, "{ch}");
        }
        assert!(cap_height(14.0, Weight::Bold) > 8.0 && cap_height(14.0, Weight::Bold) < 12.0);
    }

    #[test]
    fn 中文退到系統字型() {
        let c = shape_char('同', 14.0, Weight::Bold);
        assert!(c.font >= 2, "中文不應該用 Space Grotesk／Inter");
        assert!(c.advance > 10.0);
    }
}
