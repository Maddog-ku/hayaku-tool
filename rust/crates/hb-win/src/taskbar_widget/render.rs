//! 工作列小工具的繪製（對應 C# `TaskbarWidget.Render`）。外觀改成新設計（微互動元件牆設計稿）：
//! 黑／白／橘（#FF5B22）；數字用 Space Grotesk；工作列深色時文字白、淺色時文字黑；進度條橘色；
//! 計時中＝橘色狀態點加光暈＋橘色膠囊計數；Claude 標記是四芒星；未讀是紅色圓角徽章。
//! 版面（一排／兩排、各元素的順序與間距算法）照 C#。

use tiny_skia::{FillRule, Paint, Path, PathBuilder, Pixmap, Rect, Shader, Transform};

use super::roller::Roller;
use super::text::{self, Rgba, Weight};
use super::{WidgetContent, WidgetStatus};

const INK: Rgba = Rgba::rgb(0x0E, 0x0F, 0x13);
const WHITE: Rgba = Rgba::rgb(255, 255, 255);
const ACCENT: Rgba = Rgba::rgb(0xFF, 0x5B, 0x22);
const CLAUDE: Rgba = Rgba::rgb(217, 119, 87);
const ERROR: Rgba = Rgba::rgb(232, 84, 72);

/// 會滾動的四段文字。
#[derive(Default)]
pub(crate) struct Rollers {
    pub total: Roller,
    pub expected: Roller,
    pub running: Roller,
    pub unread: Roller,
}

impl Rollers {
    pub fn set(&mut self, content: &WidgetContent, now: f64, animate: bool) {
        self.total.set(&content.total, now, animate);
        self.expected.set(&content.expected, now, animate);
        self.running.set(&running_text(content.running), now, animate);
        self.unread.set(&unread_text(content.unread), now, animate);
    }

    pub fn active(&self, now: f64) -> bool {
        [&self.total, &self.expected, &self.running, &self.unread].iter().any(|r| r.active(now))
    }

    pub fn settle(&mut self, now: f64) {
        for r in [&mut self.total, &mut self.expected, &mut self.running, &mut self.unread] {
            r.settle(now);
        }
    }
}

pub(crate) fn running_text(running: i32) -> String {
    if running > 0 { running.to_string() } else { String::new() }
}

/// 未讀數：超過 9 顯示「9+」。
pub(crate) fn unread_text(unread: i32) -> String {
    if unread > 9 {
        "9+".into()
    } else if unread > 0 {
        unread.to_string()
    } else {
        String::new()
    }
}

pub(crate) struct Visual<'a> {
    pub content: &'a WidgetContent,
    pub rollers: &'a Rollers,
    pub now: f64,
    pub scale: f64,
    pub height: i32,
    pub two_line: bool,
    pub dark: bool,
    pub hover: bool,
    pub down: bool,
}

struct Sizes {
    total: f32,
    expected: f32,
    small: f32,
    badge_font: f32,
    pad: f32,
    dot: f32,
    gap: f32,
    bar_h: f32,
    min_bar: f32,
    one_line_bar: f32,
    sparkle: f32,
    badge_h: f32,
}

impl Sizes {
    fn new(s: f32, two: bool) -> Self {
        let pick = |one: f32, two_v: f32| if two { two_v * s } else { one * s };
        Self {
            total: pick(15.0, 13.0),
            expected: pick(12.5, 11.0),
            small: pick(11.5, 10.5),
            badge_font: 9.0 * s,
            pad: 8.0 * s,
            dot: pick(8.0, 6.0),
            gap: pick(7.0, 5.0),
            bar_h: pick(4.0, 3.0),
            min_bar: 36.0 * s,
            one_line_bar: 44.0 * s,
            sparkle: pick(12.0, 10.0),
            badge_h: 13.0 * s,
        }
    }
}

fn text_width(s: &str, size: f32, weight: Weight) -> f32 {
    text::width(&text::shape(s, size, weight))
}

/// 計時中膠囊：左右內距、圓點直徑、圓點與數字間距。
fn chip_metrics(z: &Sizes, s: f32) -> (f32, f32, f32) {
    (3.5 * s, z.small * 0.38, 2.5 * s)
}

/// 量寬度（實體像素），用最終文字（動畫中寬度不變）。
fn layout(v: &Visual, z: &Sizes) -> (f32, f32, f32, f32, f32, f32) {
    let s = v.scale as f32;
    let r = v.rollers;
    let total_w = text_width(r.total.text(), z.total, Weight::Bold);
    let expected_w = if r.expected.text().is_empty() {
        0.0
    } else {
        text_width(" ", z.expected, Weight::Medium) + text_width(r.expected.text(), z.expected, Weight::Medium)
    };
    let run_w = if r.running.text().is_empty() {
        0.0
    } else {
        let (inner, dot, gap) = chip_metrics(z, s);
        inner + dot + gap + text_width(r.running.text(), z.small, Weight::Bold) + inner
    };
    let claude_w = if v.content.claude { z.sparkle } else { 0.0 };
    let badge_w = if r.unread.text().is_empty() {
        0.0
    } else {
        z.badge_h.max(text_width(r.unread.text(), z.badge_font, Weight::Bold) + 6.0 * s)
    };
    let line1 = z.dot + z.gap + total_w + expected_w;
    let extras = (if run_w > 0.0 { z.gap + run_w } else { 0.0 })
        + (if claude_w > 0.0 { z.gap * 0.6 + claude_w } else { 0.0 })
        + (if badge_w > 0.0 { z.gap * 0.7 + badge_w } else { 0.0 });
    let (bar_w, content_w) = if v.two_line {
        // 兩排：上面「3.4h / 8h」，下面進度條＋計時數＋Claude
        let content = line1.max(z.min_bar + extras);
        (content - extras, content)
    } else {
        // 一排：● 3.4h / 8h ▬▬▬ ●6 ✦
        (z.one_line_bar, line1 + z.gap + z.one_line_bar + extras)
    };
    (content_w, bar_w, run_w, claude_w, badge_w, total_w)
}

pub(crate) fn render(v: &Visual) -> Option<Pixmap> {
    let s = v.scale as f32;
    let z = Sizes::new(s, v.two_line);
    let (content_w, bar_w, run_w, claude_w, badge_w, total_w) = layout(v, &z);
    let width = (z.pad + content_w + z.pad).ceil().max(1.0) as u32;
    let height = v.height.max(1) as u32;
    let mut pm = Pixmap::new(width, height)?;
    let (wf, hf) = (width as f32, height as f32);

    let text_color = if v.dark { Rgba::rgba(255, 255, 255, 245) } else { INK };
    let muted = if v.dark { Rgba::rgba(255, 255, 255, 150) } else { INK.with_alpha(140.0 / 255.0) };
    let track = if v.dark { Rgba::rgba(255, 255, 255, 70) } else { INK.with_alpha(50.0 / 255.0) };
    let gamma = if v.dark { 0.8 } else { 1.0 };

    // alpha 1 的底色：看不見，但讓整塊都點得到（alpha 0 的像素會穿透滑鼠）
    pm.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 1));
    if v.hover || v.down {
        let inset = 4.0 * s;
        let a = if v.dark {
            if v.down { 22 } else { 34 }
        } else if v.down {
            18
        } else {
            26
        };
        let c = if v.dark { Rgba::rgba(255, 255, 255, a) } else { Rgba::rgba(0, 0, 0, a) };
        fill(&mut pm, round_rect(1.0, inset, wf - 2.0, hf - inset * 2.0, 4.0 * s), c);
    }

    let cap_total = text::cap_height(z.total, Weight::Bold);
    let line2_h = z.bar_h.max(z.small * 1.4).max(z.sparkle).max(if badge_w > 0.0 { z.badge_h } else { 0.0 });
    let (cy1, cy2) = if v.two_line {
        let line_gap = 7.0 * s;
        let top = (hf - (cap_total + line_gap + line2_h)) / 2.0;
        (top + cap_total / 2.0, top + cap_total + line_gap + line2_h / 2.0)
    } else {
        (hf / 2.0, hf / 2.0)
    };

    // 狀態點＋工時
    let mut x = z.pad;
    let dot_c = (x + z.dot / 2.0, cy1);
    match v.content.status {
        WidgetStatus::Running => {
            // 計時中：橘色實心點＋淡橘光暈
            fill(&mut pm, PathBuilder::from_circle(dot_c.0, dot_c.1, z.dot / 2.0 + 2.5 * s), ACCENT.with_alpha(0.28));
            fill(&mut pm, PathBuilder::from_circle(dot_c.0, dot_c.1, z.dot / 2.0), ACCENT);
        }
        WidgetStatus::Error => fill(&mut pm, PathBuilder::from_circle(dot_c.0, dot_c.1, z.dot / 2.0), ERROR),
        WidgetStatus::Idle => fill(&mut pm, PathBuilder::from_circle(dot_c.0, dot_c.1, z.dot / 2.0), muted),
    }
    x += z.dot + z.gap;
    draw_rolling(&mut pm, &v.rollers.total, z.total, Weight::Bold, x, cy1, text_color, v.now, gamma);
    x += total_w;
    if !v.rollers.expected.text().is_empty() {
        x += text_width(" ", z.expected, Weight::Medium);
        // 預計時數跟工時對齊基線
        let cap_e = text::cap_height(z.expected, Weight::Medium);
        let cy_e = cy1 + cap_total / 2.0 - cap_e / 2.0;
        draw_rolling(&mut pm, &v.rollers.expected, z.expected, Weight::Medium, x, cy_e, muted, v.now, gamma);
        x += text_width(v.rollers.expected.text(), z.expected, Weight::Medium);
    }

    // 進度條＋計時數＋Claude（兩排時換到第二行）
    x = if v.two_line { z.pad } else { x + z.gap };
    fill(&mut pm, round_rect(x, cy2 - z.bar_h / 2.0, bar_w, z.bar_h, z.bar_h / 2.0), track);
    let progress = if v.content.progress.is_nan() { 0.0 } else { v.content.progress.clamp(0.0, 1.0) as f32 };
    let fill_w = bar_w * progress;
    if fill_w >= z.bar_h {
        fill(&mut pm, round_rect(x, cy2 - z.bar_h / 2.0, fill_w, z.bar_h, z.bar_h / 2.0), ACCENT);
    }
    x += bar_w;
    if run_w > 0.0 {
        // 計時中數量：淡橘膠囊＋橘點＋數字
        x += z.gap;
        let (inner, dot, gap) = chip_metrics(&z, s);
        let chip_h = (z.small * 1.4).round();
        fill(
            &mut pm,
            round_rect(x, cy2 - chip_h / 2.0, run_w, chip_h, chip_h / 2.0),
            ACCENT.with_alpha(if v.dark { 0.22 } else { 0.14 }),
        );
        fill(&mut pm, PathBuilder::from_circle(x + inner + dot / 2.0, cy2, dot / 2.0), ACCENT);
        draw_rolling(
            &mut pm,
            &v.rollers.running,
            z.small,
            Weight::Bold,
            x + inner + dot + gap,
            cy2,
            text_color,
            v.now,
            gamma,
        );
        x += run_w;
    }
    if claude_w > 0.0 {
        x += z.gap * 0.6;
        fill(&mut pm, sparkle(x + claude_w / 2.0, cy2, claude_w / 2.0), CLAUDE);
        x += claude_w;
    }
    if badge_w > 0.0 {
        // 未讀通知：紅色圓角徽章
        x += z.gap * 0.7;
        fill(&mut pm, round_rect(x, cy2 - z.badge_h / 2.0, badge_w, z.badge_h, z.badge_h / 2.0), ERROR);
        let tw = text_width(v.rollers.unread.text(), z.badge_font, Weight::Bold);
        draw_rolling(
            &mut pm,
            &v.rollers.unread,
            z.badge_font,
            Weight::Bold,
            x + (badge_w - tw) / 2.0,
            cy2,
            WHITE,
            v.now,
            1.0,
        );
    }
    Some(pm)
}

/// 畫一段會滾動的文字；cy 是字的視覺中心（大寫字高的一半）。
#[allow(clippy::too_many_arguments)]
fn draw_rolling(
    pm: &mut Pixmap,
    roller: &Roller,
    size: f32,
    weight: Weight,
    x: f32,
    cy: f32,
    color: Rgba,
    now: f64,
    gamma: f32,
) {
    let cap = text::cap_height(size, weight);
    let baseline = (cy + cap / 2.0).round();
    // 每格高 1.2em，置中在字的視覺中心
    let slot_h = size * 1.2;
    let clip = (cy - slot_h / 2.0, cy + slot_h / 2.0);
    let mut cx = x;
    for slot in roller.frame(now) {
        let current = text::shape_char(slot.current.ch, size, weight);
        let moving = slot.current.offset != 0.0 || slot.leaving.is_some();
        let clip = moving.then_some(clip);
        if let Some(old) = slot.leaving {
            let shaped = text::shape_char(old.ch, size, weight);
            let y = baseline + old.offset as f32 * slot_h;
            text::draw_char(pm, &shaped, size, weight, cx, y, color.with_alpha(old.opacity as f32), clip, gamma);
        }
        let y = baseline + slot.current.offset as f32 * slot_h;
        text::draw_char(pm, &current, size, weight, cx, y, color.with_alpha(slot.current.opacity as f32), clip, gamma);
        cx += current.advance;
    }
}

fn fill(pm: &mut Pixmap, path: Option<Path>, color: Rgba) {
    let Some(path) = path else { return };
    let paint = Paint { shader: Shader::SolidColor(color.to_skia()), anti_alias: true, ..Default::default() };
    pm.fill_path(&path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn round_rect(x: f32, y: f32, w: f32, h: f32, radius: f32) -> Option<Path> {
    let r = radius.min(w / 2.0).min(h / 2.0);
    if r <= 0.0 {
        return Some(PathBuilder::from_rect(Rect::from_xywh(x, y, w, h)?));
    }
    // 四分之一圓用三次貝茲近似
    let k = r * 0.552_284_8;
    let (l, t, rt, b) = (x, y, x + w, y + h);
    let mut pb = PathBuilder::new();
    pb.move_to(l + r, t);
    pb.line_to(rt - r, t);
    pb.cubic_to(rt - r + k, t, rt, t + r - k, rt, t + r);
    pb.line_to(rt, b - r);
    pb.cubic_to(rt, b - r + k, rt - r + k, b, rt - r, b);
    pb.line_to(l + r, b);
    pb.cubic_to(l + r - k, b, l, b - r + k, l, b - r);
    pb.line_to(l, t + r);
    pb.cubic_to(l, t + r - k, l + r - k, t, l + r, t);
    pb.close();
    pb.finish()
}

/// 四芒星（Claude 標記，C# 用「✦」字元）。
fn sparkle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    let k = r * 0.16;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.quad_to(cx + k, cy - k, cx + r, cy);
    pb.quad_to(cx + k, cy + k, cx, cy + r);
    pb.quad_to(cx - k, cy + k, cx - r, cy);
    pb.quad_to(cx - k, cy - k, cx, cy - r);
    pb.close();
    pb.finish()
}

/// tiny-skia 的 RGBA（premultiplied）轉成 UpdateLayeredWindow 要的 BGRA（premultiplied）。
pub(crate) fn to_bgra(pm: &Pixmap, out: &mut [u8]) {
    for (src, dst) in pm.data().as_chunks::<4>().0.iter().zip(out.as_chunks_mut::<4>().0.iter_mut()) {
        dst[0] = src[2];
        dst[1] = src[1];
        dst[2] = src[0];
        dst[3] = src[3];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(total: &str, expected: &str, running: i32, claude: bool, unread: i32) -> WidgetContent {
        WidgetContent {
            total: total.into(),
            expected: expected.into(),
            progress: 0.4,
            running,
            claude,
            status: WidgetStatus::Running,
            unread,
        }
    }

    fn draw(c: &WidgetContent, two_line: bool) -> Pixmap {
        let mut rollers = Rollers::default();
        rollers.set(c, 0.0, false);
        render(&Visual {
            content: c,
            rollers: &rollers,
            now: 0.0,
            scale: 1.0,
            height: 48,
            two_line,
            dark: true,
            hover: false,
            down: false,
        })
        .expect("pixmap")
    }

    #[test]
    fn 未讀數文字() {
        assert_eq!(unread_text(0), "");
        assert_eq!(unread_text(3), "3");
        assert_eq!(unread_text(10), "9+");
    }

    #[test]
    fn 兩排比一排窄() {
        let c = content("3.4h", "/ 8h", 2, true, 3);
        let one = draw(&c, false);
        let two = draw(&c, true);
        assert!(two.width() < one.width(), "{} vs {}", two.width(), one.width());
        assert_eq!(one.height(), 48);
        // 底色 alpha 1（整塊可點）
        assert_eq!(one.pixel(0, 0).map(|p| p.alpha()), Some(1));
    }

    #[test]
    fn 多出計時數與徽章會變寬() {
        let base = draw(&content("3.4h", "/ 8h", 0, false, 0), false).width();
        let more = draw(&content("3.4h", "/ 8h", 2, true, 12), false).width();
        assert!(more > base);
    }

    /// 把各種狀態畫成 PNG 檢查外觀：`HB_WIN_DUMP=<資料夾> cargo test -p hb-win -- --ignored dump`
    #[test]
    #[ignore]
    fn dump_png() {
        let Ok(dir) = std::env::var("HB_WIN_DUMP") else { return };
        let cases = [
            ("run", content("3.4h", "/ 8h", 2, true, 3)),
            ("big", content("10.5h", "/ 8h", 1, false, 12)),
            ("sync", WidgetContent { total: "同步中".into(), ..Default::default() }),
            ("login", WidgetContent { total: "未登入".into(), status: WidgetStatus::Error, ..Default::default() }),
        ];
        for (name, c) in cases {
            for dark in [true, false] {
                for two_line in [false, true] {
                    let mut rollers = Rollers::default();
                    rollers.set(&c, 0.0, false);
                    let pm = render(&Visual {
                        content: &c,
                        rollers: &rollers,
                        now: 0.0,
                        scale: 1.25,
                        height: 60,
                        two_line,
                        dark,
                        hover: false,
                        down: false,
                    })
                    .expect("pixmap");
                    // 疊在工作列底色上
                    let mut bg = Pixmap::new(pm.width() + 20, pm.height()).expect("bg");
                    bg.fill(if dark {
                        tiny_skia::Color::from_rgba8(32, 32, 32, 255)
                    } else {
                        tiny_skia::Color::from_rgba8(238, 238, 238, 255)
                    });
                    bg.draw_pixmap(10, 0, pm.as_ref(), &tiny_skia::PixmapPaint::default(), Transform::identity(), None);
                    let file = format!(
                        "{dir}/{name}_{}_{}.png",
                        if dark { "dark" } else { "light" },
                        if two_line { "two" } else { "one" }
                    );
                    bg.save_png(file).expect("save");
                }
            }
        }
    }

    #[test]
    fn 中文狀態畫得出來() {
        let pm = draw(&content("同步中", "", 0, false, 0), false);
        // 有畫到不透明的字
        assert!(pm.pixels().iter().any(|p| p.alpha() > 200));
    }
}
