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
mod popup;
mod repeat;
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

/// 用户词频路径:XDG_DATA_HOME(或 ~/.local/share)/glyph/user_freq.txt
fn user_freq_path() -> PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            home.join(".local/share")
        });
    data_home.join("glyph/user_freq.txt")
}

fn main() -> ExitCode {
    pretty_env_logger::init();
    let lp = lexicon_path();
    let mut engine = match Engine::load(&lp) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("glyph: 词库加载失败 {}: {e}", lp.display());
            return ExitCode::FAILURE;
        }
    };
    let uf = user_freq_path();
    match Engine::load_user_freq(&uf) {
        Ok(map) if !map.is_empty() => {
            log::info!("用户词频 {} 条 ← {}", map.len(), uf.display());
            engine.set_user_freq(map);
        }
        Ok(_) => {} // 空文件/无数据
        Err(e) => log::warn!("用户词频加载失败 {}: {e}(忽略,首次运行正常)", uf.display()),
    }
    // bigram 上文搭配加载(与 user_freq 同目录派生路径 user_bigram.txt)
    let bp = uf.with_file_name("user_bigram.txt");
    match Engine::load_bigram(&bp) {
        Ok(map) if !map.is_empty() => {
            log::info!("用户 bigram {} 组 ← {}", map.len(), bp.display());
            engine.set_user_bigram(map);
        }
        Ok(_) => {}
        Err(e) => log::warn!("用户 bigram 加载失败 {}: {e}(忽略,首次运行正常)", bp.display()),
    }
    let config = glyph_frontend::config::Config::load();
    let (_conn, mut eq, mut state) = match globals::connect(engine, uf, config) {
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
        ("wl_compositor", state.compositor.is_some()),
        ("wl_shm", state.shm.is_some()),
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

    // 事件循环:wayland fd + repeat 定时(poll 超时即下次长按重复到点)。
    // 每轮顺序:派发已读事件 → 到点 repeat 重放 → 刷出站请求 → poll 等下一批。
    use std::os::fd::AsFd;
    use rustix::event::{poll, PollFd, PollFlags};
    loop {
        if let Err(e) = eq.dispatch_pending(&mut state) {
            eprintln!("glyph: 事件循环错误: {e}");
            return ExitCode::FAILURE;
        }
        if state.ime.is_none() {
            eprintln!("glyph: 输入法对象被 compositor 撤回,退出");
            return ExitCode::FAILURE;
        }
        // rustix 1.x 的 poll 超时是 Option<&Timespec>(None = 无限)。
        let timeout = repeat::tick(&mut state, &qh).map(|d| {
            let ms = d.as_millis().clamp(1, i64::MAX as u128 / 1_000_000) as i64;
            rustix::time::Timespec { tv_sec: ms / 1000, tv_nsec: (ms % 1000) * 1_000_000 }
        });
        if let Err(e) = eq.flush() {
            eprintln!("glyph: 连接写失败: {e}");
            return ExitCode::FAILURE;
        }
        let Some(guard) = eq.prepare_read() else { continue }; // 竞态新事件,回循环头派发
        let mut fds = [PollFd::from_borrowed_fd(_conn.as_fd(), PollFlags::IN)];
        match poll(&mut fds, timeout.as_ref()) {
            Ok(n) if n > 0 && fds[0].revents().contains(PollFlags::IN) => {
                if let Err(e) = guard.read() {
                    // 非阻塞 fd 的 EAGAIN 是良性假唤醒(官方模式要求重试),丢弃本轮即可;
                    // read() 无论成败都已消费 guard,直接回循环头重新 prepare。
                    if matches!(&e, wayland_client::backend::WaylandError::Io(io)
                        if io.kind() == std::io::ErrorKind::WouldBlock)
                    {
                        continue;
                    }
                    eprintln!("glyph: 事件读取失败: {e}");
                    return ExitCode::FAILURE;
                }
            }
            Ok(_) => drop(guard),          // 超时:repeat 在循环头处理
            Err(rustix::io::Errno::INTR) => drop(guard),
            Err(e) => {
                eprintln!("glyph: poll 失败: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
}
