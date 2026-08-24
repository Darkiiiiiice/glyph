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
}

impl Default for Config {
    fn default() -> Self {
        Self { style: Style::default(), punct_cn: true }
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
            match k.trim() {
                "font_size" => {
                    if let Ok(x) = v.trim().parse() {
                        cfg.style.font_size = x;
                    }
                }
                "punct_cn" => cfg.punct_cn = matches!(v.trim(), "true" | "1" | "yes"),
                "bg" => set(&mut cfg.style.bg, v),
                "fg" => set(&mut cfg.style.fg, v),
                "pinyin_fg" => set(&mut cfg.style.pinyin_fg, v),
                "hilite_bg" => set(&mut cfg.style.hilite_bg, v),
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
    fn hex_parsing() {
        assert_eq!(super::Config::default().style.bg, 0xee2b2b2b);
        let mut c = 0u32;
        super::set(&mut c, "ff8800");
        assert_eq!(c, 0xffff8800, "RRGGBB 应补全不透明 alpha");
        super::set(&mut c, "#803a6ea5");
        assert_eq!(c, 0x803a6ea5, "AARRGGBB 保留自带 alpha");
    }
}
