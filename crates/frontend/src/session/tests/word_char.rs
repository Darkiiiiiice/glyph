//! 以词定字的行为契约:整句模式下 `[`/`]` 取当前页首选的首/尾字上屏。
//! 词认得、只要其中一个字时免进单字模式;整句候选必全长消耗(convert 语义),上屏即选完。

use super::*;

#[test]
fn bracket_picks_first_and_last_char() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    for ch in "nihao".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    assert_eq!(s.candidates[0].text, "你好");
    let r = s.on_keysym(&mut e, K::KEY_bracketleft);
    assert_eq!(r.commit.as_deref(), Some("你"), "`[` 取首字");
    assert!(r.consumed && !s.composing());
    assert_eq!(s.ctx.prev1(), Some("你"), "上屏字记为上屏历史(bigram 上文)");
    for ch in "nihao".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    let r = s.on_keysym(&mut e, K::KEY_bracketright);
    assert_eq!(r.commit.as_deref(), Some("好"), "`]` 取尾字");
    assert!(r.consumed && !s.composing());
}

#[test]
fn bracket_falls_through_without_candidates() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'g' as u32);
    s.on_keysym(&mut e, 'g' as u32);
    assert!(s.candidates.is_empty(), "非法音节无候选");
    // 有拼音但无候选:定字无从谈起,走默认分支(上屏原文 + 键转发)
    let r = s.on_keysym(&mut e, K::KEY_bracketleft);
    assert_eq!(r.commit.as_deref(), Some("gg"));
    assert!(!r.consumed, "键本身转发给应用");
}

#[test]
fn bracket_idle_forwards() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    // 空闲:不在组字,`[`/`]` 是应用的字面键(快捷键、markdown 等)
    let r = s.on_keysym(&mut e, K::KEY_bracketleft);
    assert!(!r.consumed && r.commit.is_none());
    let r = s.on_keysym(&mut e, K::KEY_bracketright);
    assert!(!r.consumed && r.commit.is_none());
}

#[test]
fn bracket_in_char_mode_falls_through() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32);
    s.on_keysym(&mut e, K::KEY_Tab);
    assert!(s.char_mode);
    // 单字模式候选本就是单字,定字无意义:守卫排除,落默认分支上屏原文
    let r = s.on_keysym(&mut e, K::KEY_bracketleft);
    assert_eq!(r.commit.as_deref(), Some("ni"));
    assert!(!r.consumed);
}
