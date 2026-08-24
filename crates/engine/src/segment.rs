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
    convert_ctx(lex, input, limit, None)
}

/// 带上文(bigram)的转换:`prev` 是上一次上屏的尾词;候选首词与其有用户搭配记录时额外上浮。
pub fn convert_ctx(lex: &Lexicon, input: &str, limit: usize, prev: Option<&str>) -> Vec<Candidate> {
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

    // 统一去重:保留先出现者(DP 全拼在前,优先于同文本的简拼)。
    let mut seen = HashSet::new();
    cands.retain(|c| seen.insert(c.text.clone()));

    // 排序:静态 score + 用户调频增量 + bigram 上文增量(无数据时增量 ln(1)=0)。
    // score 字段保持原始对数概率不被污染,只影响次序。
    let mut ranked: Vec<(f64, Candidate)> = cands
        .into_iter()
        .map(|c| {
            let boost = lex.user_freq.get(&c.text).copied().unwrap_or(0);
            let bigram = prev
                .and_then(|p| lex.user_bigram.get(p))
                .and_then(|m| c.words.first().and_then(|w| m.get(w.as_str())))
                .copied()
                .unwrap_or(0);
            (c.score + (1.0 + boost as f64).ln() * USER_W + (1.0 + bigram as f64).ln() * BIGRAM_W, c)
        })
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked.into_iter().take(limit).map(|(_, c)| c).collect()
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
            if word.chars().count() == 1 && seen.insert(word.clone()) {
                cands.push(Candidate {
                    text: word.clone(),
                    words: vec![word.clone()],
                    score: (*freq as f64 / total).ln(),
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
    fn first_syllable_chars_covers_ambiguous_split() {
        // xian 的首音节有歧义(xi|an 或 xian):两种切分的单字都应在候选。
        let lex = Lexicon::from_lines(
            "xi 西 900\nxi 细 800\nxian 先 700\nxian 线 600\nan 安 500\nxi'an 西安 8000\n",
        );
        let cands = first_syllable_chars(&lex, "xian", 90);
        assert!(cands.iter().all(|c| c.text.chars().count() == 1), "只含单字: {cands:?}");
        // xi 切分的单字(consumed=2)与 xian 切分的单字(consumed=4)都在
        assert!(cands.iter().any(|c| c.text == "西" && c.consumed == 2), "xi 的字: {cands:?}");
        assert!(cands.iter().any(|c| c.text == "先" && c.consumed == 4), "xian 的字: {cands:?}");
        // 词(西安)不是单字,不应出现
        assert!(!cands.iter().any(|c| c.text == "西安"));
    }

    #[test]
    fn first_syllable_chars_apply_user_freq_boost() {
        // 单字模式同样吃用户调频:选过 3 次的低频字应上浮到首位。
        let mut lex = Lexicon::from_lines("xuan 选 900\nxuan 宣 8\nze 泽 500\n");
        lex.user_freq.insert("宣".to_string(), 3);
        let cands = first_syllable_chars(&lex, "xuanze", 90);
        assert_eq!(cands[0].text, "宣", "选过 3 次的低频字应上浮: {cands:?}");
    }

    #[test]
    fn first_syllable_chars_prefers_longest_syllable() {
        // 短切分 xu 的高频字(需)也排在长音节 xuan 的字(选)之后:打什么音节定什么字。
        let lex = Lexicon::from_lines("xu 需 9000\nxuan 选 100\nan 安 500\n");
        let cands = first_syllable_chars(&lex, "xuan", 90);
        assert_eq!(cands[0].text, "选", "长音节字应优先于短切分高频字: {cands:?}");
        assert!(cands.iter().any(|c| c.text == "需"), "短切分字仍在候选后部");
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
