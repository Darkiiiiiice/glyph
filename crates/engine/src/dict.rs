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
    /// 用户词频:整句候选 text → 被选择次数。动态调频层,不混入 trie/total_freq。
    pub user_freq: HashMap<String, u32>,
    /// 简拼索引:多字词各字声母连成的 key(你好→nh、中国→zhg) → [(词, 词频)],
    /// finish 时按词频降序。简拼是精确声母匹配,不参与音节格 DP。
    pub jianpin: HashMap<String, Vec<(String, u32)>>,
    /// 用户二元搭配(bigram):上一次上屏的尾词 → {当前词 → 搭配次数}。
    /// 冷启动从用户输入历史积累,无外部语料;嵌套 map 使查询免 tuple 分配。
    pub user_bigram: HashMap<String, HashMap<String, u32>>,
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
        Self { root: Node::default(), syllables: HashSet::new(), total_freq: 0, user_freq: HashMap::new(), jianpin: HashMap::new(), user_bigram: HashMap::new() }
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
        // 多字词建简拼索引:各音节声母连成 key。
        if sylls.len() >= 2 {
            let key: String = sylls.iter().map(|s| shengmu(s)).collect();
            self.jianpin.entry(key).or_default().push((word.to_string(), freq));
        }
        Ok(())
    }

    fn finish(&mut self) {
        fn sort_node(node: &mut Node) {
            node.words.sort_by(|a, b| b.1.cmp(&a.1));
            node.children.values_mut().for_each(sort_node);
        }
        sort_node(&mut self.root);
        for v in self.jianpin.values_mut() {
            v.sort_by(|a, b| b.1.cmp(&a.1));
        }
    }

    /// 从行格式文本构建(测试与 Engine::from_str 用)。
    pub fn from_lines(lines: &str) -> Self {
        let mut lex = Self::empty();
        for line in lines.lines().filter(|l| !l.is_empty()) {
            lex.insert_line(line).unwrap();
        }
        lex.finish();
        lex
    }
}
/// 音节的声母(简拼 key 用):zh/ch/sh 为双字母整体,其余取首字母
/// (零声母音节 a/o/e 开头取首字母,如 an→a、ou→o)。
fn shengmu(syll: &str) -> &str {
    if syll.is_empty() {
        return ""; // 保险:insert_line 已拒空音节(拼音含空音节→Err),此处仅防御
    }
    let b = syll.as_bytes();
    if b.len() >= 2 && matches!(&b[..2], b"zh" | b"ch" | b"sh") { &syll[..2] } else { &syll[..1] }
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
    #[test]
    fn builds_jianpin_index() {
        let lex = Lexicon::from_lines(
            "ni'hao 你好 10000\nzhong'guo 中国 8000\nni 你 500\n",
        );
        // 多字词建简拼;zh 是双字母声母整体
        assert!(lex.jianpin["nh"].iter().any(|(w, _)| w == "你好"));
        assert!(lex.jianpin["zhg"].iter().any(|(w, _)| w == "中国"));
        // 单字不进简拼索引
        assert!(!lex.jianpin.values().flatten().any(|(w, _)| w == "你"));
    }
}
