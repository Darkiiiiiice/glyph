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
fn mixed_jianpin_full_syllable_plus_shengmu() {
    let lex = Lexicon::from_lines(
        "li'jie 理解 5000\nli'ji 立即 4000\nluo'ji 逻辑 3000\nlian'jie 连接 8000\n",
    );
    // lij = [li][j] 混合拼:首槽精确 li、次槽声母 j → 理解/立即 命中
    let cands = convert(&lex, "lij", 9);
    let texts: Vec<&str> = cands.iter().map(|c| c.text.as_str()).collect();
    assert_eq!(texts.first(), Some(&"理解"));
    assert!(texts.contains(&"立即"));
    // 声母同为 lj 但首槽非 li 的被精确槽过滤
    assert!(!texts.contains(&"逻辑"), "luo'ji 首槽≠li 应过滤: {texts:?}");
    assert!(!texts.contains(&"连接"), "lian'jie 首槽≠li 应过滤: {texts:?}");
}

#[test]
fn mixed_jianpin_zh_shengmu() {
    let lex = Lexicon::from_lines("zhong'guo 中国 8000\nzhan'guo 战国 5000\n");
    // zhongg = [zhong][g]:zh 双字母声母在精确槽里整体匹配
    assert_eq!(convert(&lex, "zhongg", 9)[0].text, "中国");
    // zhg 纯简拼行为不变(zh 声母槽)
    assert_eq!(convert(&lex, "zhg", 9)[0].text, "中国");
    // "zhang" 非音节,但音节格切出 [zhan][g] 混合拼 → 战国 命中
    // (中国 zhong'guo 首槽≠zhan 被滤);与主流输入法行为一致
    assert_eq!(convert(&lex, "zhang", 9)[0].text, "战国");
}

#[test]
fn bigram_narrow_gap_flips_once_and_recovers() {
    // 窄 gap(世纪 21100 vs 实际 12010,ln gap≈0.56):1 次搭配即翻盘,对称各 1 次恢复静态序。
    // "一次即学"是刻意行为(与 USER_W 同量纲);翻转只在相邻名次间,两词始终前二可见。
    let mut lex = Lexicon::from_lines("wo'men 我们 9000\nshi'ji 世纪 21100\nshi'ji 实际 12010\n");
    let prev = &["我们"][..];
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
    let prev = &["我们"][..];
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

#[test]
fn trigram_flips_p99_gap_with_one_collocation() {
    // 双词上下文是强证据:1 次搭配翻 p99 级 gap(ln665≈6.50;10·ln2≈6.93 压过)。
    let mut lex = Lexicon::from_lines("wo'men 我们 9000\nai 爱 8000\nzhong'wen 中文 6650\nzhong'wen 种蚊 10\n");
    assert_eq!(convert_ctx(&lex, "zhongwen", 9, &["爱", "我们"])[0].text, "中文");
    lex.user_trigram.entry("我们".to_string()).or_default().entry("爱".to_string()).or_default().insert("种蚊".to_string(), 1);
    assert_eq!(convert_ctx(&lex, "zhongwen", 9, &["爱", "我们"])[0].text, "种蚊", "1 次双词搭配应翻 6.5 gap");
}

#[test]
fn trigram_requires_both_context_words() {
    // 只有 (上上文, 上一词) 全中才触发:缺一词、次序颠倒、上上文不符都回落静态序。
    let mut lex = Lexicon::from_lines("wo'men 我们 9000\nai 爱 8000\nzhong'wen 中文 6650\nzhong'wen 种蚊 10\n");
    lex.user_trigram.entry("我们".to_string()).or_default().entry("爱".to_string()).or_default().insert("种蚊".to_string(), 1);
    assert_eq!(convert_ctx(&lex, "zhongwen", 9, &["爱"])[0].text, "中文", "只有单词上文不触发");
    assert_eq!(convert_ctx(&lex, "zhongwen", 9, &["爱", "他们"])[0].text, "中文", "上上文不符不触发");
    assert_eq!(convert_ctx(&lex, "zhongwen", 9, &["我们", "爱"])[0].text, "中文", "次序颠倒不触发");
}

#[test]
fn trigram_and_bigram_take_max_not_sum() {
    // 一次上屏同时写 bigram+trigram 两条记录:相加(4.16+6.93=11.1)会翻 gap 9,
    // max(=6.93) 不翻——静态锚保持即证明无双记;高 count bigram 强于 1 次 trigram 时 max 不丢强证据。
    let mut lex = Lexicon::from_lines("wo'men 我们 9000\nai 爱 8000\nmu'di 目的 810000\nmu'di 墓地 100\n");
    lex.user_bigram.entry("爱".to_string()).or_default().insert("墓地".to_string(), 1);
    lex.user_trigram.entry("我们".to_string()).or_default().entry("爱".to_string()).or_default().insert("墓地".to_string(), 1);
    assert_eq!(convert_ctx(&lex, "mudi", 9, &["爱", "我们"])[0].text, "目的", "max(4.16,6.93)=6.93 < gap 9,应不翻");
    lex.user_bigram.get_mut("爱").unwrap().insert("墓地".to_string(), 20);
    assert_eq!(convert_ctx(&lex, "mudi", 9, &["爱", "我们"])[0].text, "墓地", "bigram 20 次(18.3)胜过 trigram 1 次(6.93)");
}
