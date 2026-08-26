//! im-v2 状态机:activate/deactivate 生命周期、done 计数、preedit 与
//! commit 的发送助手。键盘事件处理在 keyboard.rs。

use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::globals::State;
use glyph_frontend::protocol::input_method_v2::client::zwp_input_method_v2::{
    Event, ZwpInputMethodV2,
};

impl Dispatch<ZwpInputMethodV2, ()> for State {
    fn event(
        state: &mut Self,
        ime: &ZwpInputMethodV2,
        event: Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            Event::Activate => {
                log::info!("im-v2 activate");
                state.ime_active = true;
                // 每次激活重新 grab(协议:grab 绑定一次会话;deactivate 后失效)
                if state.grab.is_none() {
                    let grab = ime.grab_keyboard(qh, ());
                    state.grab = Some(grab);
                }
                // 建立 vkb 转发通道(幂等)
                crate::keyboard::ensure_vkb(state, qh);
                // 建立候选窗 popup surface(幂等,M2)
                crate::popup::ensure_popup(state, qh);
            }
            Event::Deactivate => {
                log::info!("im-v2 deactivate");
                state.ime_active = false;
                // 放弃进行中的组字,清 preedit 与候选窗
                if state.session.composing() {
                    state.session = crate::session::Session::new(state.config.punct_cn, state.config.page_size);
                    send_preedit(state, "");
                }
                crate::popup::hide(state);
                if let Some(grab) = state.grab.take() {
                    grab.release();
                }
                state.consumed_keys.clear();
                state.held = None; // 键盘随 grab 销毁,停止重复
            }
            Event::Done => {
                state.done_count = state.done_count.wrapping_add(1);
            }
            Event::Unavailable => {
                log::error!("im-v2 unavailable — compositor 撤回了输入法对象");
                state.ime = None;
            }
            _ => {}
        }
    }
}

/// 发送 preedit(光标置于文本末尾,字节偏移)。
pub fn send_preedit(state: &State, text: &str) {
    if let Some(ime) = &state.ime {
        log::info!("preedit → {text:?}");
        let end = text.len() as i32;
        ime.set_preedit_string(text.to_string(), 0, end);
        ime.commit(state.done_count);
    }
}

/// 上屏文本(commit_string),随后清空 preedit。
pub fn send_commit(state: &State, text: &str) {
    if let Some(ime) = &state.ime {
        log::info!("commit → {text:?}");
        ime.commit_string(text.to_string());
        ime.set_preedit_string(String::new(), 0, 0);
        ime.commit(state.done_count);
    }
}
