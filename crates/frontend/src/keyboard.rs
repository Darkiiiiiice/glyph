//! 键盘 grab 事件:xkb 键码解析 → 拼音会话 → 消费或经 vkb 转发。
//!
//! 转发通道的必要性:grab 生效期间 compositor 不再处理任何键盘事件,
//! 不转发会把 niri 全局快捷键(Super+Q 等)全部吞掉。

use wayland_client::protocol::wl_keyboard;
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use xkbcommon::xkb;

use crate::globals::State;
use crate::ime;
use glyph_frontend::protocol::input_method_v2::client::zwp_input_method_keyboard_grab_v2::{
    Event, ZwpInputMethodKeyboardGrabV2,
};

/// wl_keyboard 惯例:协议里的 key 是 evdev 键码,xkb keycode = evdev + 8。
const EVDEV_TO_XKB: u32 = 8;

impl Dispatch<ZwpInputMethodKeyboardGrabV2, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpInputMethodKeyboardGrabV2,
        event: Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            Event::Keymap { format, fd, size } => on_keymap(state, format, fd, size),
            Event::Key { time, key, state: st, .. } => {
                let sym = on_key(state, time, key, st, qh);
                if st == WEnum::Value(wl_keyboard::KeyState::Pressed) {
                    // 长按重复记账:可重复键成为"按住键";不可重复键(修饰键等)按下则停重复。
                    state.held = crate::repeat::repeat_delay(state, sym)
                        .map(|d| crate::repeat::HeldKey { key, next: std::time::Instant::now() + d });
                }
            }
            Event::Modifiers { mods_depressed, mods_latched, mods_locked, group, .. } => {
                log::debug!("grab modifiers: d={mods_depressed} l={mods_latched} g={group} vkb_ready={}", state.vkb_ready);
                if let Some(xs) = &mut state.xkb_state {
                    xs.update_mask(mods_depressed, mods_latched, mods_locked, 0, 0, group);
                }
                if state.vkb_ready {
                    if let Some(vkb) = &state.vkb {
                        vkb.modifiers(mods_depressed, mods_latched, mods_locked, group);
                    }
                } else {
                    // keymap 尚未同步:缓存,keymap 发送后补发(事件顺序不由我控制)
                    state.pending_modifiers = Some((mods_depressed, mods_latched, mods_locked, group));
                }
            }
            Event::RepeatInfo { rate, delay } => {
                log::debug!("repeat_info: rate={rate}/s delay={delay}ms");
                state.repeat_info = Some((rate as u32, delay as u32));
            }
            _ => {} // non_exhaustive:lib 侧生成的枚举跨 crate 需兜底
        }
        let _ = qh;
    }
}

/// 建立 vkb 转发通道(幂等)。
pub fn ensure_vkb(state: &mut State, qh: &QueueHandle<State>) {
    if state.vkb.is_some() {
        return;
    }
    let (Some(mgr), Some(seat)) = (&state.vkb_manager, &state.seat) else { return };
    let vkb = mgr.create_virtual_keyboard(seat, qh, ());
    state.vkb = Some(vkb);
    // keymap 会在随后的 grab keymap 事件中原样转发给 vkb
}

fn on_keymap(state: &mut State, format: WEnum<wl_keyboard::KeymapFormat>, fd: std::os::fd::OwnedFd, size: u32) {
    log::debug!("grab keymap: format={format:?} size={size}");
    if format != WEnum::Value(wl_keyboard::KeymapFormat::XkbV1) {
        log::warn!("非 xkb keymap 格式,忽略");
        return;
    }
    // vkb 转发:dup 原始 fd 原样发送——内容与 size 和 compositor 给的完全一致,
    // 不经文本复制,杜绝任何损坏面。vkb 必须先收 keymap 才能收 key/modifiers。
    if let Some(vkb) = &state.vkb {
        match fd.try_clone() {
            Ok(fd2) => {
                use std::os::fd::AsFd;
                vkb.keymap(wl_keyboard::KeymapFormat::XkbV1.into(), fd2.as_fd(), size);
                state.vkb_ready = true;
                log::debug!("vkb keymap forwarded ({size} bytes)");
                if let Some((d, l, lk, g)) = state.pending_modifiers.take() {
                    vkb.modifiers(d, l, lk, g);
                }
            }
            Err(e) => log::error!("keymap fd dup 失败: {e}"),
        }
    }
    // xkb:new_from_fd 消费原 fd(mmap 从 offset 0,不受 fd 当前偏移影响)
    let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    // SAFETY: fd 来自 compositor 的 keymap 事件,有效且可读;size 即事件给的字节数。
    let km = unsafe {
        xkb::Keymap::new_from_fd(&ctx, fd, size as usize, xkb::KEYMAP_FORMAT_TEXT_V1, xkb::KEYMAP_COMPILE_NO_FLAGS)
    };
    match km {
        Ok(Some(km)) => state.xkb_state = Some(xkb::State::new(&km)),
        Ok(None) => log::error!("xkb keymap 解析失败(空)"),
        Err(e) => log::error!("xkb keymap mmap 失败: {e}"),
    }
}

/// 长按重复合成的一次按下:与真实按下走完全相同的路径(on_key 的 pressed 分支)。
/// held 记账只在 Dispatch 层对真实按键做,合成键不触碰,计时由 repeat::tick 推进。
pub(crate) fn press(state: &mut State, time: u32, key: u32, qh: &wayland_client::QueueHandle<State>) {
    on_key(state, time, key, WEnum::Value(wl_keyboard::KeyState::Pressed), qh);
}

fn on_key(state: &mut State, time: u32, key: u32, st: WEnum<wl_keyboard::KeyState>, qh: &wayland_client::QueueHandle<State>) -> u32 {
    let pressed = st == WEnum::Value(wl_keyboard::KeyState::Pressed);
    let st_raw = match st {
        WEnum::Value(v) => v as u32,
        WEnum::Unknown(u) => u,
    };
    // 返回值:pressed 时为 keysym(Dispatch 层做长按重复记账),release 时为 0。
    let mut out_sym = 0u32;
    if pressed {
        let sym = state
            .xkb_state
            .as_ref()
            .map(|xs| u32::from(xs.key_get_one_sym(xkb::Keycode::new(key + EVDEV_TO_XKB))))
            .unwrap_or(0);
        out_sym = sym;
        // Ctrl/Alt/Super 按住时,字母键是快捷键而非拼音输入(xkb 对带修饰的
        // 字母仍返回小写 keysym),必须转发给 compositor/应用。
        // Shift 不用特判:shift+a 的 keysym 是大写 A,天然不进字母分支。
        let xs = state.xkb_state.as_ref();
        let ctrl = xs.is_some_and(|x| {
            x.mod_name_is_active(xkb::MOD_NAME_CTRL, xkb::STATE_MODS_EFFECTIVE)
        });
        let shortcut = xs.is_some_and(has_shortcut_modifier);
        let reply = if ctrl && sym == xkb::keysyms::KEY_period {
            // Ctrl+. 切换中/英文标点:IME 消费该组合键,不转发给应用。
            let cn = state.session.toggle_punct();
            log::info!("标点模式: {}", if cn { "中文" } else { "英文" });
            crate::session::Reply { consumed: true, ..Default::default() }
        } else if shortcut {
            crate::session::Reply::default() // 其余快捷键不消费,转发
        } else {
            state.session.on_keysym(&mut state.engine, sym)
        };
        log::debug!("key {key} sym={sym:#x} shortcut={shortcut} consumed={} commit={:?}", reply.consumed, reply.commit);
        state.consumed_keys.insert(key, reply.consumed);
        if let Some(text) = reply.commit {
            // 动态调频:记录被选词,并立即落盘(下次启动可恢复)
            state.engine.learn(&text);
            if let Some(parent) = state.user_freq_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = state.engine.save_user_freq(&state.user_freq_path) {
                log::warn!("用户词频落盘失败: {e}");
            }
            // bigram 上文搭配落盘(与 user_freq 同目录派生路径 user_bigram.txt)
            let bigram_path = state.user_freq_path.with_file_name("user_bigram.txt");
            if let Err(e) = state.engine.save_bigram(&bigram_path) {
                log::warn!("用户 bigram 落盘失败: {e}");
            }
            // 用户造词落盘(随 commit 全量写,与 user_freq/bigram 一致)
            let dict_path = state.user_freq_path.with_file_name("user_dict.txt");
            if let Err(e) = state.engine.save_user_dict(&dict_path) {
                log::warn!("用户造词落盘失败: {e}");
            }
            ime::send_commit(state, &text);
            // 部分上屏(选字后还有剩余拼音在组字):重发 preedit + 重绘候选窗继续;选完才隐藏。
            if state.session.composing() {
                let preedit = state.session.render_preedit();
                ime::send_preedit(state, &preedit);
                crate::popup::redraw(state, qh);
            } else {
                crate::popup::hide(state);
            }
        } else if reply.preedit_dirty {
            let preedit = state.session.render_preedit();
            ime::send_preedit(state, &preedit);
            crate::popup::redraw(state, qh); // 候选变化,重绘候选窗
        }
        if !reply.consumed {
            forward_key(state, time, key, st_raw);
        }
    } else {
        if state.held.is_some_and(|h| h.key == key) {
            state.held = None; // 按住的键松开:停止重复
        }
        // 送 session:检测 Shift 单击(press 后 release 无其他键 = 切换中/英)。
        let sym = state
            .xkb_state
            .as_ref()
            .map(|xs| u32::from(xs.key_get_one_sym(xkb::Keycode::new(key + EVDEV_TO_XKB))))
            .unwrap_or(0);
        if state.session.on_release(sym) {
            // 中/英模式切换:刷新 preedit(英文=空)并隐藏候选窗。
            let preedit = state.session.render_preedit();
            ime::send_preedit(state, &preedit);
            crate::popup::hide(state);
            log::info!("输入模式: {}", if state.session.english { "英文" } else { "中文" });
        }
        // release 跟随 press 的消费决定
        match state.consumed_keys.remove(&key) {
            Some(false) => forward_key(state, time, key, st_raw),
            Some(true) => {}
            None => forward_key(state, time, key, st_raw), // 未知按键:保守转发
        }
    }
    out_sym
}

fn forward_key(state: &State, time: u32, key: u32, st: u32) {
    if !state.vkb_ready {
        return; // keymap 未同步前发 key 会触发协议错误,丢弃
    }
    if let Some(vkb) = &state.vkb {
        vkb.key(time, key, st);
    }
}

/// Ctrl/Alt/Super 任一活动即视为快捷键(不消费,转发)。
fn has_shortcut_modifier(xs: &xkb::State) -> bool {
    [xkb::MOD_NAME_CTRL, xkb::MOD_NAME_ALT, xkb::MOD_NAME_LOGO]
        .iter()
        .any(|m| xs.mod_name_is_active(m, xkb::STATE_MODS_EFFECTIVE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn us_state() -> xkb::State {
        let ctx = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let km = xkb::Keymap::new_from_names(&ctx, "evdev", "pc105", "us", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)
            .expect("本机应有 xkb 数据");
        xkb::State::new(&km)
    }

    #[test]
    fn plain_letter_is_not_shortcut() {
        assert!(!has_shortcut_modifier(&us_state()));
    }

    #[test]
    fn ctrl_down_is_shortcut() {
        let mut xs = us_state();
        // KEY_LEFTCTRL evdev=29 → xkb keycode 37
        xs.update_key(xkb::Keycode::new(29 + EVDEV_TO_XKB), xkb::KeyDirection::Down);
        assert!(has_shortcut_modifier(&xs));
    }

    #[test]
    fn super_down_is_shortcut() {
        let mut xs = us_state();
        // KEY_LEFTMETA evdev=125 → xkb keycode 133
        xs.update_key(xkb::Keycode::new(125 + EVDEV_TO_XKB), xkb::KeyDirection::Down);
        assert!(has_shortcut_modifier(&xs));
    }

    #[test]
    fn shift_alone_is_not_shortcut() {
        let mut xs = us_state();
        // KEY_LEFTSHIFT evdev=42 → xkb keycode 50
        xs.update_key(xkb::Keycode::new(42 + EVDEV_TO_XKB), xkb::KeyDirection::Down);
        assert!(!has_shortcut_modifier(&xs));
    }
}
