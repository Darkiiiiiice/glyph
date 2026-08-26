//! 候选窗样式预览:按 ~/.config/glyph/config.conf 的实际配置渲染一屏样例。
//! 调配色/字号/圆角时不用重启 daemon:
//!   cargo run --release --example popup_preview [输出.ppm]
//! PPM 可用 `magick out.ppm out.png` 转 PNG 查看。

use glyph_frontend::config::Config;
use glyph_frontend::render::Renderer;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "/tmp/glyph_popup_preview.ppm".into());
    let Some(r) = Renderer::load(Config::load().style) else {
        eprintln!("无可用 CJK 字体");
        std::process::exit(1);
    };
    let cands: Vec<String> =
        ["你好", "泥嚎", "尼好", "拟好", "妮好", "伲好", "坭好", "旎好", "儞好"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    let (w, h, px) = r.render("nihao", &cands, 0);
    // ARGB 合成到深色桌面底色(#11111b),模拟浮窗落在暗色窗口上的观感。
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in px {
        let a = (p >> 24) & 0xff;
        let inv = 255 - a;
        let ch = |shift: u32, bg: u32| ((((p >> shift) & 0xff) * a + bg * inv) / 255) as u8;
        ppm.extend_from_slice(&[ch(16, 0x11), ch(8, 0x11), ch(0, 0x1b)]);
    }
    std::fs::write(&out, ppm).expect("写 PPM 失败");
    println!("{out} ({w}x{h})");
}
