//! bigram 上文搭配:prev_word 传递与排序上浮的 session 层测试。

use super::*;

#[test]
fn bigram_prev_word_affects_candidates() {
    let mut e = Engine::from_str("wo'men 我们 9000\nxue'xi 学习 100\nxue'xi 穴息 5000\n");
    let mut s = Session::new(true, 9);
    for _ in 0..5 {
        e.learn_bigram("我们", "学习");
    }
    // 无上文:高频"穴息"第一
    for ch in "xuexi".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    assert_eq!(s.candidates[0].text, "穴息");
    s.on_keysym(&mut e, K::KEY_Escape); // 清空拼音缓冲
    // 上屏"我们" → prev_word 记为"我们"
    for ch in "women".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    s.on_keysym(&mut e, K::KEY_1);
    // 再打 xuexi:prev_word="我们",bigram 搭配让"学习"上浮第一
    for ch in "xuexi".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    assert_eq!(s.candidates[0].text, "学习", "上文\"我们\"+bigram 应让\"学习\"上浮");
}
