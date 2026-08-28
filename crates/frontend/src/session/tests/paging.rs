//! 翻页(- 上一页、= 下一页)与页边界行为。

use super::*;

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
    let mut e = paged_fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'a' as u32);
    assert_eq!(s.candidates.len(), 12, "候选池应取满 12 个(非 9)");
    assert_eq!(s.page_candidates().len(), 9);
    assert_eq!(s.page_candidates()[0].text, "w1");
    // 下一页:`=`
    let r = s.on_keysym(&mut e, K::KEY_equal);
    assert!(r.consumed && r.preedit_dirty);
    assert_eq!(s.page_candidates().len(), 3, "末页剩 3 个");
    assert_eq!(s.page_candidates()[0].text, "w10");
    // 数字选词选当前页页内第 k 个
    let r = s.on_keysym(&mut e, K::KEY_1);
    assert_eq!(r.commit.as_deref(), Some("w10"));
}

#[test]
fn page_boundaries_and_reset() {
    let mut e = paged_fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'a' as u32);
    s.on_keysym(&mut e, K::KEY_equal);
    assert_eq!(s.page, 1);
    // 末页再按 `=` 越界:消费但不动,候选保留(不取消上屏)
    let r = s.on_keysym(&mut e, K::KEY_equal);
    assert!(r.consumed && s.composing() && s.page == 1);
    // 回第 0 页,再按 `-` 不能上翻:消费但不动,候选保留
    s.on_keysym(&mut e, K::KEY_minus);
    assert_eq!(s.page, 0);
    let r = s.on_keysym(&mut e, K::KEY_minus);
    assert!(r.consumed && s.composing() && s.page == 0);
    // Escape 重输后页码重置
    s.on_keysym(&mut e, K::KEY_Escape);
    s.on_keysym(&mut e, 'a' as u32);
    assert_eq!(s.page, 0);
}

#[test]
fn page_keys_at_boundary_dont_dismiss() {
    let mut e = fixture();
    let mut s = Session::new(true, 9);
    s.on_keysym(&mut e, 'n' as u32);
    s.on_keysym(&mut e, 'i' as u32); // 仅"你"一候选(单页,第一页=最后一页)
    // 第一页按 `-`:消费但不动,候选不取消、不上屏
    let r = s.on_keysym(&mut e, K::KEY_minus);
    assert!(r.consumed && r.commit.is_none() && s.composing() && s.page == 0);
    // 最后一页按 `=`:消费但不动,候选不取消、不上屏
    let r = s.on_keysym(&mut e, K::KEY_equal);
    assert!(r.consumed && r.commit.is_none() && s.composing() && s.page == 0);
}
