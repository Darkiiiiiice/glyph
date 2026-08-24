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
/// 候选池大小(convert 的 limit):翻页的数据源,支持 POOL/PAGE 页。
const POOL: usize = 90;

pub struct Session {
    /// 当前拼音字母串,如 "nihao";空串 = 未在组字。
    pub buffer: String,
    pub candidates: Vec<Candidate>,
    /// 当前页码(0-based),输入变化/上屏时重置。
    page: usize,
}

impl Session {
    pub fn new() -> Self {
        Self { buffer: String::new(), candidates: Vec::new(), page: 0 }
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
                match self.candidates.get(self.page * PAGE + idx) {
                    Some(c) => {
                        let text = c.text.clone();
                        self.clear();
                        Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                    }
                    None => Reply { consumed: true, ..Default::default() },
                }
            }
            K::KEY_space if self.composing() => match self.candidates.get(self.page * PAGE) {
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
            // 翻页:`-` 上一页、`=` 下一页(避开 `,` `.`,留给中文标点)。
            // 拼音不变,仅 preedit_dirty 触发候选窗重绘当前页。
            K::KEY_minus if self.composing() && self.page > 0 => {
                self.page -= 1;
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_equal if self.composing() && (self.page + 1) * PAGE < self.candidates.len() => {
                self.page += 1;
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

    /// 渲染 preedit 文本:仅拼音。候选由 M2 独立候选窗(popup)显示,
    /// 不再内联进 preedit——否则会出现横向 preedit 候选 + 竖向候选窗两套。
    pub fn render_preedit(&self) -> String {
        self.buffer.clone()
    }

    fn refresh(&mut self, engine: &Engine) {
        self.candidates =
            if self.buffer.is_empty() { Vec::new() } else { engine.convert(&self.buffer, POOL) };
        self.page = 0;
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.candidates.clear();
        self.page = 0;
    }
    /// 当前页候选(候选窗渲染的数据源)。
    pub fn page_candidates(&self) -> &[Candidate] {
        let start = (self.page * PAGE).min(self.candidates.len());
        &self.candidates[start..(start + PAGE).min(self.candidates.len())]
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
        assert_eq!(s.render_preedit(), "nihao", "preedit 只显拼音,候选交给候选窗");
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
    /// 构造 12 个同音节候选的引擎,用于翻页测试(词频递减 → 排序 w1..w12)。
    fn paged_fixture() -> Engine {
        let mut lex = String::new();
        for i in 1..=12 {
            lex.push_str(&format!("a w{} {}\n", i, 100 - i));
        }
        Engine::from_str(&lex)
    }

    #[test]
    fn paging_via_minus_equal() {
        use xkbcommon::xkb::keysyms as K;
        let e = paged_fixture();
        let mut s = Session::new();
        s.on_keysym(&e, 'a' as u32);
        assert_eq!(s.candidates.len(), 12, "候选池应取满 12 个(非 9)");
        assert_eq!(s.page_candidates().len(), 9);
        assert_eq!(s.page_candidates()[0].text, "w1");
        // 下一页:`=`
        let r = s.on_keysym(&e, K::KEY_equal);
        assert!(r.consumed && r.preedit_dirty);
        assert_eq!(s.page_candidates().len(), 3, "末页剩 3 个");
        assert_eq!(s.page_candidates()[0].text, "w10");
        // 数字选词选当前页页内第 k 个
        let r = s.on_keysym(&e, K::KEY_1);
        assert_eq!(r.commit.as_deref(), Some("w10"));
    }

    #[test]
    fn page_boundaries_and_reset() {
        use xkbcommon::xkb::keysyms as K;
        let e = paged_fixture();
        let mut s = Session::new();
        s.on_keysym(&e, 'a' as u32);
        s.on_keysym(&e, K::KEY_equal);
        assert_eq!(s.page, 1);
        // 末页再按 `=` 越界:落入丢弃分支(键转发)
        let r = s.on_keysym(&e, K::KEY_equal);
        assert!(!r.consumed && !s.composing());
        // 重新输入后页码重置
        s.on_keysym(&e, 'a' as u32);
        assert_eq!(s.page, 0);
        // 第 0 页按 `-` 不能上翻:丢弃拼音、键转发
        let r = s.on_keysym(&e, K::KEY_minus);
        assert!(!r.consumed && !s.composing());
    }
}
