//! 數字滾輪（參考設計稿的 Roller）：每個字元獨立一格，只有變動的字元會滾動。
//! 數值增加時新字從下方進、減少時從上方進；舊字往反方向滑出淡出。由右往左錯開 30ms，模擬機械計數器的進位感。
//! 長度改變時整組重建，以錯開的滑入取代瞬間跳換。

/// 新字滑入：欠阻尼彈簧（阻尼比 0.52、自然頻率 13），約 600ms 靜止。
pub(crate) const ENTER_MS: f64 = 600.0;
/// 舊字滑出：320ms，cubic-bezier(.4,0,.7,.2)。
pub(crate) const LEAVE_MS: f64 = 320.0;
/// 相鄰字元錯開。
pub(crate) const STAGGER_MS: f64 = 30.0;

/// 彈簧位移（t 秒），0 → 1，會略微超過 1 再回來。
pub(crate) fn spring(t: f64) -> f64 {
    let (z, w) = (0.52_f64, 13.0_f64);
    let wd = w * (1.0 - z * z).sqrt();
    1.0 - (-z * w * t).exp() * ((wd * t).cos() + (z * w / wd) * (wd * t).sin())
}

/// CSS cubic-bezier(x1, y1, x2, y2) 在 x 時的 y。
pub(crate) fn cubic_bezier(x1: f64, y1: f64, x2: f64, y2: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let bez = |a: f64, b: f64, t: f64| {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * a + 3.0 * mt * t * t * b + t * t * t
    };
    // 二分法找 t 使 x(t) = x
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = (lo + hi) / 2.0;
        if bez(x1, x2, mid) < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(y1, y2, (lo + hi) / 2.0)
}

#[derive(Debug, Clone)]
struct Motion {
    start: f64,
    delay: f64,
    dir: f64,
}

#[derive(Debug, Clone)]
struct Slot {
    ch: char,
    enter: Option<Motion>,
    leave: Option<(char, Motion)>,
}

/// 某一格在某時刻要畫的內容：字元、垂直位移（格高的倍數，正數往下）、不透明度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Glyph {
    pub ch: char,
    pub offset: f64,
    pub opacity: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SlotFrame {
    pub current: Glyph,
    pub leaving: Option<Glyph>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct Roller {
    val: Option<String>,
    slots: Vec<Slot>,
}

/// 取出字串裡第一段數字（含小數點），比大小決定滾動方向。
fn number(s: &str) -> Option<f64> {
    let digits: String =
        s.chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    digits.trim_end_matches('.').parse().ok()
}

impl Roller {
    pub fn text(&self) -> &str {
        self.val.as_deref().unwrap_or("")
    }

    /// 換成新字串。`animate` = false（第一次或系統關閉動畫）時直接換。
    pub fn set(&mut self, s: &str, now: f64, animate: bool) {
        let prev = self.val.clone();
        if prev.as_deref() == Some(s) {
            return;
        }
        let dir = match (prev.as_deref().and_then(number), number(s)) {
            (Some(a), Some(b)) if b < a => -1.0,
            _ => 1.0,
        };
        let animate = animate && prev.is_some();
        let new_chars: Vec<char> = s.chars().collect();
        let prev_chars: Vec<char> = prev.as_deref().unwrap_or("").chars().collect();
        if !animate || prev_chars.len() != new_chars.len() {
            // 長度改變時整組重建，並以錯開的滑入取代瞬間跳換
            self.slots = new_chars
                .iter()
                .enumerate()
                .map(|(i, &ch)| Slot {
                    ch,
                    enter: animate.then_some(Motion { start: now, delay: i as f64 * STAGGER_MS, dir }),
                    leave: None,
                })
                .collect();
        } else {
            let len = new_chars.len();
            for (i, &ch) in new_chars.iter().enumerate() {
                if ch == prev_chars[i] {
                    continue;
                }
                let slot = &mut self.slots[i];
                // 由右往左錯開，模擬機械計數器的進位感
                let delay = (len - 1 - i) as f64 * STAGGER_MS;
                let motion = Motion { start: now, delay, dir };
                slot.leave = Some((slot.ch, motion.clone()));
                slot.ch = ch;
                slot.enter = Some(motion);
            }
        }
        self.val = Some(s.to_string());
    }

    /// 還有沒有在動。
    pub fn active(&self, now: f64) -> bool {
        self.slots.iter().any(|s| {
            s.enter.as_ref().is_some_and(|m| now < m.start + m.delay + ENTER_MS)
                || s.leave.as_ref().is_some_and(|(_, m)| now < m.start + m.delay + LEAVE_MS)
        })
    }

    /// 動畫結束後清掉狀態。
    pub fn settle(&mut self, now: f64) {
        for s in &mut self.slots {
            if s.enter.as_ref().is_some_and(|m| now >= m.start + m.delay + ENTER_MS) {
                s.enter = None;
            }
            if s.leave.as_ref().is_some_and(|(_, m)| now >= m.start + m.delay + LEAVE_MS) {
                s.leave = None;
            }
        }
    }

    pub fn frame(&self, now: f64) -> Vec<SlotFrame> {
        self.slots
            .iter()
            .map(|s| {
                let current = match &s.enter {
                    Some(m) => {
                        let t = ((now - m.start - m.delay) / ENTER_MS).clamp(0.0, 1.0);
                        let e = if t >= 1.0 { 1.0 } else { spring(t * ENTER_MS / 1000.0) };
                        Glyph { ch: s.ch, offset: m.dir * (1.0 - e), opacity: e.clamp(0.0, 1.0) }
                    }
                    None => Glyph { ch: s.ch, offset: 0.0, opacity: 1.0 },
                };
                let leaving = s.leave.as_ref().and_then(|(ch, m)| {
                    let t = (now - m.start - m.delay) / LEAVE_MS;
                    if t >= 1.0 {
                        return None;
                    }
                    let e = cubic_bezier(0.4, 0.0, 0.7, 0.2, t.max(0.0));
                    Some(Glyph { ch: *ch, offset: -m.dir * e, opacity: 1.0 - e })
                });
                SlotFrame { current, leaving }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 彈簧收斂() {
        assert_eq!(spring(0.0), 0.0);
        assert!((spring(0.6) - 1.0).abs() < 0.03);
        // 有回彈（超過 1）
        assert!((0..60).map(|i| spring(i as f64 / 100.0)).fold(0.0, f64::max) > 1.05);
    }

    #[test]
    fn 貝茲曲線端點() {
        assert_eq!(cubic_bezier(0.4, 0.0, 0.7, 0.2, 0.0), 0.0);
        assert_eq!(cubic_bezier(0.4, 0.0, 0.7, 0.2, 1.0), 1.0);
        let mid = cubic_bezier(0.4, 0.0, 0.7, 0.2, 0.5);
        assert!(mid > 0.0 && mid < 0.5, "ease-in 前段慢：{mid}");
        assert!((cubic_bezier(0.0, 0.0, 1.0, 1.0, 0.3) - 0.3).abs() < 1e-6);
    }

    #[test]
    fn 只有變動的字元滾動() {
        let mut r = Roller::default();
        r.set("3.4h", 0.0, true);
        assert!(!r.active(0.0), "第一次直接顯示");
        r.set("3.5h", 1000.0, true);
        assert!(r.active(1000.0));
        let f = r.frame(1000.0);
        assert_eq!(f.len(), 4);
        assert!(f[0].leaving.is_none() && f[1].leaving.is_none() && f[3].leaving.is_none());
        // 增加：新字從下方（正位移）進來、舊字往上滑出
        assert_eq!(f[2].current.ch, '5');
        assert_eq!(f[2].current.offset, 1.0);
        assert_eq!(f[2].current.opacity, 0.0);
        assert_eq!(f[2].leaving.map(|g| g.ch), Some('4'));
        let later = r.frame(1000.0 + 30.0 + 200.0);
        assert!(later[2].leaving.is_some_and(|g| g.offset < 0.0));
        assert!(!r.active(1000.0 + 30.0 + ENTER_MS));
        let end = r.frame(1000.0 + 30.0 + ENTER_MS);
        assert_eq!(end[2].current, Glyph { ch: '5', offset: 0.0, opacity: 1.0 });
        assert!(end[2].leaving.is_none());
    }

    #[test]
    fn 減少時從上方進來() {
        let mut r = Roller::default();
        r.set("12", 0.0, true);
        r.set("11", 0.0, true);
        assert_eq!(r.frame(0.0)[1].current.offset, -1.0);
    }

    #[test]
    fn 由右往左錯開() {
        let mut r = Roller::default();
        r.set("19", 0.0, true);
        r.set("20", 0.0, true);
        let f = r.frame(20.0);
        // 右邊（個位）先動，左邊（十位）還沒開始
        assert!(f[1].current.offset < 1.0);
        assert_eq!(f[0].current.offset, 1.0);
    }

    #[test]
    fn 長度改變整組重建() {
        let mut r = Roller::default();
        r.set("9.9h", 0.0, true);
        r.set("10.0h", 0.0, true);
        let f = r.frame(0.0);
        assert_eq!(f.len(), 5);
        assert!(f.iter().all(|s| s.leaving.is_none() && s.current.opacity == 0.0));
        assert_eq!(r.text(), "10.0h");
    }

    #[test]
    fn 關閉動畫直接換() {
        let mut r = Roller::default();
        r.set("3.4h", 0.0, false);
        r.set("3.5h", 0.0, false);
        assert!(!r.active(0.0));
        assert_eq!(r.frame(0.0)[2].current, Glyph { ch: '5', offset: 0.0, opacity: 1.0 });
    }
}
