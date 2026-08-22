//! 拼音会话状态机:按键 → 拼音缓冲 → 候选 → 上屏决策。
//! 纯逻辑、不碰 Wayland,便于单元测试。

use glyph_engine::{Candidate, Engine};

/// 一次按键的处理结果。
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    /// IME 是否消费该键;false 表示应经 virtual-keyboard 转发回 compositor。
    pub consumed: bool,
    /// 需要上屏(commit_string)的文本。
    pub commit: Option<String>,
    /// preedit 内容已变,需要重发 set_preedit_string。
    pub preedit_dirty: bool,
}

/// 一页候选数(数字键 1-9 直选)。
const PAGE: usize = 9;

pub struct Session {
    /// 当前拼音字母串,如 "nihao";空串 = 未在组字。
    pub buffer: String,
    pub candidates: Vec<Candidate>,
}

impl Session {
    pub fn new() -> Self {
        Self { buffer: String::new(), candidates: Vec::new() }
    }

    pub fn composing(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// keysym 路由。sym 为 xkb keysym(已含 shift 等修饰后的结果)。
    pub fn on_keysym(&mut self, engine: &Engine, sym: u32) -> Reply {
        use xkbcommon::xkb::keysyms as K;
        match sym {
            s if (K::KEY_a..=K::KEY_z).contains(&s) => {
                self.buffer.push(char::from_u32(s).unwrap());
                self.refresh(engine);
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_1..=K::KEY_9 if self.composing() => {
                let idx = (sym - K::KEY_1) as usize;
                match self.candidates.get(idx) {
                    Some(c) => {
                        let text = c.text.clone();
                        self.clear();
                        Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                    }
                    None => Reply { consumed: true, ..Default::default() },
                }
            }
            K::KEY_space if self.composing() => match self.candidates.first() {
                Some(c) => {
                    let text = c.text.clone();
                    self.clear();
                    Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                }
                None => {
                    // 无候选:上屏原文,避免吞键
                    let text = self.buffer.clone();
                    self.clear();
                    Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                }
            },
            K::KEY_BackSpace if self.composing() => {
                self.buffer.pop();
                self.refresh(engine);
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_Return if self.composing() => {
                let text = self.buffer.clone();
                self.clear();
                Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
            }
            K::KEY_Escape if self.composing() => {
                self.clear();
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            // 其余键:若正在组字则丢弃拼音(简化决策),键本身转发
            _ => {
                if self.composing() {
                    self.clear();
                    Reply { consumed: false, preedit_dirty: true, ..Default::default() }
                } else {
                    Reply::default()
                }
            }
        }
    }

    /// 渲染 preedit 文本:`拼音 + 编号候选`,光标由 ime 层置于末尾。
    pub fn render_preedit(&self) -> String {
        if self.buffer.is_empty() {
            return String::new();
        }
        let mut s = self.buffer.clone();
        for (i, c) in self.candidates.iter().take(PAGE).enumerate() {
            s.push_str(&format!(" {}.{}", i + 1, c.text));
        }
        s
    }

    fn refresh(&mut self, engine: &Engine) {
        self.candidates =
            if self.buffer.is_empty() { Vec::new() } else { engine.convert(&self.buffer, PAGE) };
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.candidates.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Engine {
        Engine::from_str("ni'hao 你好 10000\nni 你 500\nhao 好 300\n")
    }

    #[test]
    fn letters_accumulate_and_commit_on_number() {
        let e = fixture();
        let mut s = Session::new();
        for ch in ['n', 'i', 'h', 'a', 'o'] {
            let r = s.on_keysym(&e, ch as u32);
            assert!(r.consumed && r.preedit_dirty);
        }
        assert_eq!(s.buffer, "nihao");
        assert!(s.render_preedit().contains("1.你好"));
        let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_1);
        assert_eq!(r.commit.as_deref(), Some("你好"));
        assert!(s.buffer.is_empty());
    }

    #[test]
    fn space_commits_first_candidate() {
        let e = fixture();
        let mut s = Session::new();
        s.on_keysym(&e, 'n' as u32);
        s.on_keysym(&e, 'i' as u32);
        let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_space);
        assert_eq!(r.commit.as_deref(), Some("你"));
    }

    #[test]
    fn backspace_edits_buffer_then_forwards_when_empty() {
        let e = fixture();
        let mut s = Session::new();
        s.on_keysym(&e, 'n' as u32);
        let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_BackSpace);
        assert!(r.consumed && s.buffer.is_empty());
        // buffer 已空:backspace 属于应用(删文本)
        let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_BackSpace);
        assert!(!r.consumed);
    }

    #[test]
    fn escape_cancels_without_commit() {
        let e = fixture();
        let mut s = Session::new();
        s.on_keysym(&e, 'n' as u32);
        let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_Escape);
        assert!(r.consumed && r.commit.is_none() && r.preedit_dirty);
        assert!(!s.composing());
    }

    #[test]
    fn unrelated_key_forwards_and_drops_buffer() {
        let e = fixture();
        let mut s = Session::new();
        s.on_keysym(&e, 'n' as u32);
        let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_F1);
        assert!(!r.consumed && r.commit.is_none());
        assert!(!s.composing());
    }
}
