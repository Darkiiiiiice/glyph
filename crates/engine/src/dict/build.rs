//! 词库构建管线：行解析 → 定长记录大 Vec → 按音节路径稳定排序 → 一遍递归
//! 展平成池化 trie。构建期零树形小堆块——200 万个小分配与构建垃圾交错造成
//! 页稀疏(RSS 顶在 2x 活数据)是旧实现的病根,换分配器无效,只能让构建期
//! 也不产生小堆块。

use std::collections::{HashMap, HashSet};

use super::{Lexicon, NodeSlot, SyllId, WordSlot, shengmu};

/// 解析期的一行记录：路径与文本都是池中的切片，零内嵌堆分配。
/// 全部记录进一个大 Vec，排序后直接展平成 trie 池。
#[derive(Clone, Copy)]
struct Rec {
    path_off: u32,
    path_len: u16,
    freq: u32,
    text_off: u32,
    text_len: u32,
}

/// 词库构建器：`insert_line` 往四个大池追加，`finish` 排序展平出 Lexicon。
#[derive(Default)]
pub(super) struct Builder {
    records: Vec<Rec>,
    /// 全部记录的音节路径拼接池(定序前可增长,记录只存区间)。
    paths_buf: Vec<SyllId>,
    /// 词文本 arena(即成品 texts,构建期直接写终态)。
    texts: String,
    syllables: HashSet<String>,
    syllable_ids: HashMap<String, SyllId>,
    total_freq: u64,
    /// 简拼桶:声母 key → 路径区间(paths_buf 的 (off, len);finish 排序去重再摊平)。
    /// 不存记录下标:finish 会按路径重排 records,下标会失效。
    jianpin: HashMap<String, Vec<(u32, u16)>>,
}

impl Builder {
    pub(super) fn insert_line(&mut self, line: &str) -> Result<(), String> {
        let mut fields = line.split_whitespace();
        let pinyin = fields.next().ok_or("缺拼音列")?;
        let word = fields.next().ok_or("缺词列")?;
        let freq: u32 =
            fields.next().and_then(|s| s.parse().ok()).ok_or("缺词频列")?;
        let path_off = self.paths_buf.len() as u32;
        let mut path_len = 0u16;
        for syll in pinyin.split('\'') {
            if syll.is_empty() {
                return Err(format!("拼音含空音节: {pinyin}"));
            }
            // 音节 → 编号,首次见到时注册(音节表与 id 表同步生长)
            let id = match self.syllable_ids.get(syll) {
                Some(&id) => id,
                None => {
                    let id = self.syllable_ids.len() as SyllId;
                    self.syllable_ids.insert(syll.to_string(), id);
                    self.syllables.insert(syll.to_string());
                    id
                }
            };
            self.paths_buf.push(id);
            path_len += 1;
        }
        let text_off = self.texts.len() as u32;
        self.texts.push_str(word);
        self.records.push(Rec {
            path_off,
            path_len,
            freq,
            text_off,
            text_len: word.len() as u32,
        });
        self.total_freq += u64::from(freq);
        // 多字词建简拼索引:各音节声母连成 key;条目存路径区间(轻量)。
        if path_len >= 2 {
            let key: String = pinyin.split('\'').map(shengmu).collect();
            self.jianpin.entry(key).or_default().push((path_off, path_len));
        }
        Ok(())
    }

    pub(super) fn finish(mut self) -> Lexicon {
        // 按音节路径字典序稳定排序:同一节点的词相邻(保持文件序),
        // 子树连续 → 递归一遍即可展平成池。
        let Builder { records, paths_buf, .. } = &mut self;
        records.sort_by(|a, b| {
            paths_buf[a.path_off as usize..(a.path_off + u32::from(a.path_len)) as usize]
                .cmp(&paths_buf[b.path_off as usize..(b.path_off + u32::from(b.path_len)) as usize])
        });
        let mut p = Pools {
            nodes: Vec::new(),
            edges: Vec::new(),
            words: Vec::with_capacity(records.len()),
        };
        build_node(records, 0, paths_buf, &mut p);
        // 简拼摊平:桶内按路径排序去重(同音词的路径重复),接入 paths/jp_entries 池。
        // 桶内顺序与候选排名无关(segment 收集后按 score 统一重排)。
        let mut paths = Vec::new();
        let mut jp_entries = Vec::new();
        let mut jianpin = HashMap::with_capacity(self.jianpin.len());
        // 先取出 map:桶内排序/去重要回查 paths_buf,不能带着部分移动借 self。
        let jp_map = std::mem::take(&mut self.jianpin);
        let pb = &self.paths_buf;
        let slice = |(o, l): (u32, u16)| &pb[o as usize..(o + u32::from(l)) as usize];
        for (key, mut bucket) in jp_map {
            bucket.sort_by(|&a, &b| slice(a).cmp(slice(b)));
            bucket.dedup_by(|&mut a, &mut b| slice(a) == slice(b));
            let off = jp_entries.len() as u32;
            for range in bucket {
                let path = slice(range);
                let po = paths.len() as u32;
                jp_entries.push((po, path.len() as u32));
                paths.extend_from_slice(path);
            }
            jianpin.insert(key, (off, jp_entries.len() as u32 - off));
        }
        // id → 音节串的反查表(id 即下标),混合拼槽过滤用。
        let mut syllable_strs = vec![String::new(); self.syllable_ids.len()];
        for (s, id) in &self.syllable_ids {
            syllable_strs[*id as usize] = s.clone();
        }
        // 构建垃圾集中在少数大分配里,释放后堆顶连续空闲,trim 有效。
        #[cfg(target_env = "gnu")]
        unsafe {
            extern "C" { fn malloc_trim(pad: usize) -> i32; }
            malloc_trim(0);
        }
        Lexicon {
            nodes: p.nodes,
            edges: p.edges,
            words: p.words,
            texts: self.texts,
            syllables: self.syllables,
            syllable_ids: self.syllable_ids,
            syllable_strs,
            total_freq: self.total_freq,
            user_freq: HashMap::new(),
            jianpin,
            jp_entries,
            paths,
            user_bigram: HashMap::new(),
            user_trigram: HashMap::new(),
            fuzzy: Vec::new(),
            user_words: crate::user_dict::UserDict::default(),
        }
    }
}

/// finish 展平时的目标池集合(单独成 struct 以满足借用检查)。
struct Pools {
    nodes: Vec<NodeSlot>,
    edges: Vec<(SyllId, u32)>,
    words: Vec<WordSlot>,
}

/// 把排好序的一段记录展平成子树,返回本节点下标。
/// recs 已按路径字典序:前导 path_len == depth 的记录是本节点的词,
/// 其余按 path[depth] 分组为子树区间(同组必相邻)。
fn build_node(recs: &mut [Rec], depth: usize, pb: &[SyllId], p: &mut Pools) -> u32 {
    let idx = p.nodes.len() as u32;
    p.nodes.push(NodeSlot::default()); // 占位,范围在子树展平后回填
    // 本节点的词:就地把前导组按词频降序排序(稳定,平频保持文件序)后入池。
    let mut nw = 0;
    while nw < recs.len() && recs[nw].path_len as usize == depth {
        nw += 1;
    }
    recs[..nw].sort_by(|a, b| b.freq.cmp(&a.freq));
    let word_off = p.words.len() as u32;
    for r in &recs[..nw] {
        p.words.push(WordSlot { text_off: r.text_off, text_len: r.text_len, freq: r.freq });
    }
    let word_len = p.words.len() as u32 - word_off;
    let rest = &mut recs[nw..];
    // 子边占位(两遍扫描,不为每节点分配临时 Vec):第一遍登记区间。
    let child_off = p.edges.len() as u32;
    let mut child_len = 0u32;
    let mut i = 0;
    while i < rest.len() {
        let s = pb[rest[i].path_off as usize + depth];
        p.edges.push((s, 0));
        child_len += 1;
        while i < rest.len() && pb[rest[i].path_off as usize + depth] == s {
            i += 1;
        }
    }
    // 第二遍重放同样的分组,递归并回填子节点下标。
    let mut k = 0usize;
    let mut j = 0;
    while j < rest.len() {
        let s = pb[rest[j].path_off as usize + depth];
        let mut e = j + 1;
        while e < rest.len() && pb[rest[e].path_off as usize + depth] == s {
            e += 1;
        }
        let cidx = build_node(&mut rest[j..e], depth + 1, pb, p);
        p.edges[child_off as usize + k].1 = cidx;
        k += 1;
        j = e;
    }
    p.nodes[idx as usize] = NodeSlot { child_off, child_len, word_off, word_len };
    idx
}
