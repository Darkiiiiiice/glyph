//! 词典：以音节为键的 trie，每个节点挂着该音节序列对应的全部（词, 词频）。
//!
//! lexicon 文件行格式（glyph-build 生成）：`pin'yin 词 词频`，空白分隔。
//! 词库加载后同时派生出两样东西：
//! - `syllables`：全部出现过的音节集合 → 音节格的合法性判据；
//! - `total_freq`：词频总和 → unigram 概率的分母。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use crate::syllable::Lattice;

#[derive(Default)]
struct Node {
    children: HashMap<String, Node>,
    /// 该音节序列对应的词，按词频降序（finish 时排序）。
    words: Vec<(String, u32)>,
}

pub struct Lexicon {
    root: Node,
    pub syllables: HashSet<String>,
    pub total_freq: u64,
}

impl Lexicon {
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut lex = Self::empty();
        for (lineno, line) in BufReader::new(File::open(path)?).lines().enumerate() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            lex.insert_line(&line).map_err(|msg| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}:{}: {}", path.display(), lineno + 1, msg),
                )
            })?;
        }
        lex.finish();
        Ok(lex)
    }

    /// 在音节格上走 trie，回调每一条命中的词边：（起点字节, 终点字节, 词条列表）。
    /// 从每个有音节出发的字节位置各走一次；同一（起, 终）可能因多音字词条
    /// 出现多次，由上层按候选文本去重。
    pub fn for_each_word_edge<'a>(
        &'a self,
        lattice: &Lattice,
        mut f: impl FnMut(usize, usize, &'a [(String, u32)]),
    ) {
        let mut stack: Vec<(&Node, usize)> = Vec::new();
        for start in 0..lattice.text.len() {
            if lattice.starts[start].is_empty() {
                continue;
            }
            stack.push((&self.root, start));
            while let Some((node, pos)) = stack.pop() {
                for &idx in &lattice.starts[pos] {
                    let (a, b) = lattice.syllables[idx];
                    match node.children.get(&lattice.text[a..b]) {
                        Some(child) => {
                            if !child.words.is_empty() {
                                f(start, b, &child.words);
                            }
                            stack.push((child, b));
                        }
                        None => {} // 该音节序列无词，剪枝
                    }
                }
            }
        }
    }

    fn empty() -> Self {
        Self { root: Node::default(), syllables: HashSet::new(), total_freq: 0 }
    }

    fn insert_line(&mut self, line: &str) -> Result<(), String> {
        let mut fields = line.split_whitespace();
        let pinyin = fields.next().ok_or("缺拼音列")?;
        let word = fields.next().ok_or("缺词列")?;
        let freq: u32 =
            fields.next().and_then(|s| s.parse().ok()).ok_or("缺词频列")?;
        let sylls: Vec<&str> = pinyin.split('\'').collect();
        if sylls.iter().any(|s| s.is_empty()) {
            return Err(format!("拼音含空音节: {pinyin}"));
        }
        let mut node = &mut self.root;
        for syll in &sylls {
            self.syllables.insert(syll.to_string());
            node = node.children.entry(syll.to_string()).or_default();
        }
        node.words.push((word.to_string(), freq));
        self.total_freq += u64::from(freq);
        Ok(())
    }

    fn finish(&mut self) {
        fn sort_node(node: &mut Node) {
            node.words.sort_by(|a, b| b.1.cmp(&a.1));
            node.children.values_mut().for_each(sort_node);
        }
        sort_node(&mut self.root);
    }

    #[cfg(test)]
    pub(crate) fn from_lines(lines: &str) -> Self {
        let mut lex = Self::empty();
        for line in lines.lines().filter(|l| !l.is_empty()) {
            lex.insert_line(line).unwrap();
        }
        lex.finish();
        lex
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derives_syllables_and_total() {
        let lex = Lexicon::from_lines("ni'hao 你好 10000\nni 你 500\nhao 好 300\n");
        assert!(lex.syllables.contains("ni") && lex.syllables.contains("hao"));
        assert_eq!(lex.total_freq, 10800);
    }
}
