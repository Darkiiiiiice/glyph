//! 候选窗(M2):zwp_input_popup_surface_v2。
//!
//! compositor 负责把 popup 定位到文本光标附近,无需 layer-shell 手动算坐标。
//! text_input_rectangle 事件报告光标矩形(surface 局部坐标),供内容布局参考。
//! 当前为最小验证版:建 surface + 收事件,渲染(fontdue)随后接入。

use wayland_client::protocol::wl_buffer::WlBuffer;
use wayland_client::protocol::wl_shm;
use wayland_client::protocol::wl_shm_pool::WlShmPool;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, QueueHandle};

use crate::globals::State;
use glyph_frontend::protocol::input_method_v2::client::zwp_input_popup_surface_v2::{
    Event, ZwpInputPopupSurfaceV2,
};

/// activate 时创建候选窗(幂等):wl_surface + input_popup role。
pub fn ensure_popup(state: &mut State, qh: &QueueHandle<State>) {
    if state.popup.is_some() {
        return;
    }
    let (Some(ime), Some(comp)) = (&state.ime, &state.compositor) else { return };
    let surface = comp.create_surface(qh, ());
    let popup = ime.get_input_popup_surface(&surface, qh, ());
    log::info!("候选窗 popup surface 已创建");
    state.popup_surface = Some(surface);
    state.popup = Some(popup);
}

impl Dispatch<ZwpInputPopupSurfaceV2, ()> for State {
    fn event(
        state: &mut State,
        _: &ZwpInputPopupSurfaceV2,
        event: Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        match event {
            Event::TextInputRectangle { x, y, width, height } => {
                state.cursor_rect = Some((x, y, width, height));
                log::info!("光标矩形: {x},{y} {width}x{height}");
            }
            _ => {} // non_exhaustive 兜底
        }
    }
}

// wl_surface 有 enter/leave/preferred_buffer_scale 等事件,忽略。
impl Dispatch<WlSurface, ()> for State {
    fn event(
        _: &mut State,
        _: &WlSurface,
        _: <WlSurface as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

/// 候选窗重绘:渲染当前会话候选到 shm buffer 并提交。
/// 候选或拼音皆空时隐藏。显示尺寸随候选数自适应。
pub fn redraw(state: &mut State, qh: &QueueHandle<State>) {
    if state.renderer.is_none() {
        state.renderer = crate::render::Renderer::load();
    }
    let pinyin = state.session.buffer.clone();
    let cands: Vec<String> = state.session.page_candidates().iter().map(|c| c.text.clone()).collect();
    if pinyin.is_empty() && cands.is_empty() {
        hide(state);
        return;
    }
    let (Some(renderer), Some(surface), Some(shm)) =
        (&state.renderer, &state.popup_surface, &state.shm)
    else {
        return;
    };
    let (w, h, px) = renderer.render(&pinyin, &cands, 0);
    let stride = w as i32 * 4;
    let size = (stride * h as i32) as u64;
    log::debug!("redraw: 渲染完成 {}x{} 候选{} size={size}", w, h, cands.len());

    // shm buffer 用 memfd 承载像素:smithay 的 mmap 路径只认 memfd 型 fd
    // (M1 keymap 同款坑:普通临时文件 Failed to mmap)。memfd 匿名、无 /tmp 残留。
    use rustix::fs::{memfd_create, MemfdFlags};
    use std::io::Write;
    use std::os::fd::AsFd;
    let fd = match memfd_create("glyph-popup", MemfdFlags::empty()) {
        Ok(fd) => fd,
        Err(e) => {
            log::error!("memfd_create 失败: {e}");
            return;
        }
    };
    let mut file = std::fs::File::from(fd);
    if file.set_len(size).is_err() {
        log::error!("redraw: set_len 失败 size={size}");
        return;
    }
    let pool = shm.create_pool(file.as_fd(), size as i32, qh, ());
    let buffer =
        pool.create_buffer(0, w as i32, h as i32, stride, wl_shm::Format::Argb8888, qh, ());
    let bytes: Vec<u8> = px.iter().flat_map(|p| p.to_ne_bytes()).collect();
    if file.write_all(&bytes).is_err() {
        return;
    }
    surface.attach(Some(&buffer), 0, 0);
    surface.damage_buffer(0, 0, w as i32, h as i32);
    surface.commit();
    // buffer/pool/file 随局部作用域 drop:destroy/close 安全(compositor 已 dup fd)。
}

/// 隐藏候选窗(attach 空 buffer)。deactivate 与上屏后调用。
pub fn hide(state: &State) {
    if let Some(surface) = &state.popup_surface {
        surface.attach(None, 0, 0);
        surface.commit();
    }
}

// WlShmPool 无事件可 noop;WlBuffer 有 release 事件,手写忽略。
wayland_client::delegate_noop!(State: WlShmPool);
impl Dispatch<WlBuffer, ()> for State {
    fn event(
        _: &mut State,
        _: &WlBuffer,
        _: <WlBuffer as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}
