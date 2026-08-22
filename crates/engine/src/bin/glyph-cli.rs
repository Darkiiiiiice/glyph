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
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lexicon" => match args.next() {
                Some(p) => lexicon = p,
                None => return usage(),
            },
            _ => return usage(),
        }
    }

    let engine = match Engine::load(Path::new(&lexicon)) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("加载词库失败 {lexicon}: {err}");
            eprintln!("提示: 先运行 cargo run --bin glyph-build 生成词库, 或用 --lexicon 指定路径");
            return ExitCode::FAILURE;
        }
    };

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
        let cands = engine.convert(&input, 9);
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
    eprintln!("用法: glyph-cli [--lexicon <路径>] < 拼音串");
    ExitCode::FAILURE
}
