//! dict 测试:音节表/总词频派生、简拼索引(路径化条目经 trie 解析出词)。
use super::*;

#[test]
fn derives_syllables_and_total() {
    let lex = Lexicon::from_lines("ni'hao 你好 10000\nni 你 500\nhao 好 300\n");
    assert!(lex.syllables.contains("ni") && lex.syllables.contains("hao"));
    assert_eq!(lex.total_freq, 10800);
}

#[test]
fn words_stop_at_exact_path() {
    // 池化展平时 word_len 必须在递归子树前定格,否则子树词条漏进前缀节点。
    let lex = Lexicon::from_lines("ni'hao 你好 10000\nni'hao'xiang 你好像 8000\nni 你 500\n");
    let id = |s: &str| lex.syllable_ids[s];
    let ws = lex.words_at_path(&[id("ni"), id("hao")]).unwrap();
    let texts: Vec<&str> = ws.iter().map(|(w, _)| w).collect();
    assert_eq!(texts, ["你好"], "前缀节点词边必须止于本节点: {texts:?}");
}

#[test]
fn builds_jianpin_index() {
    let lex = Lexicon::from_lines("ni'hao 你好 10000\nzhong'guo 中国 8000\nni 你 500\n");
    // 多字词建简拼;zh 是双字母声母整体;条目是音节路径,经 trie 解析出词
    let has = |key: &str, word: &str| {
        lex.jianpin_bucket(key).is_some_and(|mut paths| {
            paths.any(|p| lex.words_at_path(p).is_some_and(|ws| ws.iter().any(|(w, _)| w == word)))
        })
    };
    assert!(has("nh", "你好"));
    assert!(has("zhg", "中国"));
    // 单字不进简拼索引(单音节词不建 key)
    assert!(lex.jianpin_bucket("n").is_none());
}

/// 真实词库的规模/布局测量,手工跑:
/// `cargo test --release -p glyph-engine measure_real -- --ignored --nocapture`
#[test]
#[ignore]
fn measure_real_lexicon() {
    let lex = Lexicon::load(Path::new("../../data/lexicon.txt")).unwrap();
    let jp_paths: usize = lex.jianpin.values().map(|&(_, l)| l as usize).sum();
    println!(
        "nodes={} edges={} words={} texts={}MB jp_keys={} jp_paths={jp_paths}",
        lex.nodes.len(),
        lex.edges.len(),
        lex.words.len(),
        lex.texts.len() / 1_000_000,
        lex.jianpin.len(),
    );
    println!(
        "pools={}MB (nodes {} + edges {} + words {} + texts {})",
        (lex.nodes.len() * 16 + lex.edges.len() * 8 + lex.words.len() * 12 + lex.texts.len())
            / 1_000_000,
        lex.nodes.len() * 16 / 1_000_000,
        lex.edges.len() * 8 / 1_000_000,
        lex.words.len() * 12 / 1_000_000,
        lex.texts.len() / 1_000_000,
    );
}
