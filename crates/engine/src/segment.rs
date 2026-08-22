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
    lex.for_each_word_edge(&lattice, |start, end, words| {
        for (word, freq) in words.iter().take(EDGE_WORD_CAP) {
            incoming[end].push((start, word.as_str(), *freq));
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

    let mut seen = HashSet::new();
    dp[len]
        .iter()
        .map(|(score, words)| Candidate {
            text: words.concat(),
            words: words.iter().map(|w| w.to_string()).collect(),
            score: *score,
        })
        .filter(|c| seen.insert(c.text.clone()))
        .take(limit)
        .collect()
}

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
}
