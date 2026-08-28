use super::*;
use xkbcommon::xkb::keysyms as K;

mod bigram;
mod char_mode;
mod coin;
mod paging;
mod word_char;

fn fixture() -> Engine {
    Engine::from_str("ni'hao 你好 10000\nni 你 500\nhao 好 300\n")
}

#[test]
fn letters_accumulate_and_commit_on_number() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    for ch in ['n', 'i', 'h', 'a', 'o'] {
        let r = s.on_keysym(&mut e, ch as u32);
        assert!(r.consumed && r.preedit_dirty);
    }
    assert_eq!(s.buffer, "nihao");
    assert_eq!(s.render_preedit(), "nihao", "preedit 只显拼音,候选交给候选窗");
    let r = s.on_keysym(&mut e, xkbcommon::xkb::keysyms::KEY_1);
    assert_eq!(r.commit.as_deref(), Some("你好"));
    assert!(s.buffer.is_empty());
}

#[test]
fn space_commits_first_candidate() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32);
    let r = s.on_keysym(&mut e, xkbcommon::xkb::keysyms::KEY_space);
    assert_eq!(r.commit.as_deref(), Some("你"));
}

#[test]
fn backspace_edits_buffer_then_forwards_when_empty() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    let r = s.on_keysym(&mut e, xkbcommon::xkb::keysyms::KEY_BackSpace);
    assert!(r.consumed && s.buffer.is_empty());
    // buffer 已空:backspace 属于应用(删文本)
    let r = s.on_keysym(&mut e, xkbcommon::xkb::keysyms::KEY_BackSpace);
    assert!(!r.consumed);
}

#[test]
fn escape_cancels_without_commit() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    let r = s.on_keysym(&mut e, xkbcommon::xkb::keysyms::KEY_Escape);
    assert!(r.consumed && r.commit.is_none() && r.preedit_dirty);
    assert!(!s.composing());
}

#[test]
fn unrelated_key_forwards_and_drops_buffer() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    let r = s.on_keysym(&mut e, xkbcommon::xkb::keysyms::KEY_F1);
    assert!(!r.consumed, "键仍转发给应用");
        assert_eq!(r.commit.as_deref(), Some("n"), "组字中无关键上屏拼音原文,不丢输入");
    assert!(!s.composing());
}
#[test]
fn cn_punct_commit() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    // 空闲打 `.` → 中文句号
    let r = s.on_keysym(&mut e, K::KEY_period);
    assert_eq!(r.commit.as_deref(), Some("。"));
    assert!(r.consumed && !r.preedit_dirty);
    // 组字中打 `.` → 当前页首选 + 句号
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32);
    let r = s.on_keysym(&mut e, K::KEY_period);
    assert_eq!(r.commit.as_deref(), Some("你。"));
    assert!(r.consumed && r.preedit_dirty && !s.composing());
}

#[test]
fn punct_english_mode_passthrough() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.toggle_punct(); // 切英文标点
    // 英文模式空闲打 `.`:不在映射生效,转发给应用(不消费)
    let r = s.on_keysym(&mut e, K::KEY_period);
    assert!(!r.consumed && r.commit.is_none());
    // 切回中文又生效
    s.toggle_punct();
    let r = s.on_keysym(&mut e, K::KEY_comma);
    assert_eq!(r.commit.as_deref(), Some(","));
}
#[test]
fn quote_pairs_alternate() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    // 双引号开闭交替
    assert_eq!(s.on_keysym(&mut e, K::KEY_quotedbl).commit.as_deref(), Some("\u{201C}"));
    assert_eq!(s.on_keysym(&mut e, K::KEY_quotedbl).commit.as_deref(), Some("\u{201D}"));
    assert_eq!(s.on_keysym(&mut e, K::KEY_quotedbl).commit.as_deref(), Some("\u{201C}"));
    // 单引号独立配对
    assert_eq!(s.on_keysym(&mut e, K::KEY_apostrophe).commit.as_deref(), Some("\u{2018}"));
    assert_eq!(s.on_keysym(&mut e, K::KEY_apostrophe).commit.as_deref(), Some("\u{2019}"));
}
#[test]
fn modifier_keys_do_not_disturb_composing() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32);
    // 组字中按 Shift(欲打引号的 Shift+'):不上屏、不丢拼音,键转发
    let r = s.on_keysym(&mut e, K::KEY_Shift_L);
    assert!(!r.consumed && r.commit.is_none());
    assert!(s.composing() && s.buffer == "ni", "Shift 不应打断组字: buffer={}", s.buffer);
}
#[test]
fn shift_click_toggles_english() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    // 单击 Shift → 英文
    s.on_keysym(&mut e, K::KEY_Shift_L);
    assert!(s.on_release(K::KEY_Shift_L));
    assert!(s.english);
    // 英文模式:字母转发(不消费、不上屏、不进拼音)
    let r = s.on_keysym(&mut e, 'n' as u32);
    assert!(!r.consumed && r.commit.is_none() && s.buffer.is_empty());
    // 单击 Shift → 切回中文
    s.on_keysym(&mut e, K::KEY_Shift_L);
    assert!(s.on_release(K::KEY_Shift_L));
    assert!(!s.english);
    // 中文模式:字母进拼音
    assert!(s.on_keysym(&mut e, 'n' as u32).consumed);
}

#[test]
fn shift_chord_is_not_click() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, K::KEY_Shift_L);
    s.on_keysym(&mut e, 'n' as u32); // Shift+n:搭配了其他键
    assert!(!s.on_release(K::KEY_Shift_L), "Shift+字母不是单击,不应切换");
    assert!(!s.english);
}
#[test]
fn shift_click_while_composing_clears_buffer() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32); // 组字中
    assert!(s.composing());
    // 组字中单击 Shift:切英文 + 丢弃拼音缓冲(drop,不上屏拼音原文)
    s.on_keysym(&mut e, K::KEY_Shift_L);
    assert!(s.on_release(K::KEY_Shift_L));
    assert!(s.english && !s.composing(), "切入英文应清空拼音缓冲,候选窗随之隐藏");
}


#[test]
fn pick_full_candidate_clears_buffer() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    for ch in "nihao".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    let (text, consumed) = s
        .candidates
        .iter()
        .find(|c| c.text == "你好")
        .map(|c| (c.text.clone(), c.consumed))
        .unwrap();
    let reply = s.pick(&mut e, text, consumed);
    assert_eq!(reply.commit.as_deref(), Some("你好"));
    assert_eq!(s.buffer, "", "整句候选应清空缓冲");
    assert!(s.candidates.is_empty());
}
