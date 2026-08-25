//! rime 词库合并:把 data/raw/rime-{ice,frost}/ 下的 rime dict.yaml 追加进 lexicon.txt。
//!
//! 数据源(GPL-3.0,不入 git,个人本地使用):雾凇拼音 rime-ice + 白霜拼音 rime-frost。
//! 文件缺失的源自动跳过(全新克隆无 rime 数据时构建不受影响)。
//!
//! 词频策略:
//! - 已有 (词,拼音) 键一律保留 jieba 词频(真实语料计数,不接 rime 权重);
//! - 带权重的源按 ln 空间 p50/p90 分位锚定映射到现有词频带(新词落在长尾,
//!   不扰动既有高频序,靠 user_freq/bigram 随使用上浮),并 clamp 到现有最大值;
//! - 无权重/平权重的源给常数词频(见 SOURCES);跨源重复键取最大值。
//!
//! 过滤:词须全 CJK 字符(ASCII/双拼码表不可达);拼音逐音节须在既有音节表内
//! (方言/边缘读音如 lo、yo 不在 8105 音节表,丢弃并计数)。

use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

/// 源处理:Weighted = ln 分位锚定映射;Flat(c) = 常数词频 c。
enum Src {
    Weighted,
    Flat(u32),
}

/// 合并清单。ice/tencent(98 万条无权重网络抓取)不含:frost/tencent 已是其
/// 34 万条精选子集;frost word/chengyu 是双拼码表、corrections 是容错注音,均不可用。
const SOURCES: &[(&str, Src)] = &[
    ("rime-ice/base.dict.yaml", Src::Weighted),
    ("rime-ice/ext.dict.yaml", Src::Flat(30)),
    ("rime-ice/8105.dict.yaml", Src::Weighted),
    ("rime-ice/41448.dict.yaml", Src::Flat(3)),
    ("rime-ice/others.dict.yaml", Src::Flat(30)),
    ("rime-frost/base.dict.yaml", Src::Weighted),
    ("rime-frost/ext.dict.yaml", Src::Weighted),
    ("rime-frost/tencent.dict.yaml", Src::Flat(5)),
    ("rime-frost/GB18030-2022.dict.yaml", Src::Flat(3)),
    ("rime-frost/others.dict.yaml", Src::Flat(30)),
];

fn is_cjk(c: char) -> bool {
    ('\u{3400}'..='\u{9fff}').contains(&c) || ('\u{f900}'..='\u{faff}').contains(&c)
}

/// 排序后取 ln 分位:(ln p50, ln p90, ln max)。
fn ln_quantiles(v: &mut Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.total_cmp(b));
    let at = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
    (at(0.5), at(0.9), v[v.len() - 1])
}

/// 把 rime 权重 w 映射到现有词频带:ln 空间 p50→p50、p90→p90 线性,clamp 到 [1, max]。
fn map_weight(w: u32, src: (f64, f64), dst: (f64, f64, f64)) -> u32 {
    let (s50, s90) = src;
    let (d50, d90, dmax) = dst;
    let slope = if s90 > s50 { (d90 - d50) / (s90 - s50) } else { 0.0 };
    let ln_f = d50 + ((w as f64 + 1.0).ln() - s50) * slope;
    ln_f.clamp(0.0, dmax).exp().round().max(1.0) as u32
}

/// 合并 rime 源到 lexicon,返回各源 (文件, 新增, 跳过) 统计。无 rime 数据时 Ok(None)。
pub fn merge(raw: &Path, lexicon_path: &Path) -> std::io::Result<Option<Vec<(String, usize, usize)>>> {
    if !SOURCES.iter().any(|(f, _)| raw.join(f).exists()) {
        return Ok(None);
    }
    // 读现有词库:已有键、词频分布(锚)、音节表(校验)
    let text = std::fs::read_to_string(lexicon_path)?;
    let mut keys: HashSet<(String, String)> = HashSet::new();
    let mut freqs: Vec<f64> = Vec::new();
    let mut syllables: HashSet<&str> = HashSet::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(py), Some(word), Some(freq)) = (it.next(), it.next(), it.next()) else { continue };
        keys.insert((word.to_string(), py.to_string()));
        if let Ok(f) = freq.parse::<u32>() {
            freqs.push((f as f64).ln());
        }
        syllables.extend(py.split('\''));
    }
    let dst = ln_quantiles(&mut freqs);

    let mut added: HashMap<(String, String), u32> = HashMap::new();
    let mut stats = Vec::new();
    for (file, kind) in SOURCES {
        let path = raw.join(file);
        if !path.exists() {
            continue;
        }
        // 一遍解析+校验:合法条目 (词, 拼音, 权重)
        let mut valid: Vec<(String, String, u32)> = Vec::new();
        let mut skipped = 0usize;
        let mut in_head = true;
        for line in BufReader::new(File::open(&path)?).lines() {
            let line = line?;
            if in_head {
                if line.trim() == "..." {
                    in_head = false;
                }
                continue;
            }
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut it = line.split('\t');
            let (Some(word), Some(py)) = (it.next(), it.next()) else { continue };
            let weight = it.next().and_then(|s| s.parse::<u32>().ok());
            let sylls: Vec<&str> = py.split(' ').collect();
            if !word.chars().all(is_cjk)
                || sylls.len() != word.chars().count()
                || !sylls.iter().all(|s| syllables.contains(s))
            {
                skipped += 1;
                continue;
            }
            valid.push((word.to_string(), sylls.join("'"), weight.unwrap_or(0)));
        }
        // 映射并入(键已存在=保留现有词频,跳过)
        let mut src_q = (0.0, 0.0);
        if matches!(kind, Src::Weighted) {
            let mut ws: Vec<f64> = valid.iter().map(|(_, _, w)| (*w as f64 + 1.0).ln()).collect();
            let q = ln_quantiles(&mut ws);
            src_q = (q.0, q.1);
        }
        let mut kept = 0usize;
        for (word, py, w) in valid {
            let freq = match kind {
                Src::Weighted => map_weight(w, src_q, dst),
                Src::Flat(c) => *c,
            };
            if keys.contains(&(word.clone(), py.clone())) {
                skipped += 1;
                continue;
            }
            added.entry((word, py)).and_modify(|f| *f = (*f).max(freq)).or_insert(freq);
            kept += 1;
        }
        stats.push((file.to_string(), kept, skipped));
    }
    // 追加写(行序无关:引擎加载时按词频排序节点)
    let mut out = BufWriter::new(OpenOptions::new().append(true).open(lexicon_path)?);
    for ((word, py), freq) in &added {
        writeln!(out, "{py} {word} {freq}")?;
    }
    stats.push(("合计新增".to_string(), added.len(), 0));
    Ok(Some(stats))
}
