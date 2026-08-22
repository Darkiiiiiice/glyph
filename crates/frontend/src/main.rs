//! glyph — Wayland 中文输入法前端(im-v2 路线,目标 niri)。
//!
//! M1 数据流:
//!   niri → zwp_input_method_v2.activate → grab_keyboard
//!        → grab key 事件 → xkb 解析 → 拼音会话(session)
//!        → set_preedit_string(拼音+候选) / commit_string(上屏)
//!   未消费按键 → virtual-keyboard-v1 转发回 compositor(保住全局快捷键)

mod globals;
mod ime;
mod keyboard;
mod session;

use std::path::PathBuf;
use std::process::ExitCode;

use glyph_engine::Engine;

fn lexicon_path() -> PathBuf {
    if let Some(p) = std::env::var_os("GLYPH_LEXICON") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/lexicon.txt")
}

fn main() -> ExitCode {
    pretty_env_logger::init();
    let lp = lexicon_path();
    let engine = match Engine::load(&lp) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("glyph: 词库加载失败 {}: {e}", lp.display());
            return ExitCode::FAILURE;
        }
    };
    let (_conn, mut eq, mut state) = match globals::connect(engine) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("glyph: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut ok = true;
    for (name, present) in [
        ("zwp_input_method_manager_v2", state.im_manager.is_some()),
        ("zwp_virtual_keyboard_manager_v1", state.vkb_manager.is_some()),
        ("wl_seat", state.seat.is_some()),
    ] {
        if !present {
            eprintln!("glyph: compositor 未公告 {name}");
            ok = false;
        }
    }
    if !state.ti_v3_seen {
        eprintln!("glyph: 警告 — 未发现 zwp_text_input_v3,文本将无法提交");
    }
    if !ok {
        return ExitCode::FAILURE;
    }

    // 注册输入法对象,等待 activate
    let qh = eq.handle();
    let seat = state.seat.clone().unwrap();
    let ime = state.im_manager.as_ref().unwrap().get_input_method(&seat, &qh, ());
    state.ime = Some(ime);
    println!("glyph: 已连接 niri,等待输入焦点…");

    loop {
        if let Err(e) = eq.blocking_dispatch(&mut state) {
            eprintln!("glyph: 事件循环错误: {e}");
            return ExitCode::FAILURE;
        }
        if state.ime.is_none() {
            eprintln!("glyph: 输入法对象被 compositor 撤回,退出");
            return ExitCode::FAILURE;
        }
    }
}
