//! DP 切分：在音节格（字节位置）上跑 unigram k-best，输出整句候选。
//!
//! 模型：路径得分 = Σ ln(词频 / 总词频)（jieba 词频即语料计数，取对数变可加）。
//! 长词天然占优：一个词的对数概率通常高于拆成两个单字的对数概率之和。
//! 每个终点位置只保留 BEAM 条路径防组合爆炸；最终按候选文本去重——
//! 不同切分/多音字可能殊途同归出同一串汉字。

use std::collections::HashSet;

use crate::dict::Lexicon;
use crate::syllable;

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
    let lattice = syllable::build_lattice(input, &lex.syllables);
    let len = lattice.text.len();
    if len == 0 {
        return Vec::new();
    }

    // 词边按终点归桶：incoming[end] = [(起点, 词, 词频)]
    let mut incoming: Vec<Vec<(usize, &str, u32)>> = vec![Vec::new(); len + 1];
    // 首词候选(逐字/逐词选择):位置 0 出发、未覆盖全输入的词边,记下消耗字节数。
    let mut prefix: Vec<(usize, &str, u32)> = Vec::new();
    lex.for_each_word_edge(&lattice, |start, end, words| {
        for (word, freq) in words.iter().take(EDGE_WORD_CAP) {
            incoming[end].push((start, word.as_str(), *freq));
            if start == 0 && end < len {
                prefix.push((end, word.as_str(), *freq));
            }
        }
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

    // 简拼候选:输入(音节格已去 ')作为声母 key 精确匹配。score 用同一
    // unigram 对数量纲 ln(freq/total),与全拼候选公平竞争。
    if let Some(jp) = lex.jianpin.get(&lattice.text) {
        for (word, freq) in jp {
            cands.push(Candidate {
                text: word.clone(),
                words: vec![word.clone()],
                score: (*freq as f64 / total).ln(),
                consumed: len,
            });
        }
    }

    // 首词候选并入(consumed<len 标记部分消耗,供逐字/逐词选择)。
    for (end, word, freq) in prefix {
        cands.push(Candidate {
            text: word.to_string(),
            words: vec![word.to_string()],
            score: (freq as f64 / total).ln(),
            consumed: end,
        });
    }

    // 统一去重:保留先出现者(DP 全拼在前,优先于同文本的简拼/首词)。
    let mut seen = HashSet::new();
    cands.retain(|c| seen.insert(c.text.clone()));

    // 排序:静态 score + 用户调频增量(无用户数据时增量 ln(1)=0)。分两组:
    // 整句候选(消耗全部拼音)在前——多字输入主选整句;首词候选(部分消耗)
    // 在后附加,供逐字/逐词选择。score 字段保持原始对数概率不被污染,只影响次序。
    let (mut full, mut rest): (Vec<(f64, Candidate)>, Vec<(f64, Candidate)>) = cands
        .into_iter()
        .map(|c| {
            let boost = lex.user_freq.get(&c.text).copied().unwrap_or(0);
            (c.score + (1.0 + boost as f64).ln() * USER_W, c)
        })
        .partition(|(_, c)| c.consumed >= len);
    full.sort_by(|a, b| b.0.total_cmp(&a.0));
    rest.sort_by(|a, b| b.0.total_cmp(&a.0));
    full.into_iter().chain(rest).take(limit).map(|(_, c)| c).collect()
}

/// 用户调频权重:被选 1 次等效于静态词频自然对数提升 USER_W 倍。
/// 静态概率差距可用到 ~7(低频词 vs 万频词),选 3 次需 ln(4)·6≈8.3 才能翻越,
/// 故 USER_W=6 使"选过的低频词 3 次上浮到首位"成立;过高会让单次选择产生跳变。
const USER_W: f64 = 6.0;

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Lexicon {
        Lexicon::from_lines(
            "ni 你 500\n\
             hao 好 300\n\
             ni'hao 你好 10000\n\
             ni'hao 泥蒿 5\n\
             xi 西 300\n\
             an 安 400\n\
             xi'an 西安 8000\n\
             xian 先 9000\n",
        )
    }

    #[test]
    fn nihao_top_is_ni_hao() {
        let cands = convert(&fixture(), "nihao", 9);
        assert_eq!(cands[0].text, "你好");
        assert!(cands.iter().any(|c| c.text == "泥蒿"), "低频词也应在候选中: {cands:?}");
        assert_eq!(cands.iter().filter(|c| c.text == "你好").count(), 1, "同文本候选须去重");
    }

    #[test]
    fn apostrophe_blocks_single_syllable() {
        let cands = convert(&fixture(), "xi'an", 9);
        assert_eq!(cands[0].text, "西安");
        assert!(!cands.iter().any(|c| c.text == "先"));
    }

    #[test]
    fn bare_xian_is_ambiguous() {
        let cands = convert(&fixture(), "xian", 9);
        assert_eq!(cands[0].text, "先", "高频单音节词应胜出");
        assert!(cands.iter().any(|c| c.text == "西安"));
    }

    #[test]
    fn unknown_input_yields_nothing() {
        assert!(convert(&fixture(), "zzz", 9).is_empty());
    }

    #[test]
    fn prefix_candidates_enable_char_by_char() {
        let cands = convert(&fixture(), "nihao", 90);
        // 整句候选消耗全部拼音(5 字节 nihao)。
        let full = cands.iter().find(|c| c.text == "你好").unwrap();
        assert_eq!(full.consumed, 5, "整句候选消耗全部拼音");
        // 首词候选"你"只消耗 ni(2 字节),供逐字选择。
        let zi = cands.iter().find(|c| c.text == "你").unwrap();
        assert_eq!(zi.consumed, 2, "首词候选只消耗第一音节");
        // 整句排在首词前(多字输入主选整句)。
        let pos_full = cands.iter().position(|c| c.text == "你好").unwrap();
        let pos_zi = cands.iter().position(|c| c.text == "你").unwrap();
        assert!(pos_full < pos_zi, "整句候选应排在首词前: {cands:?}");
    }

    #[test]
    fn user_freq_boosts_repeatedly_picked_word() {
        let mut lex = fixture();
        // 你好 10000 本就第一;选低频"泥蒿"(5)三次,应被顶到首位
        lex.user_freq.insert("泥蒿".to_string(), 3);
        let cands = convert(&lex, "nihao", 9);
        assert_eq!(cands[0].text, "泥蒿", "选过 3 次的低频词应上浮到首位: {cands:?}");
    }

    #[test]
    fn single_selection_does_not_dethrone_high_freq() {
        let mut lex = fixture();
        // 选 1 次的"泥蒿"不足以压过静态 10000 的"你好"
        lex.user_freq.insert("泥蒿".to_string(), 1);
        let cands = convert(&lex, "nihao", 9);
        assert_eq!(cands[0].text, "你好");
    }
    #[test]
    fn jianpin_matches_shengmu_key() {
        let lex = Lexicon::from_lines("ni'hao 你好 10000\nni 你 500\nhao 好 300\n");
        // "nh" 不是合法音节序列(全拼 DP 无候选),简拼索引精确命中 你好
        let cands = convert(&lex, "nh", 9);
        assert_eq!(cands.first().map(|c| c.text.as_str()), Some("你好"));
        // 全拼路径不受影响
        assert_eq!(convert(&lex, "nihao", 9)[0].text, "你好");
        // 无声母 key 的输入仍为空
        assert!(convert(&lex, "zzz", 9).is_empty());
    }
}
