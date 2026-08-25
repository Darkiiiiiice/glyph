//! glyph-build：把开源原始数据编译成引擎词库 data/lexicon.txt。
//!
//! 数据源（全部 MIT，见 PLAN.md；需先放入 data/raw/）：
//! - jieba.dict.txt        jieba 词典，行格式 `词 词频 词性`，提供词与词频；
//! - kMandarin_8105.txt    pinyin-data，行格式 `U+4E00: yī  # 一`，提供单字最常用读音；
//! - phrase-pinyin.txt     phrase-pinyin-data，行格式 `重庆: chóng qìng`，提供词语级读音。
//!
//! 可选数据源(GPL-3.0,不入 git,个人本地使用):data/raw/rime-{ice,frost}/*.dict.yaml
//! 存在时自动合并进词库(见 rime_merge.rs 头注),缺失则静默跳过。
//!
//! 读音策略：词语优先查词语级数据（多音字正确，如 重庆→chong'qing），
//! 查不到再逐字查 kMandarin（单音字场景）。含未收录字符的词（夹 ASCII 等）跳过。
//! 输出行格式：`pin'yin 词 词频`（音节间用 ' 连接，声调记号已剥除，ü 记作 v）。

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod rime_merge;

fn main() -> ExitCode {
    let mut raw = PathBuf::from("data/raw");
    let mut out = PathBuf::from("data/lexicon.txt");
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--raw" => match args.next() {
                Some(p) => raw = PathBuf::from(p),
                None => return usage(),
            },
            "--out" => match args.next() {
                Some(p) => out = PathBuf::from(p),
                None => return usage(),
            },
            _ => return usage(),
        }
    }

    match run(&raw, &out) {
        Ok((written, skipped)) => {
            println!("完成: {written} 词条 -> {}, {skipped} 条跳过", out.display());
            match rime_merge::merge(&raw, &out) {
                Ok(Some(stats)) => {
                    for (file, kept, skip) in stats {
                        println!("rime 合并 {file}: 新增 {kept}, 跳过 {skip}");
                    }
                }
                Ok(None) => println!("rime 词库不存在,跳过合并"),
                Err(e) => {
                    eprintln!("rime 合并失败: {e}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("生成失败: {err}");
            eprintln!("提示: 需先下载三份原始数据到 {}/，见 PLAN.md 数据源一节", raw.display());
            ExitCode::FAILURE
        }
    }
}

fn run(raw: &Path, out: &Path) -> io::Result<(usize, usize)> {
    let chars = load_char_pinyin(&raw.join("kMandarin_8105.txt"))?;
    let phrases = load_phrase_pinyin(&raw.join("phrase-pinyin.txt"))?;
    build(&raw.join("jieba.dict.txt"), &chars, &phrases, out)
}

/// 剥除声调记号：āáǎà→a 等；ü 系→v（全拼惯例以 v 代 ü）。
/// 无法识别的记号（ê、ḿ 等边缘读音）返回 None，该条跳过。
fn strip_tones(s: &str) -> Option<String> {
    s.chars()
        .map(|c| match c {
            'ā' | 'á' | 'ǎ' | 'à' => Some('a'),
            'ē' | 'é' | 'ě' | 'è' => Some('e'),
            'ī' | 'í' | 'ǐ' | 'ì' => Some('i'),
            'ō' | 'ó' | 'ǒ' | 'ò' => Some('o'),
            'ū' | 'ú' | 'ǔ' | 'ù' => Some('u'),
            'ǖ' | 'ǘ' | 'ǚ' | 'ǜ' | 'ü' => Some('v'),
            c if c.is_ascii_lowercase() => Some(c),
            _ => None,
        })
        .collect()
}

/// `U+4E00: yī  # 一` → 一 → yi
fn load_char_pinyin(path: &Path) -> io::Result<HashMap<char, String>> {
    let mut map = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if !line.starts_with("U+") {
            continue; // 注释行
        }
        let (left, hanzi) = match line.split_once('#') {
            Some(x) => x,
            None => continue,
        };
        let pinyin = match left.split_once(':') {
            Some((_, p)) => p.trim(),
            None => continue,
        };
        let hanzi = match hanzi.trim().chars().next() {
            Some(c) => c,
            None => continue,
        };
        if let Some(plain) = strip_tones(pinyin) {
            map.insert(hanzi, plain);
        }
    }
    Ok(map)
}

/// `重庆: chóng qìng` → 重庆 → chong'qing
fn load_phrase_pinyin(path: &Path) -> io::Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.starts_with('#') {
            continue;
        }
        let (word, syllables) = match line.split_once(':') {
            Some(x) => x,
            None => continue,
        };
        let plain: Option<Vec<String>> =
            syllables.split_whitespace().map(strip_tones).collect();
        if let Some(plain) = plain {
            map.insert(word.trim().to_string(), plain.join("'"));
        }
    }
    Ok(map)
}

/// 词语读音：优先词语级数据，否则逐字查。任一字符无读音 → None（跳过该词）。
fn pinyin_of(word: &str, chars: &HashMap<char, String>, phrases: &HashMap<String, String>) -> Option<String> {
    if let Some(p) = phrases.get(word) {
        return Some(p.clone());
    }
    word.chars()
        .map(|c| chars.get(&c).cloned())
        .collect::<Option<Vec<_>>>()
        .map(|v| v.join("'"))
}

fn build(
    dict_path: &Path,
    chars: &HashMap<char, String>,
    phrases: &HashMap<String, String>,
    out_path: &Path,
) -> io::Result<(usize, usize)> {
    let mut seen = HashSet::new();
    let (mut written, mut skipped) = (0usize, 0usize);
    let mut out = BufWriter::new(File::create(out_path)?);
    for line in BufReader::new(File::open(dict_path)?).lines() {
        let line = line?;
        let mut fields = line.split_whitespace();
        let (word, freq) = match (fields.next(), fields.next().and_then(|f| f.parse::<u32>().ok())) {
            (Some(w), Some(f)) => (w, f),
            _ => {
                skipped += 1;
                continue;
            }
        };
        let pinyin = match pinyin_of(word, chars, phrases) {
            Some(p) => p,
            None => {
                skipped += 1;
                continue;
            }
        };
        if seen.insert((pinyin.clone(), word.to_string())) {
            writeln!(out, "{pinyin} {word} {freq}")?;
            written += 1;
        }
    }
    Ok((written, skipped))
}

fn usage() -> ExitCode {
    eprintln!("用法: glyph-build [--raw data/raw] [--out data/lexicon.txt]");
    ExitCode::FAILURE
}
