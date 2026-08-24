//! segment 单元测试(行数合规拆分:本文件使 segment.rs 保持 <300 行)。

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

#[test]
fn bigram_narrow_gap_flips_once_and_recovers() {
    // 窄 gap(世纪 21100 vs 实际 12010,ln gap≈0.56):1 次搭配即翻盘,对称各 1 次恢复静态序。
    // "一次即学"是刻意行为(与 USER_W 同量纲);翻转只在相邻名次间,两词始终前二可见。
    let mut lex = Lexicon::from_lines("wo'men 我们 9000\nshi'ji 世纪 21100\nshi'ji 实际 12010\n");
    let prev = Some("我们");
    assert_eq!(convert_ctx(&lex, "shiji", 9, prev)[0].text, "世纪");
    lex.user_bigram.entry("我们".to_string()).or_default().insert("实际".to_string(), 1);
    assert_eq!(convert_ctx(&lex, "shiji", 9, prev)[0].text, "实际", "1 次搭配应翻窄 gap");
    lex.user_bigram.get_mut("我们").unwrap().insert("世纪".to_string(), 1);
    assert_eq!(convert_ctx(&lex, "shiji", 9, prev)[0].text, "世纪", "对称各 1 次应恢复静态序");
}

#[test]
fn bigram_wide_gap_requires_dominant_usage() {
    // 宽 gap(探索 2653 vs 坍缩 3,ln gap≈6.78):交替使用(7:6)永不翻盘,独占 3 次才翻。
    // 交替 = 上下文无区分度,bigram 正确弃权给静态先验;宽 gap 从不抖动。
    let mut lex = Lexicon::from_lines("wo'men 我们 9000\ntan'suo 探索 2653\ntan'suo 坍缩 3\n");
    let prev = Some("我们");
    let m = lex.user_bigram.entry("我们".to_string()).or_default();
    m.insert("坍缩".to_string(), 7);
    m.insert("探索".to_string(), 6);
    assert_eq!(convert_ctx(&lex, "tansuo", 9, prev)[0].text, "探索", "交替使用时 bigram 应弃权");
    let m = lex.user_bigram.get_mut("我们").unwrap();
    m.insert("坍缩".to_string(), 3);
    m.insert("探索".to_string(), 0);
    assert_eq!(convert_ctx(&lex, "tansuo", 9, prev)[0].text, "坍缩", "独占 3 次应翻宽 gap");
    // 翻盘后恢复极便宜:探索×1,Δboost=6·ln(4/2)=4.16 < gap 6.78,静态锚即刻夺回首位。
    lex.user_bigram.get_mut("我们").unwrap().insert("探索".to_string(), 1);
    assert_eq!(convert_ctx(&lex, "tansuo", 9, prev)[0].text, "探索", "1 次反向选择应即刻恢复");
    // 但锚不锁死:坍缩追到 6:1(Δboost=6·ln3.5≈7.52>6.78)再次翻盘。
    lex.user_bigram.get_mut("我们").unwrap().insert("坍缩".to_string(), 5);
    assert_eq!(convert_ctx(&lex, "tansuo", 9, prev)[0].text, "探索", "5:1 仍不够");
    lex.user_bigram.get_mut("我们").unwrap().insert("坍缩".to_string(), 6);
    assert_eq!(convert_ctx(&lex, "tansuo", 9, prev)[0].text, "坍缩", "6:1 应再翻盘");
}
