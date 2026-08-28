//! Tab 单字模式(逐字定字)的切换与部分上屏行为。

use super::*;

#[test]
fn tab_char_mode_pick_char_keeps_remaining_pinyin() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    for ch in "nihao".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    assert_eq!(s.buffer, "nihao");
    // Tab 进单字模式:候选变首音节 ni 的单字
    s.on_keysym(&mut e, K::KEY_Tab);
    assert!(s.char_mode, "Tab 应进单字模式");
    assert!(s.candidates.iter().all(|c| c.text.chars().count() == 1), "单字模式候选全是单字");
    let (text, consumed) = s
        .candidates
        .iter()
        .find(|c| c.text == "你" && c.consumed == 2)
        .map(|c| (c.text.clone(), c.consumed))
        .expect("单字模式应有\"你\"");
    // 选"你":上屏 + 剩余 hao 续打 + 回整句模式
    let reply = s.pick(&mut e, text, consumed);
    assert_eq!(reply.commit.as_deref(), Some("你"));
    assert_eq!(s.buffer, "hao", "选字后剩余拼音继续组字");
    assert!(s.char_mode, "有剩余拼音时保持单字模式,连续逐字");
    assert!(s.candidates.iter().all(|c| c.text.chars().count() == 1), "候选仍是单字");
    // 连续选"好"(不用再按 Tab):选完退出单字模式
    let (t2, c2) = s
        .candidates
        .iter()
        .find(|c| c.text == "好")
        .map(|c| (c.text.clone(), c.consumed))
        .expect("hao 的单字应有\"好\"");
    let reply = s.pick(&mut e, t2, c2);
    assert_eq!(reply.commit.as_deref(), Some("好"));
    assert!(!s.char_mode && !s.composing(), "选完退出单字模式");
}

#[test]
fn tab_toggles_back_to_sentence_mode() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    for ch in "nihao".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    s.on_keysym(&mut e, K::KEY_Tab);
    assert!(s.char_mode);
    s.on_keysym(&mut e, K::KEY_Tab); // 再按 Tab 回整句模式
    assert!(!s.char_mode);
    assert!(s.candidates.iter().any(|c| c.text == "你好"), "回整句后候选恢复整句");
}

#[test]
fn escape_clears_char_mode() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32);
    s.on_keysym(&mut e, K::KEY_Tab);
    assert!(s.char_mode);
    s.on_keysym(&mut e, K::KEY_Escape);
    assert!(!s.composing() && !s.char_mode, "Esc 取消并退出单字模式");
}
