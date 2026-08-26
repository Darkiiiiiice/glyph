//! 候选窗渲染(M2):fontdue 光栅化文本到 ARGB 位图。
//! 纯逻辑、不碰 Wayland,可单元测试。像素格式对应 wl_shm ARGB8888(0xAARRGGBB)。

/// 候选窗外观:配色 + 字号 + 圆角。默认值即原硬编码配色,可被 config.conf 覆盖。
#[derive(Clone)]
pub struct Style {
    pub bg: u32,       // 微透明深灰底
    pub fg: u32,       // 候选白字
    pub pinyin_fg: u32, // 拼音浅灰
    pub hilite_bg: u32, // 选中行蓝底
    pub hilite_fg: u32, // 选中行字色
    pub font_size: f32,
    /// 圆角半径(px),0 = 直角。
    pub radius: f32,
    /// 自定义字体文件路径;None = 内置候选列表(Noto → 文楷 → Maple)。
    pub font_path: Option<String>,
}

impl Default for Style {
    fn default() -> Self {
        Self { bg: 0xee2b2b2b, fg: 0xffff_ffff, pinyin_fg: 0xff9a_9a9a, hilite_bg: 0xff3a_6ea5, hilite_fg: 0xffff_ffff, font_size: 18.0, radius: 8.0, font_path: None }
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
    /// 加载 CJK 字体:优先配置的 font_path,否则 Noto 黑体,fallback 霞鹜文楷/Maple。
    pub fn load(style: Style) -> Option<Self> {
        const CANDIDATES: &[&str] = &[
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/TTF/LXGWWenKai-Regular.ttf",
            "/usr/share/fonts/maple/MapleMono-NF-CN-Regular.ttf",
        ];
        let custom = style.font_path.as_deref().into_iter();
        for path in custom.chain(CANDIDATES.iter().copied()) {
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

        // 逐行绘制文字;选中行用 hilite_fg 反衬。
        for (li, line) in lines.iter().enumerate() {
            let fg = if li == 0 {
                self.style.pinyin_fg
            } else if li == sel_line {
                self.style.hilite_fg
            } else {
                self.style.fg
            };
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
        // 圆角最后遮:按覆盖率缩放角部像素 alpha,统一收住底色、高亮行与文字。
        let r = self.style.radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
        if r > 0.0 {
            round_corners(&mut px, w, h, r);
        }
        (width as u32, height as u32, px)
    }
}

/// 四角 alpha 遮罩:以 (r, r) 为圆心,像素中心到圆心距离超出 r 的按超出量衰减。
fn round_corners(px: &mut [u32], w: usize, h: usize, r: f32) {
    for cy in 0..r.ceil() as usize {
        for cx in 0..r.ceil() as usize {
            let dx = r - cx as f32 - 0.5;
            let dy = r - cy as f32 - 0.5;
            let cov = (r - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            if cov >= 1.0 {
                continue;
            }
            for (x, y) in [(cx, cy), (w - 1 - cx, cy), (cx, h - 1 - cy), (w - 1 - cx, h - 1 - cy)] {
                let p = &mut px[y * w + x];
                let a = ((*p >> 24) & 0xff) as f32 * cov;
                *p = (*p & 0x00ff_ffff) | ((a as u32) << 24);
            }
        }
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

    #[test]
    fn rounded_corners_are_transparent() {
        let Some(r) = renderer() else { return };
        let (w, h, px) = r.render("nihao", &["你好".to_string()], 0);
        let (w, h) = (w as usize, h as usize);
        // 四角顶点(0,0)/(w-1,0)/(0,h-1)/(w-1,h-1)在半径 8 的圆外,alpha 必须为 0
        for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)] {
            assert_eq!(px[y * w + x] >> 24, 0, "角点 ({x},{y}) 应全透明");
        }
        // 窗中心保持不透明底
        assert!(px[h / 2 * w + w / 2] >> 24 > 0);
        // radius=0 时角点保持底色(不遮罩)
        let mut flat = Style::default();
        flat.radius = 0.0;
        let Some(r0) = Renderer::load(flat) else { return };
        let (_, _, px0) = r0.render("nihao", &["你好".to_string()], 0);
        assert_eq!(px0[0] >> 24, Style::default().bg >> 24);
    }
}
