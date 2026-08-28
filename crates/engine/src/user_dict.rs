//! 用户造词 overlay：运行期可插入的小型词典，与池化 trie 并查。
//!
//! 为什么不插进池化 trie：节点子边是边池中的连续段（构建期排序展平），
//! 中间插入一条边要移动全池、重建索引，等于把 1.39M 词条重展平一遍（秒级）。
//! 用户造词量级小（千级），独立嵌套 map 小 trie 即可：convert 时沿音节格
//! 走一遍，词边与 trie 词边同桶进 DP，去重/排序/user_freq 增量全部复用。
//!
//! 边界：只接全拼 DP；简拼/混合拼索引（构建期摊平的路径池）不接 overlay——
//! 造词场景是生僻词/专名，第一遍本就是全拼逐字打的，全拼能出即闭环。

use std::collections::HashMap;

use crate::syllable::Lattice;

/// 用户词静态词频。量纲参照：jieba 普通词几十到几千；新词需进候选池(pool=90)
/// 但不必霸榜——造词伴随的上屏会触发 user_freq +1(ln 空间 +4.16),排序由
/// 动态调频接管,静态分只保证"能见到"。
pub(crate) const USER_WORD_FREQ: u32 = 100;

/// 一层节点：路径终点挂的词 + 下一音节的子节点。
#[derive(Default)]
struct Node {
    /// (词文本, 词频)。同一路径可挂多词(同音词)。
    words: Vec<(String, u32)>,
    children: HashMap<String, Node>,
}

/// 用户词典。`order` 记录插入序(pinyin, word),写盘顺序稳定可 diff。
#[derive(Default)]
pub(crate) struct UserDict {
    roots: HashMap<String, Node>,
    order: Vec<(String, String)>,
}

impl UserDict {
    /// 插入一个词。音节路径与文本均非空;同路径同文本已存在时不重复(幂等)。
    pub(crate) fn insert(&mut self, syllables: &[&str], word: &str) -> bool {
        debug_assert!(!syllables.is_empty() && !word.is_empty());
        let mut node = self.roots.entry(syllables[0].to_string()).or_default();
        for syll in &syllables[1..] {
            node = node.children.entry(syll.to_string()).or_default();
        }
        if node.words.iter().any(|(w, _)| w == word) {
            return false;
        }
        node.words.push((word.to_string(), USER_WORD_FREQ));
        self.order.push((syllables.join("'"), word.to_string()));
        true
    }

    /// 在音节格上走用户词 trie,回调每一条命中词边:(起点字节, 终点字节, 词, 词频)。
    /// 与 Lexicon::for_each_word_edge 同构,词边在 segment 里同桶合并。
    pub(crate) fn for_each_edge<'a>(
        &'a self,
        lattice: &Lattice,
        mut f: impl FnMut(usize, usize, &'a str, u32),
    ) {
        if self.roots.is_empty() {
            return;
        }
        // None = 伪根(查 roots);Some = 已进 trie(查 children)
        let mut stack: Vec<(Option<&Node>, usize)> = Vec::new();
        for start in 0..lattice.text.len() {
            if lattice.starts[start].is_empty() {
                continue;
            }
            stack.push((None, start));
            while let Some((node, pos)) = stack.pop() {
                for &idx in &lattice.starts[pos] {
                    let (a, b) = lattice.syllables[idx];
                    let child = match node {
                        None => self.roots.get(&lattice.text[a..b]),
                        Some(n) => n.children.get(&lattice.text[a..b]),
                    };
                    let Some(c) = child else { continue };
                    for (word, freq) in &c.words {
                        f(start, b, word, *freq);
                    }
                    stack.push((Some(c), b));
                }
            }
        }
    }

    /// 全部词的行格式(`pin'yin 词 词频`),按插入序。
    pub(crate) fn to_lines(&self) -> String {
        let mut out = String::new();
        for (pinyin, word) in &self.order {
            out.push_str(&format!("{pinyin} {word} {USER_WORD_FREQ}\n"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syllable::build_lattice;
    use std::collections::HashSet;

    fn lattice_of(input: &str, sylls: &[&str]) -> Lattice {
        let valid: HashSet<String> = sylls.iter().map(|s| s.to_string()).collect();
        build_lattice(input, &valid)
    }

    #[test]
    fn insert_dedup_and_order() {
        let mut d = UserDict::default();
        assert!(d.insert(&["chi", "mei"], "魑魅"));
        assert!(!d.insert(&["chi", "mei"], "魑魅"), "同路径同文本幂等");
        assert!(d.insert(&["chi", "mei"], "魑眉"), "同路径异文本是新词");
        assert_eq!(
            d.to_lines(),
            format!("chi'mei 魑魅 {USER_WORD_FREQ}\nchi'mei 魑眉 {USER_WORD_FREQ}\n")
        );
    }

    #[test]
    fn edges_hit_only_exact_path() {
        let mut d = UserDict::default();
        d.insert(&["chi", "mei"], "魑魅");
        // 完整路径:命中
        let lat = lattice_of("chimei", &["chi", "mei"]);
        let mut hits = Vec::new();
        d.for_each_edge(&lat, |s, e, w, _| hits.push((s, e, w)));
        assert_eq!(hits, [(0, 6, "魑魅")]);
        // 只打首音节:中间节点不挂词,不误命中
        let lat = lattice_of("chi", &["chi", "mei"]);
        let mut hits = Vec::new();
        d.for_each_edge(&lat, |_, _, w, _| hits.push(w));
        assert!(hits.is_empty());
        // 无关音节:不命中
        let lat = lattice_of("wang", &["wang"]);
        let mut hits = Vec::new();
        d.for_each_edge(&lat, |_, _, w, _| hits.push(w));
        assert!(hits.is_empty());
    }
}
