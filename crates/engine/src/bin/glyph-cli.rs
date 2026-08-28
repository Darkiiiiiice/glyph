//! glyph-cli：M0 验收入口。stdin 每行一串拼音，stdout 输出编号候选。
//!
//! 用法：echo "nihao" | glyph-cli [--lexicon data/lexicon.txt]

use std::env;
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;

use glyph_engine::Engine;

fn main() -> ExitCode {
    let mut lexicon = env::var("GLYPH_LEXICON").unwrap_or_else(|_| "data/lexicon.txt".to_string());
    let mut char_mode = false;
    let mut ctx: Option<String> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lexicon" => match args.next() {
                Some(p) => lexicon = p,
                None => return usage(),
            },
            "--chars" => char_mode = true, // Tab 单字模式:第一音节单字候选
            "--ctx" => match args.next() {
                Some(w) => ctx = Some(w), // 上文:空格分隔多词(自然语序,旧→新);bigram 用末词,trigram 用末两词
                None => return usage(),
            },
            _ => return usage(),
        }
    }

    let mut engine = match Engine::load(Path::new(&lexicon)) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("加载词库失败 {lexicon}: {err}");
            eprintln!("提示: 先运行 cargo run --bin glyph-build 生成词库, 或用 --lexicon 指定路径");
            return ExitCode::FAILURE;
        }
    };
    // 加载用户 bigram(XDG 路径,与 glyph 同),供 --ctx 上文排序
    if let Some(bp) = bigram_path() {
        if let Ok(map) = Engine::load_bigram(&bp) {
            if !map.is_empty() {
                engine.set_user_bigram(map);
            }
        }
    }
    // 加载用户 trigram(与 bigram 同目录派生 user_trigram.txt),供 --ctx 双词上文排序
    if let Some(tp) = bigram_path().map(|p| p.with_file_name("user_trigram.txt")) {
        if let Ok(map) = Engine::load_trigram(&tp) {
            if !map.is_empty() {
                engine.set_user_trigram(map);
            }
        }
    }
    // 用户造词 overlay(与 bigram 同目录派生 user_dict.txt)
    if let Some(dp) = bigram_path().map(|p| p.with_file_name("user_dict.txt")) {
        let _ = engine.load_user_dict(&dp);
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let input = line.trim().to_lowercase();
        if input.is_empty() {
            continue;
        }
        // ctx 转成最近优先切片(用户按自然语序给词,反转后 ctx[0]=末词=最近一次)
        let ctx_words: Vec<&str> = ctx.as_deref().map(|c| c.split_whitespace().rev().collect()).unwrap_or_default();
        let cands = if char_mode { engine.first_syllable_chars(&input, 9) } else { engine.convert_ctx(&input, 9, &ctx_words) };
        if cands.is_empty() {
            writeln!(out, "{input} -> (无候选)").ok();
            continue;
        }
        let list = cands
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{}.{}", i + 1, c.text))
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(out, "{list}").ok();
    }
    out.flush().ok();
    ExitCode::SUCCESS
}

fn usage() -> ExitCode {
    eprintln!("用法: glyph-cli [--lexicon <路径>] [--chars] [--ctx <上文...>] < 拼音串");
    ExitCode::FAILURE
}

/// 用户 bigram 路径:XDG_DATA_HOME(或 ~/.local/share)/glyph/user_bigram.txt(与 glyph 同)。
fn bigram_path() -> Option<std::path::PathBuf> {
    let data_home = env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))?;
    Some(data_home.join("glyph/user_bigram.txt"))
}
