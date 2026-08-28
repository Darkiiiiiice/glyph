//! 词典：音节 trie，池化紧凑布局（1.39M 词条,daemon RSS ~1.0GB → ~0.2GB,实测见 PLAN.md）。
//!
//! lexicon 文件行格式（glyph-build 生成）：`pin'yin 词 词频`，空白分隔。
//! 词库加载后同时派生出两样东西：
//! - `syllables`：全部出现过的音节集合 → 音节格的合法性判据；
//! - `total_freq`：词频总和 → unigram 概率的分母。
//!
//! 紧凑化设计：
//! - 音节全表仅 ~410 个，trie 边存 u16 音节编号而非字符串；
//! - 构建零树形结构:每行解析成定长记录进大 Vec,按音节路径稳定排序后
//!   一遍递归展平成节点池/边池/词条池/文本 arena——旧实现每节点一个
//!   HashMap、200 万个小堆块与构建垃圾交错,页稀疏把 RSS 顶在 2x 活数据,
//!   换分配器无效,只能让构建期也不产生小堆块;
//! - 词文本全部进一个 String arena,词条存 (偏移, 长度, 词频);
//! - 简拼索引条目只存音节路径,词文本/词频查询时沿路径走 trie 解析。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

use crate::syllable::Lattice;
use crate::user_dict::UserDict;

mod build;

/// 音节编号：边池与简拼路径的键。音节表仅数百项，u16 足够。
pub(crate) type SyllId = u16;

/// 节点槽：子边与词条分别是边池/词条池中的连续段。下标 0 是根。
#[derive(Default, Clone, Copy)]
struct NodeSlot {
    child_off: u32,
    child_len: u32,
    word_off: u32,
    word_len: u32,
}

/// 词条槽：词文本在 texts arena 中的切片 + 词频。
#[derive(Clone, Copy)]
struct WordSlot {
    text_off: u32,
    text_len: u32,
    freq: u32,
}

/// 一条词边（trie 某节点挂的词列表）的只读视图。
pub(crate) struct WordEdges<'a> {
    slots: &'a [WordSlot],
    texts: &'a str,
}

impl<'a> WordEdges<'a> {
    /// (词文本, 词频)，按词频降序。
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&'a str, u32)> + '_ {
        self.slots.iter().map(|w| {
            let (a, b) = (w.text_off as usize, (w.text_off + w.text_len) as usize);
            (&self.texts[a..b], w.freq)
        })
    }
}

pub struct Lexicon {
    nodes: Vec<NodeSlot>,
    /// (音节编号, 子节点下标)，每个节点的子边连续且按编号有序。
    edges: Vec<(SyllId, u32)>,
    words: Vec<WordSlot>,
    /// 全部词文本的单一 arena。
    texts: String,
    pub syllables: HashSet<String>,
    /// 音节 → 编号：查询侧把音节格上的音节串翻成边池的键。
    syllable_ids: HashMap<String, SyllId>,
    /// 编号 → 音节串（id 即下标）,finish 时从 syllable_ids 反转生成。
    syllable_strs: Vec<String>,
    pub total_freq: u64,
    /// 用户词频:整句候选 text → 被选择次数。动态调频层,不混入 trie/total_freq。
    pub user_freq: HashMap<String, u32>,
    /// 简拼/混合拼索引:声母 key(你好→nh、中国→zhg) → jp_entries 中的连续段;
    /// 每条目又是 paths 池的一段音节路径(finish 时桶内排序去重)。
    jianpin: HashMap<String, (u32, u32)>,
    jp_entries: Vec<(u32, u32)>,
    paths: Vec<SyllId>,
    /// 用户二元搭配(bigram):上一次上屏的尾词 → {当前词 → 搭配次数}。
    /// 冷启动从用户输入历史积累,无外部语料;嵌套 map 使查询免 tuple 分配。
    pub user_bigram: HashMap<String, HashMap<String, u32>>,
    /// 用户三元搭配(trigram):上上文 → {上文 → {当前词 → 搭配次数}}。
    /// 双词上下文比单词更特异,与 bigram 并行积累、查询时取两者增量 max(见 segment)。
    pub user_trigram: HashMap<String, HashMap<String, HashMap<String, u32>>>,
    /// 模糊音等价类:音节编号 → 等价音节集(含自身,有序)。默认空表 = 精确匹配;
    /// set_fuzzy 后填满(未受影响音节为自身单元素),for_each_word_edge 逐枚子边。
    fuzzy: Vec<Vec<SyllId>>,
    /// 用户造词 overlay(逐字序列学成的词):运行期可插,与池化 trie 并查。
    pub(crate) user_words: UserDict,
}

impl Lexicon {
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut b = build::Builder::default();
        for (lineno, line) in BufReader::new(File::open(path)?).lines().enumerate() {
            let line = line?;
            if line.is_empty() {
                continue;
            }
            b.insert_line(&line).map_err(|msg| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{}:{}: {}", path.display(), lineno + 1, msg),
                )
            })?;
        }
        Ok(b.finish())
    }

    /// 从行格式文本构建(测试与 Engine::from_str 用)。
    pub fn from_lines(lines: &str) -> Self {
        let mut b = build::Builder::default();
        for line in lines.lines().filter(|l| !l.is_empty()) {
            b.insert_line(line).unwrap();
        }
        b.finish()
    }

    /// 编号 → 音节串。
    pub(crate) fn syll_str(&self, id: SyllId) -> &str {
        &self.syllable_strs[id as usize]
    }

    /// 音节串 → 编号(用户造词的词库查重用;不在音节表 = 永远切不出候选)。
    pub(crate) fn syllable_id(&self, syll: &str) -> Option<SyllId> {
        self.syllable_ids.get(syll).copied()
    }

    /// 把模糊音规则对展开成音节等价类。规则是声母("z"/"zh")或韵母("an"/"ang")
    /// 片段,前缀/后缀替换双向应用(z↔zh 互找、an↔ang 互找),再取传递闭包
    /// (z=zh + an=ang 时 zan↔zhan↔zang↔zhang 一类)。空片段/音节表外的规则静默忽略。
    /// 等价无惩罚:模糊命中与精确命中同权,按词频竞争(与主流输入法一致)。
    pub(crate) fn set_fuzzy(&mut self, rules: &[(&str, &str)]) {
        let n = self.syllable_strs.len();
        self.fuzzy = (0..n).map(|i| vec![i as SyllId]).collect();
        // 规则 → 音节伙伴边
        let mut partners: Vec<HashSet<SyllId>> = vec![HashSet::new(); n];
        for &(a, b) in rules {
            if a.is_empty() || b.is_empty() {
                continue;
            }
            for (s, &id) in &self.syllable_ids {
                for (from, to) in [(a, b), (b, a)] {
                    if let Some(t) = replace_affix(s, from, to) {
                        if let Some(&tid) = self.syllable_ids.get(&t) {
                            partners[id as usize].insert(tid);
                        }
                    }
                }
            }
        }
        // 洪泛求连通分量,>1 的分量写成等价类(每成员映射到整类)
        let mut done = vec![false; n];
        for i in 0..n {
            if done[i] {
                continue;
            }
            let mut class = vec![i as SyllId];
            let mut stack = vec![i];
            done[i] = true;
            while let Some(x) = stack.pop() {
                for &p in &partners[x] {
                    if !done[p as usize] {
                        done[p as usize] = true;
                        class.push(p);
                        stack.push(p as usize);
                    }
                }
            }
            if class.len() > 1 {
                class.sort_unstable();
                for &m in &class {
                    self.fuzzy[m as usize] = class.clone();
                }
            }
        }
    }

    /// 节点的子节点(按音节编号二分)。
    fn child(&self, ni: u32, id: SyllId) -> Option<u32> {
        let n = &self.nodes[ni as usize];
        let es = &self.edges[n.child_off as usize..(n.child_off + n.child_len) as usize];
        es.binary_search_by_key(&id, |&(s, _)| s).ok().map(|i| es[i].1)
    }

    fn word_edges(&self, ni: u32) -> WordEdges<'_> {
        let n = &self.nodes[ni as usize];
        WordEdges {
            slots: &self.words[n.word_off as usize..(n.word_off + n.word_len) as usize],
            texts: &self.texts,
        }
    }

    /// 按音节路径走 trie,返回终点节点挂的词边(简拼桶解析用)。
    /// 路径来自词库构建,必合法;中间断裂防御性返回 None。
    pub(crate) fn words_at_path(&self, path: &[SyllId]) -> Option<WordEdges<'_>> {
        let mut ni = 0;
        for &id in path {
            ni = self.child(ni, id)?;
        }
        Some(self.word_edges(ni))
    }

    /// 简拼/混合拼桶:声母 key → 桶内各音节路径。
    pub(crate) fn jianpin_bucket(&self, key: &str) -> Option<impl Iterator<Item = &[SyllId]> + '_> {
        let &(off, len) = self.jianpin.get(key)?;
        Some(
            self.jp_entries[off as usize..(off + len) as usize]
                .iter()
                .map(|&(o, l)| &self.paths[o as usize..(o + l) as usize]),
        )
    }

    /// 在音节格上走 trie，回调每一条命中的词边：（起点字节, 终点字节, 词条列表）。
    /// 从每个有音节出发的字节位置各走一次；同一（起, 终）可能因多音字词条
    /// 出现多次，由上层按候选文本去重。
    pub fn for_each_word_edge<'a>(
        &'a self,
        lattice: &Lattice,
        mut f: impl FnMut(usize, usize, WordEdges<'a>),
    ) {
        let mut stack: Vec<(u32, usize)> = Vec::new();
        for start in 0..lattice.text.len() {
            if lattice.starts[start].is_empty() {
                continue;
            }
            stack.push((0, start));
            while let Some((ni, pos)) = stack.pop() {
                for &idx in &lattice.starts[pos] {
                    let (a, b) = lattice.syllables[idx];
                    // 音节表即由本词库构建,必命中;取不到防御性剪枝
                    let Some(&id) = self.syllable_ids.get(&lattice.text[a..b]) else {
                        continue;
                    };
                    // 模糊音:等价类(含自身)逐枚走子边;空表 = 仅自身(精确匹配)
                    let ids = self.fuzzy.get(id as usize).map_or(std::slice::from_ref(&id), Vec::as_slice);
                    for &fid in ids {
                        if let Some(ci) = self.child(ni, fid) {
                            if self.nodes[ci as usize].word_len > 0 {
                                f(start, b, self.word_edges(ci));
                            }
                            stack.push((ci, b));
                        }
                    }
                }
            }
        }
    }
}

/// 前/后缀替换:`s` 以 `from` 开头则换成 `to`(声母规则),否则以 `from` 结尾则换(韵母规则)。
fn replace_affix(s: &str, from: &str, to: &str) -> Option<String> {
    if let Some(rest) = s.strip_prefix(from) {
        Some(format!("{to}{rest}"))
    } else {
        s.strip_suffix(from).map(|head| format!("{head}{to}"))
    }
}

/// 音节的声母(简拼 key 用):zh/ch/sh 为双字母整体,其余取首字母
/// (零声母音节 a/o/e 开头取首字母,如 an→a、ou→o)。
pub(crate) fn shengmu(syll: &str) -> &str {
    if syll.is_empty() {
        return ""; // 保险:insert_line 已拒空音节(拼音含空音节→Err),此处仅防御
    }
    let b = syll.as_bytes();
    if b.len() >= 2 && matches!(&b[..2], b"zh" | b"ch" | b"sh") { &syll[..2] } else { &syll[..1] }
}

#[cfg(test)]
mod tests;
