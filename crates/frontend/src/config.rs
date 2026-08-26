//! 用户配置:`~/.config/glyph/config.conf`,`key = value` 行格式,`#` 注释。
//! 极简手写解析(零依赖,符合"代码可完全理解");未配置的键用默认值
//! (即参数化之前的硬编码值),缺文件/坏行静默回退默认,不影响启动。

use std::path::PathBuf;

use crate::render::Style;

/// 输入法配置:候选窗外观(Style) + 默认行为。
pub struct Config {
    /// 候选窗配色与字号。
    pub style: Style,
    /// 默认中/英文标点模式(true=中文标点)。
    pub punct_cn: bool,
    /// 每页候选数(数字键 1-9 直选,>9 时超出数字键范围的只能翻页选中)。
    pub page_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { style: Style::default(), punct_cn: true, page_size: 9 }
    }
}

impl Config {
    /// 配置路径:`$XDG_CONFIG_HOME/glyph/config.conf`,默认 `~/.config/glyph/config.conf`。
    pub fn path() -> PathBuf {
        let base = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
        });
        base.join("glyph").join("config.conf")
    }

    /// 加载配置;文件不存在或解析失败的键回退默认值。
    pub fn load() -> Self {
        let mut cfg = Self::default();
        let Ok(text) = std::fs::read_to_string(Self::path()) else { return cfg };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            // 行尾注释:首个"空白 + #"起丢弃;值开头的 #RRGGBB 无前置空白,不受影响。
            let v = v.trim_start();
            let cut = v
                .char_indices()
                .skip(1)
                .find(|&(i, _)| v.as_bytes()[i] == b'#' && v.as_bytes()[i - 1].is_ascii_whitespace())
                .map(|(i, _)| i);
            let v = cut.map_or(v, |i| v[..i].trim_end());
            match k.trim() {
                "font_size" => {
                    if let Ok(x) = v.trim().parse() {
                        cfg.style.font_size = x;
                    }
                }
                "punct_cn" => cfg.punct_cn = matches!(v.trim(), "true" | "1" | "yes"),
                "page_size" => {
                    if let Ok(x) = v.trim().parse::<usize>() {
                        cfg.page_size = x.clamp(1, 20);
                    }
                }
                "radius" => {
                    if let Ok(x) = v.trim().parse() {
                        cfg.style.radius = x;
                    }
                }
                "font_path" => cfg.style.font_path = Some(v.trim().to_string()),
                "bg" => set(&mut cfg.style.bg, v),
                "fg" => set(&mut cfg.style.fg, v),
                "pinyin_fg" => set(&mut cfg.style.pinyin_fg, v),
                "hilite_bg" => set(&mut cfg.style.hilite_bg, v),
                "hilite_fg" => set(&mut cfg.style.hilite_fg, v),
                other => log::warn!("glyph 配置:未知键 {other}"),
            }
        }
        cfg
    }
}

/// 把 `RRGGBB`/`AARRGGBB`(可带 `0x`/`#` 前缀)写入颜色字段;RRGGBB 补全 alpha=ff。
fn set(field: &mut u32, v: &str) {
    let s = v.trim().trim_start_matches("0x").trim_start_matches('#');
    if let Ok(x) = u32::from_str_radix(s, 16) {
        *field = if s.len() <= 6 { 0xff00_0000 | x } else { x };
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trailing_comment_is_stripped() {
        // 值后的"空白 + #"是注释;#RRGGBB 前缀不是注释。用临时 XDG_CONFIG_HOME 跑完整 load。
        let xdg = std::env::temp_dir().join(format!("glyph-test-xdg-{}", std::process::id()));
        std::fs::create_dir_all(xdg.join("glyph")).unwrap();
        std::fs::write(xdg.join("glyph/config.conf"), "bg = ff2a6d   # 霓虹粉\nfg = #00f0ff\n").unwrap();
        std::env::set_var("XDG_CONFIG_HOME", &xdg);
        let cfg = super::Config::load();
        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&xdg);
        assert_eq!(cfg.style.bg, 0xffff2a6d, "行尾注释应被剥离");
        assert_eq!(cfg.style.fg, 0xff00f0ff, "# 前缀颜色应保留");
    }

    #[test]
    fn hex_parsing() {
        assert_eq!(super::Config::default().style.bg, 0xee2b2b2b);
        let mut c = 0u32;
        super::set(&mut c, "ff8800");
        assert_eq!(c, 0xffff8800, "RRGGBB 应补全不透明 alpha");
        super::set(&mut c, "#803a6ea5");
        assert_eq!(c, 0x803a6ea5, "AARRGGBB 保留自带 alpha");
    }
}
