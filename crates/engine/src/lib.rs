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

    /// 转换整串拼音，返回按 unigram 概率降序的候选，最多 `limit` 条。
    /// 输入中的 `'` 是强制音节边界（如 `xi'an`）。
    pub fn convert(&self, input: &str, limit: usize) -> Vec<Candidate> {
        segment::convert(&self.lexicon, input, limit)
    }
}
