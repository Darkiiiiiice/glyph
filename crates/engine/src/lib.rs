//! Glyph 引擎内核：拼音串 → 中文候选。
//!
//! 三层结构，每层一个模块：
//! - [`syllable`] 音节格：把输入字母串按合法音节的所有可能切法展开；
//! - [`dict`] 词典 trie：音节序列 → （词, 词频），并派生出合法音节集合；
//! - [`segment`] DP 切分：在音节格上跑 unigram k-best，输出整句候选。

mod dict;
mod segment;
mod syllable;
mod user_dict;

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

    /// 带上文的转换:`ctx` 最近优先(ctx[0]=上一次上屏尾词,ctx[1]=上上次);
    /// 候选首词与上文有搭配记录时上浮(bigram 用 ctx[0],trigram 用 ctx[..2])。
    pub fn convert_ctx(&self, input: &str, limit: usize, ctx: &[&str]) -> Vec<Candidate> {
        segment::convert_ctx(&self.lexicon, input, limit, ctx)
    }

    /// Tab 单字模式:第一音节的全部单字候选(见 segment::first_syllable_chars)。
    pub fn first_syllable_chars(&self, input: &str, limit: usize) -> Vec<Candidate> {
        segment::first_syllable_chars(&self.lexicon, input, limit)
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

    /// 记录一次上文搭配:prev(上一次上屏尾词) → cur(本次上屏的首词),次数 +1。
    pub fn learn_bigram(&mut self, prev: &str, cur: &str) {
        *self.lexicon.user_bigram.entry(prev.to_string()).or_default().entry(cur.to_string()).or_insert(0) += 1;
    }

    /// 记录一次双词上文搭配:(上上文 prev2, 上文 prev1) → cur(本次上屏首词),次数 +1。
    pub fn learn_trigram(&mut self, prev2: &str, prev1: &str, cur: &str) {
        *self.lexicon.user_trigram.entry(prev2.to_string()).or_default().entry(prev1.to_string()).or_default().entry(cur.to_string()).or_insert(0) += 1;
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

    /// 从用户 bigram 文件加载(`上文 当前词 次数` 行);文件不存在时为空。
    pub fn load_bigram(path: &Path) -> io::Result<std::collections::HashMap<String, std::collections::HashMap<String, u32>>> {
        let mut map: std::collections::HashMap<String, std::collections::HashMap<String, u32>> = std::collections::HashMap::new();
        for (lineno, line) in io::BufReader::new(std::fs::File::open(path)?).lines().enumerate() {
            let line = line?;
            let mut fields = line.split_whitespace();
            let (Some(prev), Some(cur), Some(count)) = (fields.next(), fields.next(), fields.next()) else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}:{}: 缺 上文/当前词/次数 列", path.display(), lineno + 1)));
            };
            let count: u32 = count.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("{}:{}: 次数非整数", path.display(), lineno + 1)))?;
            map.entry(prev.to_string()).or_default().insert(cur.to_string(), count);
        }
        Ok(map)
    }

    /// 把用户 bigram 合并进引擎(供启动时加载)。
    pub fn set_user_bigram(&mut self, map: std::collections::HashMap<String, std::collections::HashMap<String, u32>>) {
        self.lexicon.user_bigram = map;
    }

    /// 用户造词:逐字序列学成一个新词,进运行期 overlay(不动池化 trie)。
    /// 词库已有同路径同文本词时不进 overlay(防膨胀,该词靠 user_freq 上浮),返回 false。
    /// 音节不在词库音节表中 = 该拼音永远切不出候选,插入无意义,返回 false(防御)。
    pub fn add_user_word(&mut self, syllables: &[&str], text: &str) -> bool {
        if syllables.is_empty() || text.is_empty() {
            return false;
        }
        let ids: Option<Vec<dict::SyllId>> =
            syllables.iter().map(|s| self.lexicon.syllable_id(s)).collect();
        let Some(ids) = ids else { return false };
        if self
            .lexicon
            .words_at_path(&ids)
            .is_some_and(|ws| ws.iter().any(|(w, _)| w == text))
        {
            return false;
        }
        self.lexicon.user_words.insert(syllables, text)
    }

    /// 从用户造词文件加载(`pin'yin 词 词频` 行,与 lexicon 同格式;词频列忽略,
    /// overlay 统一用 USER_WORD_FREQ)。逐条过 add_user_dict 的词库查重。
    pub fn load_user_dict(&mut self, path: &Path) -> io::Result<usize> {
        let mut n = 0;
        for (lineno, line) in io::BufReader::new(std::fs::File::open(path)?).lines().enumerate() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            let mut fields = line.split_whitespace();
            let (Some(pinyin), Some(word)) = (fields.next(), fields.next()) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}:{}: 缺 拼音/词 列", path.display(), lineno + 1),
                ));
            };
            let sylls: Vec<&str> = pinyin.split('\'').collect();
            if self.add_user_word(&sylls, word) {
                n += 1;
            }
        }
        Ok(n)
    }

    /// 把用户造词写盘(`pin'yin 词 词频` 行,按造词先后)。无数据时清空该文件。
    pub fn save_user_dict(&self, path: &Path) -> io::Result<()> {
        std::fs::write(path, self.lexicon.user_words.to_lines())
    }

    /// 把 user_bigram 写盘(`上文 当前词 次数` 行,上文字典序 + 次数降序)。无数据时清空该文件。
    pub fn save_bigram(&self, path: &Path) -> io::Result<()> {
        let mut prevs: Vec<_> = self.lexicon.user_bigram.iter().collect();
        prevs.sort_by(|a, b| a.0.cmp(b.0));
        let mut out = String::new();
        for (prev, m) in prevs {
            let mut curs: Vec<_> = m.iter().collect();
            curs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            for (cur, count) in curs {
                out.push_str(&format!("{prev} {cur} {count}\n"));
            }
        }
        std::fs::write(path, out)
    }

    /// 从用户 trigram 文件加载(`上上文 上文 当前词 次数` 行);文件不存在时为空。
    pub fn load_trigram(path: &Path) -> io::Result<std::collections::HashMap<String, std::collections::HashMap<String, std::collections::HashMap<String, u32>>>> {
        let mut map: std::collections::HashMap<String, std::collections::HashMap<String, std::collections::HashMap<String, u32>>> = std::collections::HashMap::new();
        for (lineno, line) in io::BufReader::new(std::fs::File::open(path)?).lines().enumerate() {
            let line = line?;
            let mut fields = line.split_whitespace();
            let (Some(prev2), Some(prev1), Some(cur), Some(count)) = (fields.next(), fields.next(), fields.next(), fields.next()) else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{}:{}: 缺 上上文/上文/当前词/次数 列", path.display(), lineno + 1)));
            };
            let count: u32 = count.parse().map_err(|_| io::Error::new(io::ErrorKind::InvalidData, format!("{}:{}: 次数非整数", path.display(), lineno + 1)))?;
            map.entry(prev2.to_string()).or_default().entry(prev1.to_string()).or_default().insert(cur.to_string(), count);
        }
        Ok(map)
    }

    /// 把用户 trigram 合并进引擎(供启动时加载)。
    pub fn set_user_trigram(&mut self, map: std::collections::HashMap<String, std::collections::HashMap<String, std::collections::HashMap<String, u32>>>) {
        self.lexicon.user_trigram = map;
    }

    /// 把 user_trigram 写盘(`上上文 上文 当前词 次数` 行,字典序 + 次数降序)。无数据时清空该文件。
    pub fn save_trigram(&self, path: &Path) -> io::Result<()> {
        let mut out = String::new();
        let mut p2s: Vec<_> = self.lexicon.user_trigram.iter().collect();
        p2s.sort_by(|a, b| a.0.cmp(b.0));
        for (p2, m1) in p2s {
            let mut p1s: Vec<_> = m1.iter().collect();
            p1s.sort_by(|a, b| a.0.cmp(b.0));
            for (p1, m) in p1s {
                let mut curs: Vec<_> = m.iter().collect();
                curs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
                for (cur, count) in curs {
                    out.push_str(&format!("{p2} {p1} {cur} {count}\n"));
                }
            }
        }
        std::fs::write(path, out)
    }
}

#[cfg(test)]
mod tests;
