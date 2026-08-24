//! 候选窗渲染(M2):fontdue 光栅化文本到 ARGB 位图。
//! 纯逻辑、不碰 Wayland,可单元测试。像素格式对应 wl_shm ARGB8888(0xAARRGGBB)。

/// 候选窗外观:配色 + 字号。默认值即原硬编码配色,可被 config.conf 覆盖。
#[derive(Clone)]
pub struct Style {
    pub bg: u32,       // 微透明深灰底
    pub fg: u32,       // 候选白字
    pub pinyin_fg: u32, // 拼音浅灰
    pub hilite_bg: u32, // 选中行蓝底
    pub font_size: f32,
}

impl Default for Style {
    fn default() -> Self {
        Self { bg: 0xee2b2b2b, fg: 0xffff_ffff, pinyin_fg: 0xff9a_9a9a, hilite_bg: 0xff3a_6ea5, font_size: 18.0 }
    }
}

const PAD: i32 = 10;
const LINE_GAP: i32 = 8;

/// 候选窗渲染器:持有一个 CJK 字体与外观。
pub struct Renderer {
    font: fontdue::Font,
    style: Style,
}

impl Renderer {
    /// 加载 CJK 字体:优先 Noto 黑体,fallback 霞鹜文楷/Maple。
    pub fn load(style: Style) -> Option<Self> {
        const CANDIDATES: &[&str] = &[
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/TTF/LXGWWenKai-Regular.ttf",
            "/usr/share/fonts/maple/MapleMono-NF-CN-Regular.ttf",
        ];
        for path in CANDIDATES {
            if let Ok(bytes) = std::fs::read(path) {
                if let Ok(font) = fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default()) {
                    log::info!("候选窗字体: {path}");
                    return Some(Self { font, style });
                }
            }
        }
        log::error!("未找到可用 CJK 字体,候选窗无法渲染");
        None
    }

    /// 文本像素宽(累加各字形 advance)。
    fn text_width(&self, text: &str) -> i32 {
        text.chars().map(|c| self.font.metrics(c, self.style.font_size).advance_width as i32).sum()
    }

    /// 渲染候选窗内容:第 0 行拼音,其后编号候选,`selected` 候选行整行高亮。
    /// 返回 (宽, 高, ARGB 像素)。
    pub fn render(&self, pinyin: &str, candidates: &[String], selected: usize) -> (u32, u32, Vec<u32>) {
        let line_h = self.style.font_size as i32 + LINE_GAP;
        let mut lines: Vec<String> = vec![pinyin.to_string()];
        for (i, c) in candidates.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, c));
        }
        let width = lines.iter().map(|l| self.text_width(l)).max().unwrap_or(0) + PAD * 2;
        let height = lines.len() as i32 * line_h + PAD * 2 - LINE_GAP;
        let (w, h) = (width.max(1) as usize, height.max(1) as usize);
        let mut px = vec![self.style.bg; w * h];

        // 选中行高亮(候选 index → 行号 selected+1,因第 0 行是拼音)。
        let sel_line = selected + 1;
        if sel_line < lines.len() {
            let y0 = PAD + sel_line as i32 * line_h;
            for y in y0..(y0 + line_h).min(height) {
                for x in 0..width {
                    px[y as usize * w + x as usize] = self.style.hilite_bg;
                }
            }
        }

        // 逐行绘制文字。
        for (li, line) in lines.iter().enumerate() {
            let fg = if li == 0 { self.style.pinyin_fg } else { self.style.fg };
            let baseline = PAD + li as i32 * line_h + self.style.font_size as i32;
            let mut pen_x = PAD;
            for ch in line.chars() {
                let (m, bitmap) = self.font.rasterize(ch, self.style.font_size);
                let gx = pen_x + m.xmin;
                let gy = baseline - m.ymin - m.height as i32;
                for row in 0..m.height as i32 {
                    for col in 0..m.width as i32 {
                        let (dx, dy) = (gx + col, gy + row);
                        if dx < 0 || dy < 0 || dx >= width || dy >= height {
                            continue;
                        }
                        let cov = bitmap[row as usize * m.width + col as usize];
                        let dst = &mut px[dy as usize * w + dx as usize];
                        *dst = blend(*dst, fg, cov);
                    }
                }
                pen_x += m.advance_width as i32;
            }
        }
        (width as u32, height as u32, px)
    }
}

/// 前景色 fg 按覆盖率 cov(0-255)叠加到背景 dst 上(简化 alpha 合成)。
fn blend(dst: u32, fg: u32, cov: u8) -> u32 {
    let a = cov as u32;
    let inv = 255 - a;
    let ch = |s: u32, f: u32| (f * a + s * inv) / 255;
    let r = ch((dst >> 16) & 0xff, (fg >> 16) & 0xff);
    let g = ch((dst >> 8) & 0xff, (fg >> 8) & 0xff);
    let b = ch(dst & 0xff, fg & 0xff);
    0xff00_0000 | (r << 16) | (g << 8) | b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn renderer() -> Option<Renderer> {
        Renderer::load(Style::default())
    }

    #[test]
    fn render_size_grows_with_candidates() {
        let Some(r) = renderer() else { return }; // 无字体环境跳过
        let (w1, h1, _) = r.render("ni", &["你".to_string()], 0);
        let cands: Vec<String> = (0..5).map(|i| format!("候选{i}")).collect();
        let (w2, h2, _) = r.render("nihao", &cands, 0);
        assert!(h2 > h1, "更多候选应更高: {h1} vs {h2}");
        assert!(w2 >= w1);
    }

    #[test]
    fn render_produces_nonblank_pixels() {
        let Some(r) = renderer() else { return };
        let (_, _, px) = r.render("nihao", &["你好".to_string()], 0);
        // 文字像素应区别于背景(有非 BG 像素)
        assert!(px.iter().any(|&p| p != Style::default().bg), "渲染结果应有文字像素");
    }
}
