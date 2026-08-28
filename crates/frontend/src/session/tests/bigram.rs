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

fn type_and_pick(s: &mut Session, e: &mut Engine, input: &str, key: u32) {
    for ch in input.chars() {
        s.on_keysym(e, ch as u32);
    }
    s.on_keysym(e, key);
}

#[test]
fn trigram_learns_from_two_word_history() {
    // 端到端接线:连续上屏"我们""爱"后选"学习" → trigram(我们,爱)→学习 学成。
    // gap ln(245)≈5.50:bigram(4.16) 单独翻不动、唯 trigram(6.93) 能翻——隔离验证接线。
    // (user_freq 的 learn 在 keyboard 层 commit 回调,session 测试不涉及,故不计入。)
    let mut e = Engine::from_str("wo'men 我们 9000\nai 爱 8000\nxue'xi 穴息 24500\nxue'xi 学习 100\n");
    let mut s = Session::new(true, 9);
    type_and_pick(&mut s, &mut e, "women", K::KEY_1); // 我们
    type_and_pick(&mut s, &mut e, "ai", K::KEY_1); // 爱 → 历史 [爱, 我们]
    for ch in "xuexi".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    assert_eq!(s.candidates[0].text, "穴息", "无任何记录时静态第一");
    s.on_keysym(&mut e, K::KEY_2); // 选"学习":user_freq/bigram/trigram 同时记录
    type_and_pick(&mut s, &mut e, "women", K::KEY_1);
    type_and_pick(&mut s, &mut e, "ai", K::KEY_1); // 历史重回 [爱, 我们]
    for ch in "xuexi".chars() {
        s.on_keysym(&mut e, ch as u32);
    }
    assert_eq!(s.candidates[0].text, "学习", "双词上文 trigram 应翻 gap 9.7");
}
