//! 应用状态与 Wayland registry 全局发现。
//! 所有协议对象的事件都汇总到 [`State`],各模块分别实现对应 Dispatch。

use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_registry;
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_shm::WlShm;
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use xkbcommon::xkb;

use glyph_engine::Engine;

use glyph_frontend::protocol::input_method_v2::client::zwp_input_method_keyboard_grab_v2::ZwpInputMethodKeyboardGrabV2;
use glyph_frontend::protocol::input_method_v2::client::zwp_input_method_manager_v2::ZwpInputMethodManagerV2;
use glyph_frontend::protocol::input_method_v2::client::zwp_input_method_v2::ZwpInputMethodV2;
use glyph_frontend::protocol::virtual_keyboard_v1::client::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use glyph_frontend::protocol::virtual_keyboard_v1::client::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;
use crate::session::Session;

/// 全局共享状态:registry 结果 + IME 会话 + xkb + 引擎。
pub struct State {
    // --- 全局对象 ---
    pub im_manager: Option<ZwpInputMethodManagerV2>,
    pub vkb_manager: Option<ZwpVirtualKeyboardManagerV1>,
    pub ti_v3_seen: bool,
    pub seat: Option<WlSeat>,
    pub compositor: Option<WlCompositor>,
    pub shm: Option<WlShm>,
    // --- 候选窗(M2):popup surface + 光标矩形 ---
    pub popup_surface: Option<wayland_client::protocol::wl_surface::WlSurface>,
    pub popup: Option<glyph_frontend::protocol::input_method_v2::client::zwp_input_popup_surface_v2::ZwpInputPopupSurfaceV2>,
    /// compositor 报告的文本光标矩形(相对焦点 surface)。
    pub cursor_rect: Option<(i32, i32, i32, i32)>,
    /// 候选窗渲染器(M2,lazy 加载 CJK 字体)。
    pub renderer: Option<glyph_frontend::render::Renderer>,
    // --- im-v2 会话 ---
    pub ime: Option<ZwpInputMethodV2>,
    pub ime_active: bool,
    /// 已收 done 事件数:commit(serial) 的 serial 必须等于它。
    pub done_count: u32,
    pub grab: Option<ZwpInputMethodKeyboardGrabV2>,
    // --- virtual-keyboard 转发通道(未消费按键放回 compositor) ---
    pub vkb: Option<ZwpVirtualKeyboardV1>,
    /// vkb keymap 已成功发送;未就绪前不得发 key/modifiers(协议错误)。
    pub vkb_ready: bool,
    /// vkb 未就绪时缓存的最近一次 modifiers,keymap 同步后补发。
    pub pending_modifiers: Option<(u32, u32, u32, u32)>,
    // --- xkb ---
    pub xkb_state: Option<xkb::State>,
    // --- 引擎与拼音会话 ---
    pub engine: Engine,
    pub session: Session,
    /// 用户配置:候选窗外观 style 供 popup 渲染,punct_cn 决定 session 标点初值。
    pub config: glyph_frontend::config::Config,
    /// 用户词频落盘路径(可能不存在;save 时懒建目录)。
    pub user_freq_path: std::path::PathBuf,
    /// 按键消费一致性表:keycode → press 时是否被 IME 消费;
    /// release 必须跟随 press 的决定,否则应用会收到孤儿 release。
    pub consumed_keys: std::collections::HashMap<u32, bool>,
    /// 长按重复:compositor 报告的 (rate 次/秒, delay ms);rate==0 = 禁用。
    pub repeat_info: Option<(u32, u32)>,
    /// 当前按住的可重复键:主循环 poll 超时到点后由 repeat::tick 重放 press。
    pub held: Option<crate::repeat::HeldKey>,
}

impl State {
    pub fn new(engine: Engine, user_freq_path: std::path::PathBuf, config: glyph_frontend::config::Config) -> Self {
        Self {
            im_manager: None,
            vkb_manager: None,
            ti_v3_seen: false,
            seat: None,
            compositor: None,
            shm: None,
            popup_surface: None,
            popup: None,
            cursor_rect: None,
            renderer: None,
            ime: None,
            ime_active: false,
            done_count: 0,
            grab: None,
            vkb: None,
            vkb_ready: false,
            pending_modifiers: None,
            xkb_state: None,
            engine,
            session: Session::new(config.punct_cn, config.page_size),
            config,
            user_freq_path,
            consumed_keys: std::collections::HashMap::new(),
            repeat_info: None,
            held: None,
        }
    }
}

/// 连接 compositor 并枚举 registry,返回连接、事件队列与就绪状态。
pub fn connect(engine: Engine, user_freq_path: std::path::PathBuf, config: glyph_frontend::config::Config) -> Result<(Connection, EventQueue<State>, State), String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("connect wayland: {e}"))?;
    let display = conn.display();
    let mut eq = conn.new_event_queue();
    let qh = eq.handle();
    let _registry = display.get_registry(&qh, ());
    let mut state = State::new(engine, user_freq_path, config);
    eq.roundtrip(&mut state).map_err(|e| format!("registry roundtrip: {e}"))?;
    Ok((conn, eq, state))
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global { name, interface, version } = event {
            log::debug!("global: {interface} v{version} (name {name})");
            match interface.as_str() {
                "zwp_input_method_manager_v2" => {
                    state.im_manager =
                        Some(registry.bind::<ZwpInputMethodManagerV2, _, _>(name, version.min(1), qh, ()));
                    log::info!("bound zwp_input_method_manager_v2");
                }
                "zwp_virtual_keyboard_manager_v1" => {
                    state.vkb_manager =
                        Some(registry.bind::<ZwpVirtualKeyboardManagerV1, _, _>(name, version.min(1), qh, ()));
                    log::info!("bound zwp_virtual_keyboard_manager_v1");
                }
                "zwp_text_input_v3" | "zwp_text_input_manager_v3" => state.ti_v3_seen = true,
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(7), qh, ()));
                }
                "wl_compositor" => {
                    state.compositor =
                        Some(registry.bind::<WlCompositor, _, _>(name, version.min(5), qh, ()));
                    log::info!("bound wl_compositor");
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<WlShm, _, _>(name, version.min(1), qh, ()));
                    log::info!("bound wl_shm");
                }
                _ => {}
            }
        }
    }
}

// 管理器、vkb、compositor 无事件,可 noop;WlSeat 与 WlShm 有事件,必须手写忽略。
wayland_client::delegate_noop!(State: ZwpInputMethodManagerV2);
wayland_client::delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(State: ZwpVirtualKeyboardV1);
wayland_client::delegate_noop!(State: WlCompositor);

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// wl_shm 绑定后立即收到一批 format 事件(公告支持的像素格式),忽略即可。
impl Dispatch<WlShm, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: <WlShm as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
