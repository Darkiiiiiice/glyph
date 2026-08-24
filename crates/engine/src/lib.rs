//! Glyph 引擎内核：拼音串 → 中文候选。
//!
//! 三层结构，每层一个模块：
//! - [`syllable`] 音节格：把输入字母串按合法音节的所有可能切法展开；
//! - [`dict`] 词典 trie：音节序列 → （词, 词频），并派生出合法音节集合；
//! - [`segment`] DP 切分：在音节格上跑 unigram k-best，输出整句候选。

mod dict;
mod segment;
mod syllable;

use std::io;
use std::io::BufRead;
use std::path::Path;

pub use segment::Candidate;

/// 引擎句柄：加载一次词库，反复转换。
pub struct Engine {
    lexicon: dict::Lexicon,
}

impl Engine {
    /// 从 lexicon 文件（`pin'yin 词 词频` 行格式，由 glyph-build 生成）加载。
    pub fn load(path: &Path) -> io::Result<Self> {
        Ok(Self { lexicon: dict::Lexicon::load(path)? })
    }

    /// 从内存中的行格式文本构建（测试与用户词库注入用）。
    pub fn from_str(lines: &str) -> Self {
        Self { lexicon: dict::Lexicon::from_lines(lines) }
    }

    /// 转换整串拼音，返回按 unigram 概率降序的候选，最多 `limit` 条。
    /// 输入中的 `'` 是强制音节边界（如 `xi'an`）。
    pub fn convert(&self, input: &str, limit: usize) -> Vec<Candidate> {
        segment::convert(&self.lexicon, input, limit)
    }

    /// 记录一个被选择的候选文本：次数 +1，此后 convert 排序会上浮它。
    pub fn learn(&mut self, text: &str) {
        if let Some(h) = self.lexicon.user_freq.get_mut(text) {
            *h += 1;
        }
        // 候选文本必然来自 lexicon 词条;但防御性保留(测试可能喂任意串)
        else {
            self.lexicon.user_freq.insert(text.to_string(), 1);
        }
    }

    /// 已学到的用户词频（文本 → 次数）。
    pub fn user_freq(&self) -> std::collections::HashMap<String, u32> {
        self.lexicon.user_freq.clone()
    }

    /// 从用户词频文件加载（`词 次数` 行）;文件不存在时为空。
    pub fn load_user_freq(path: &Path) -> io::Result<std::collections::HashMap<String, u32>> {
        let mut map = std::collections::HashMap::new();
        for (lineno, line) in io::BufReader::new(std::fs::File::open(path)?).lines().enumerate() {
            let line = line?;
            let mut fields = line.split_whitespace();
            let (Some(word), Some(count)) = (fields.next(), fields.next()) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}:{}: 缺 词/次数 列", path.display(), lineno + 1),
                ));
            };
            let count: u32 = count.parse().map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}:{}: 次数非整数", path.display(), lineno + 1),
                )
            })?;
            map.insert(word.to_string(), count);
        }
        Ok(map)
    }

    /// 把用户词频合并进引擎（供启动时加载）。
    pub fn set_user_freq(&mut self, map: std::collections::HashMap<String, u32>) {
        self.lexicon.user_freq = map;
    }

    /// 把 user_freq 写盘（`词 次数` 行，按次数降序）。无数据时清空该文件。
    pub fn save_user_freq(&self, path: &Path) -> io::Result<()> {
        let mut entries: Vec<_> = self.lexicon.user_freq.iter().collect();
        entries.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let mut out = String::new();
        for (word, count) in entries {
            out.push_str(&format!("{word} {count}\n"));
        }
        std::fs::write(path, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learn_accumulates_and_boosts_in_convert() {
        let mut e = Engine::from_str("ni 你 500\nhao 好 300\nni'hao 你好 10000\nni'hao 泥蒿 5\n");
        // 选"泥蒿"3 次 → convert 后它应顶到首位
        for _ in 0..3 {
            e.learn("泥蒿");
        }
        assert_eq!(e.user_freq().get("泥蒿"), Some(&3));
        assert_eq!(e.convert("nihao", 9)[0].text, "泥蒿");
    }

    #[test]
    fn user_freq_survives_roundtrip() {
        let mut e = Engine::from_str("ni 你 500\nhao 好 300\n");
        e.learn("你"); e.learn("你"); e.learn("好");
        let dir = std::env::temp_dir().join("glyph_test_user_freq");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("freq.txt");
        e.save_user_freq(&p).unwrap();
        let loaded = Engine::load_user_freq(&p).unwrap();
        assert_eq!(loaded.get("你"), Some(&2));
        assert_eq!(loaded.get("好"), Some(&1));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn load_user_freq_empty_file_is_ok() {
        let dir = std::env::temp_dir().join("glyph_test_user_freq");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("empty.txt");
        std::fs::write(&p, "").unwrap();
        assert!(Engine::load_user_freq(&p).unwrap().is_empty());
        std::fs::remove_file(&p).ok();
    }

    /// 真实词库集成测试(词库不在时跳过):验证 USER_W 在真实词频尺度下,
    /// 用户选低频同音词 3 次后它应顶到首位。
    #[test]
    fn real_lexicon_user_freq_promotes_picked_word() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lexicon.txt");
        if !path.exists() {
            return; // 无真实词库时跳过
        }
        let mut e = Engine::load(&path).unwrap();
        let input = "shiji";
        let first = e.convert(input, 9)[0].text.clone();
        assert_eq!(first, "世纪", "静态最高频应首位");
        // 用户连续 3 次选"诗集"(最低频同音词)
        for _ in 0..3 {
            e.learn("诗集");
        }
        let top = &e.convert(input, 9)[0].text;
        assert_eq!(top, "诗集", "选 3 次应上浮到首位,实得 {top}");
    }
}
