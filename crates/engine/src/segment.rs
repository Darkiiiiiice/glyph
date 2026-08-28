//! DP 切分：在音节格（字节位置）上跑 unigram k-best，输出整句候选。
//!
//! 模型：路径得分 = Σ ln(词频 / 总词频)（jieba 词频即语料计数，取对数变可加）。
//! 长词天然占优：一个词的对数概率通常高于拆成两个单字的对数概率之和。
//! 每个终点位置只保留 BEAM 条路径防组合爆炸；最终按候选文本去重——
//! 不同切分/多音字可能殊途同归出同一串汉字。

use std::collections::HashSet;

use crate::dict::{Lexicon, SyllId, shengmu};
use crate::syllable::{self, Lattice};

/// 一个整句候选。
#[derive(Debug)]
pub struct Candidate {
    /// 拼接后的中文（候选展示的就是它）。
    pub text: String,
    /// 分词路径（调试用；同 text 的多条路径已去重，只留得分最高者）。
    pub words: Vec<String>,
    /// 对数概率得分，越大越好。
    pub score: f64,
    /// 该候选消耗的拼音字节数（相对去掉 `'` 的输入）。整句候选 = 输入全长；
    /// 首词候选 < 全长——选中后只消耗前缀拼音，剩余继续组字（逐字/逐词选择）。
    pub consumed: usize,
}

/// 每个字节位置保留的最大路径数。
const BEAM: usize = 100;

/// 单条词边参与 DP 的最大词条数。单音节节点（如 yi）挂着上百个同音字，
/// 全展开会把 beam 挤满低频字；词频排序后截断即可，长尾几乎不可能胜出。
const EDGE_WORD_CAP: usize = 32;

pub fn convert(lex: &Lexicon, input: &str, limit: usize) -> Vec<Candidate> {
    convert_ctx(lex, input, limit, &[])
}

/// 带上文的转换:`ctx` 最近优先(ctx[0]=上一次上屏尾词、ctx[1]=上上次);候选首词与上文
/// 有用户搭配记录时上浮:bigram 用 ctx[0],trigram 用 ctx[..2](双词更特异,见 TRIGRAM_W)。
pub fn convert_ctx(lex: &Lexicon, input: &str, limit: usize, ctx: &[&str]) -> Vec<Candidate> {
    let lattice = syllable::build_lattice(input, &lex.syllables);
    let len = lattice.text.len();
    if len == 0 {
        return Vec::new();
    }

    // 词边按终点归桶：incoming[end] = [(起点, 词, 词频)]
    let mut incoming: Vec<Vec<(usize, &str, u32)>> = vec![Vec::new(); len + 1];
    lex.for_each_word_edge(&lattice, |start, end, words| {
        for (word, freq) in words.iter().take(EDGE_WORD_CAP) {
            incoming[end].push((start, word, freq));
        }
    });
    // 用户造词 overlay:词边同桶进 DP(量级千级,不过 EDGE_WORD_CAP)。
    lex.user_words.for_each_edge(&lattice, |start, end, word, freq| {
        incoming[end].push((start, word, freq));
    });

    // 前向 DP：dp[pos] = 以字节 pos 结尾的最佳若干（得分, 词路径）。
    let total = lex.total_freq as f64;
    let mut dp: Vec<Vec<(f64, Vec<&str>)>> = vec![Vec::new(); len + 1];
    dp[0].push((0.0, Vec::new()));
    for end in 1..=len {
        let mut merged: Vec<(f64, Vec<&str>)> = Vec::new();
        for &(start, word, freq) in &incoming[end] {
            let weight = (freq as f64 / total).ln();
            for (prev_score, prev_path) in &dp[start] {
                let mut path = prev_path.clone();
                path.push(word);
                merged.push((prev_score + weight, path));
            }
        }
        merged.sort_by(|a, b| b.0.total_cmp(&a.0));
        merged.truncate(BEAM);
        dp[end] = merged;
    }

    // DP 全拼候选(先不去重,留待与简拼统一处理)。
    let mut cands: Vec<Candidate> = dp[len]
        .iter()
        .map(|(score, words)| Candidate {
            text: words.concat(),
            words: words.iter().map(|w| w.to_string()).collect(),
            score: *score,
            consumed: len,
        })
        .collect();

    // 简拼/混合拼候选:枚举输入的"音节或声母"槽位切分(lij→[li][j],
    // 纯声母 nh 是全 None 的特例),声母 key 定位索引桶;桶内存音节路径,
    // 按音节槽精确过滤后走 trie 解析出词与词频(逻辑 luo'ji 声母同为 lj
    // 但首槽≠li,被滤掉)。score 与全拼同量纲公平竞争。
    for (key, slots) in mixed_patterns(&lattice) {
        if let Some(jp) = lex.jianpin_bucket(&key) {
            for path in jp {
                if !slots_match(lex, path, &slots) {
                    continue;
                }
                let Some(words) = lex.words_at_path(path) else { continue };
                for (word, freq) in words.iter() {
                    cands.push(Candidate {
                        text: word.to_string(),
                        words: vec![word.to_string()],
                        score: (freq as f64 / total).ln(),
                        consumed: len,
                    });
                }
            }
        }
    }

    // 统一去重:保留先出现者(DP 全拼在前,优先于同文本的简拼)。
    let mut seen = HashSet::new();
    cands.retain(|c| seen.insert(c.text.clone()));

    // 排序:静态 score + 用户调频增量 + 上下文增量(无数据时增量 ln(1)=0)。
    // 上下文取 bigram/trigram 增量 max 而非相加:同一次上屏会同时写两条记录,
    // 相加等于把一份证据记两次;max 保留各自独立积累中更强者。
    // score 字段保持原始对数概率不被污染,只影响次序。
    let bigram_map = ctx.first().and_then(|p| lex.user_bigram.get(*p));
    let trigram_map = if ctx.len() >= 2 { lex.user_trigram.get(ctx[1]).and_then(|m| m.get(ctx[0])) } else { None };
    let mut ranked: Vec<(f64, Candidate)> = cands
        .into_iter()
        .map(|c| {
            let boost = lex.user_freq.get(&c.text).copied().unwrap_or(0);
            let first = c.words.first().map(|w| w.as_str());
            let bi = bigram_map.and_then(|m| first.and_then(|w| m.get(w))).copied().unwrap_or(0);
            let tri = trigram_map.and_then(|m| first.and_then(|w| m.get(w))).copied().unwrap_or(0);
            let ctx_boost = ((1.0 + bi as f64).ln() * BIGRAM_W).max((1.0 + tri as f64).ln() * TRIGRAM_W);
            (c.score + (1.0 + boost as f64).ln() * USER_W + ctx_boost, c)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked.into_iter().take(limit).map(|(_, c)| c).collect()
}

/// 混合拼槽位切分枚举:把输入切成 ≥2 个槽,每槽是完整音节(精确,取自音节格)
/// 或声母(模糊;zh/ch/sh 双字母,或单字母声母/零声母 a/e/o)。
/// 返回 (声母 key, 各槽精确音节)。i/u/v 不能作声母,相应切分自然剪枝。
fn mixed_patterns(lattice: &Lattice) -> Vec<(String, Vec<Option<&str>>)> {
    fn dfs<'a>(
        lattice: &'a Lattice,
        pos: usize,
        key: &mut String,
        slots: &mut Vec<Option<&'a str>>,
        out: &mut Vec<(String, Vec<Option<&'a str>>)>,
    ) {
        let text = lattice.text.as_str();
        if pos == text.len() {
            if slots.len() >= 2 {
                out.push((key.clone(), slots.clone()));
            }
            return;
        }
        let rest = &text[pos..];
        let b = rest.as_bytes()[0];
        if matches!(b, b'b' | b'p' | b'm' | b'f' | b'd' | b't' | b'n' | b'l' | b'g' | b'k'
            | b'h' | b'j' | b'q' | b'x' | b'r' | b'z' | b'c' | b's' | b'y' | b'w'
            | b'a' | b'e' | b'o')
        {
            key.push(b as char);
            slots.push(None);
            dfs(lattice, pos + 1, key, slots, out);
            slots.pop();
            key.pop();
            if rest.len() >= 2 && matches!(&rest[..2], "zh" | "ch" | "sh") {
                key.push_str(&rest[..2]);
                slots.push(None);
                dfs(lattice, pos + 2, key, slots, out);
                slots.pop();
                key.truncate(key.len() - 2);
            }
        }
        for &idx in &lattice.starts[pos] {
            let (a, end) = lattice.syllables[idx];
            let syll = &text[a..end];
            let sm = shengmu(syll);
            key.push_str(sm);
            slots.push(Some(syll));
            dfs(lattice, end, key, slots, out);
            slots.pop();
            key.truncate(key.len() - sm.len());
        }
    }
    let mut out = Vec::new();
    dfs(lattice, 0, &mut String::new(), &mut Vec::new(), &mut out);
    out
}

/// 音节路径是否满足槽位约束:槽数相等,精确槽音节相同
/// (声母槽的声母相等已由 key 命中保证,无需再查)。
fn slots_match(lex: &Lexicon, path: &[SyllId], slots: &[Option<&str>]) -> bool {
    if path.len() != slots.len() {
        return false;
    }
    for (slot, &id) in slots.iter().zip(path) {
        if let Some(exact) = slot {
            if lex.syll_str(id) != *exact {
                return false;
            }
        }
    }
    true
}

/// Tab 单字模式:第一音节的全部单字候选(逐字定字)。
/// 覆盖所有合法首音节切分(xian→xi/xian),只收单字;consumed=该首音节字节数,
/// 选中后截掉续打剩余拼音。排序同 convert(静态词频 + 用户调频)。
pub fn first_syllable_chars(lex: &Lexicon, input: &str, limit: usize) -> Vec<Candidate> {
    let lattice = syllable::build_lattice(input, &lex.syllables);
    if lattice.text.is_empty() {
        return Vec::new();
    }
    let total = lex.total_freq as f64;
    let mut seen = HashSet::new();
    let mut cands: Vec<Candidate> = Vec::new();
    lex.for_each_word_edge(&lattice, |start, end, words| {
        if start != 0 {
            return; // 只要第一音节(位置 0 出发的边)
        }
        for (word, freq) in words.iter() {
            // 只收单字;同一字若出现在多个首音节切分下,按文本去重
            if word.chars().count() == 1 && seen.insert(word.to_string()) {
                cands.push(Candidate {
                    text: word.to_string(),
                    words: vec![word.to_string()],
                    score: (freq as f64 / total).ln(),
                    consumed: end,
                });
            }
        }
    });
    let mut ranked: Vec<(f64, Candidate)> = cands
        .into_iter()
        .map(|c| {
            let boost = lex.user_freq.get(&c.text).copied().unwrap_or(0);
            (c.score + (1.0 + boost as f64).ln() * USER_W, c)
        })
        .collect();
    // 长音节(consumed 大)的字优先:打 xuan 定 xuan 的字,短切分 xu 的高频字(需/须)靠后;
    // 同一切分内按得分(静态词频+用户调频)。
    ranked.sort_by(|a, b| b.1.consumed.cmp(&a.1.consumed).then_with(|| b.0.total_cmp(&a.0)));
    ranked.into_iter().take(limit).map(|(_, c)| c).collect()
}

/// 用户调频权重:被选 1 次等效于静态词频自然对数提升 USER_W 倍。
/// 静态概率差距可用到 ~7(低频词 vs 万频词),选 3 次需 ln(4)·6≈8.3 才能翻越,
/// 故 USER_W=6 使"选过的低频词 3 次上浮到首位"成立;过高会让单次选择产生跳变。
const USER_W: f64 = 6.0;

/// bigram 上文搭配权重:候选首词与上一次上屏尾词的用户搭配次数的对数增量。
/// 冷启动积累,与 USER_W 同量纲——搭配几次即可压过纯词频序,且被 ln 压顶避免霸榜。
const BIGRAM_W: f64 = 6.0;

/// trigram 双词上文搭配权重:候选首词与(上上文,上一词)的用户搭配次数的对数增量。
/// 双词上下文比单词更特异:1 次搭配应翻 p99 静态 gap(6.78),10·ln2≈6.93 恰好压过。
const TRIGRAM_W: f64 = 10.0;

#[cfg(test)]
mod tests;
