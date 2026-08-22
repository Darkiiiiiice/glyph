//! 音节格：把输入的拼音字母串按合法音节展开成所有可能切法。
//!
//! 音节合法性完全由词库中出现的音节集合决定（数据驱动，不硬编码 410 音节表——
//! 词库本身就是音节表，PLAN.md 中"音节表"一项由此满足）。
//! 输入中的 `'` 是强制边界：音节可以恰好结束在边界处，但不得跨越边界，
//! 以此区分 `xian`（先/西安两可）与 `xi'an`（只能是西安类）。

use std::collections::{BTreeSet, HashSet};

/// 音节格。所有区间都是 `text` 的字节区间（输入为 ASCII，字节即字符）。
pub struct Lattice {
    /// 去掉 `'` 后的输入。
    pub text: String,
    /// 全部合法音节区间，按（起点, 终点）排序。
    pub syllables: Vec<(usize, usize)>,
    /// 字节起点 → 从该点出发的音节在 `syllables` 中的下标。
    pub starts: Vec<Vec<usize>>,
}

/// 单音节最长字节数（chuang / shuang）。
const MAX_SYLLABLE_LEN: usize = 6;

pub fn build_lattice(input: &str, valid: &HashSet<String>) -> Lattice {
    // 清洗：去掉 '，并记录它在清洗后串中对应的强制边界位置。
    let mut text = String::with_capacity(input.len());
    let mut forced = BTreeSet::new();
    for ch in input.chars() {
        if ch == '\'' {
            forced.insert(text.len());
        } else {
            text.push(ch);
        }
    }

    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut syllables = Vec::new();
    let mut starts = vec![Vec::new(); n + 1];
    for start in 0..n {
        if !bytes[start].is_ascii_lowercase() {
            continue; // 非小写字母（数字、大写残留）不可能成音节
        }
        let max_end = (start + MAX_SYLLABLE_LEN).min(n);
        for end in start + 1..=max_end {
            // 音节不得跨越强制边界（边界只能恰好落在音节末尾）。
            if forced.range(start + 1..end).next().is_some() {
                continue;
            }
            let span = &text[start..end];
            if span.bytes().all(|b| b.is_ascii_lowercase()) && valid.contains(span) {
                starts[start].push(syllables.len());
                syllables.push((start, end));
            }
        }
    }
    Lattice { text, syllables, starts }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid(s: &[&str]) -> HashSet<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn spans(lat: &Lattice) -> Vec<&str> {
        lat.syllables.iter().map(|&(a, b)| &lat.text[a..b]).collect()
    }

    #[test]
    fn splits_nihao() {
        let lat = build_lattice("nihao", &valid(&["ni", "hao", "ha", "o"]));
        let s = spans(&lat);
        assert!(s.contains(&"ni") && s.contains(&"hao") && s.contains(&"ha"));
        assert!(!s.contains(&"nih"));
    }

    #[test]
    fn apostrophe_forces_boundary() {
        let lat = build_lattice("xi'an", &valid(&["xi", "an", "xian"]));
        let s = spans(&lat);
        assert!(s.contains(&"xi") && s.contains(&"an"));
        assert!(!s.contains(&"xian"), "xian 跨越了 ' 边界");
    }

    #[test]
    fn bare_xian_is_ambiguous() {
        let lat = build_lattice("xian", &valid(&["xi", "an", "xian"]));
        let s = spans(&lat);
        assert!(s.contains(&"xian") && s.contains(&"xi") && s.contains(&"an"));
    }
}
