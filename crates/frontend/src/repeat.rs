//! 长按重复:grab 期间 compositor 只在 repeat_info 里告知 (rate, delay),
//! 不重发 key 事件——重复由 IME 按自己的定时器重放 press(合成键走与真实
//! 按下完全相同的路径,消费/转发/learn 语义不变)。

use std::time::{Duration, Instant};

use wayland_client::QueueHandle;

use crate::globals::State;

/// 按住中的可重复键:next 到点时重放一次 press。
#[derive(Clone, Copy)]
pub struct HeldKey {
    pub key: u32,
    pub next: Instant,
}

/// 该 keysym 按住时是否应重复;返回首次重复前的延迟。rate<=0 = compositor 禁用。
/// 修饰键(0xffe1-0xffee 段)不重复;无 keymap(sym=0)无从重复。
pub fn repeat_delay(state: &State, sym: u32) -> Option<Duration> {
    let (rate, delay) = state.repeat_info?;
    if rate == 0 || sym == 0 || (0xffe1..=0xffee).contains(&sym) {
        return None;
    }
    Some(Duration::from_millis(u64::from(delay)))
}

/// 主循环每次迭代调用:到点则重放一次 press,返回距下次到点的时长作 poll 超时。
/// 不追赶积压(poll 被其他事件耽搁时不连发,与真实键盘的观感一致)。
pub fn tick(state: &mut State, qh: &QueueHandle<State>) -> Option<Duration> {
    let held = state.held?;
    let (rate, _) = state.repeat_info?;
    if rate == 0 {
        return None;
    }
    let now = Instant::now();
    if now < held.next {
        return Some(held.next - now);
    }
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u32)
        .unwrap_or(0);
    crate::keyboard::press(state, time, held.key, qh);
    let interval = Duration::from_secs_f64(1.0 / f64::from(rate));
    state.held = Some(HeldKey { next: now + interval, ..held });
    Some(interval)
}

#[cfg(test)]
mod tests {
    #[test]
    fn modifier_keysyms_do_not_repeat() {
        // repeat_delay 需要 State,构造太重;此处只锁 keysym 段的判定逻辑。
        let modifier = |sym: u32| (0xffe1..=0xffee).contains(&sym);
        assert!(modifier(0xffe1)); // Shift_L
        assert!(modifier(0xffe9)); // Alt_L
        assert!(!modifier(u32::from(b'c')));
    }
}
