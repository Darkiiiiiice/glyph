//! 逐字造词链(session/coin.rs)的行为契约:连续逐字定字学成词,断链不学。

use super::*;

/// 两个单字音节,词库无"魑魅"词。
fn coin_fixture() -> Engine {
    // 高频"的"模拟真实 total 量级:整词 overlay 边才能压过单字拼合路径赢去重
    Engine::from_str("chi 魑 500\nmei 魅 500\nde 的 100000\n")
}

fn type_input(s: &mut Session, e: &mut Engine, input: &str) {
    for ch in input.chars() {
        s.on_keysym(e, ch as u32);
    }
}

/// 词库已按拼接出"魑魅"(魑+魅 两条单字边),造出的词与其判别在分词路径:
/// 拼接候选 words == ["魑","魅"],用户词 words == ["魑魅"]。
fn coined(e: &Engine) -> bool {
    e.convert("chimei", 9).iter().any(|c| c.words == ["魑魅"])
}

#[test]
fn consecutive_char_picks_coin_word() {
    let mut e = coin_fixture();
    let mut s = Session::new(true, 9);
    type_input(&mut s, &mut e, "chimei");
    s.on_keysym(&mut e, K::KEY_Tab); // 进单字模式
    let r = s.on_keysym(&mut e, K::KEY_1);
    assert_eq!(r.commit.as_deref(), Some("魑"));
    assert!(s.composing(), "剩余拼音继续组字");
    let r = s.on_keysym(&mut e, K::KEY_1);
    assert_eq!(r.commit.as_deref(), Some("魅"));
    assert!(!s.composing());
    assert!(coined(&e), "连续逐字应学成\"魑魅\"");
}

#[test]
fn escape_aborts_coining() {
    let mut e = coin_fixture();
    let mut s = Session::new(true, 9);
    type_input(&mut s, &mut e, "chimei");
    s.on_keysym(&mut e, K::KEY_Tab);
    s.on_keysym(&mut e, K::KEY_1); // 选了"魑"
    s.on_keysym(&mut e, K::KEY_Escape); // 取消剩余拼音 → 断链
    assert!(!coined(&e));
}

#[test]
fn word_pick_breaks_chain() {
    let mut e = Engine::from_str("chi 魑 500\nmei 魅 500\nchi'mei 痴迷 800\n");
    let mut s = Session::new(true, 9);
    type_input(&mut s, &mut e, "chimei");
    s.on_keysym(&mut e, K::KEY_Tab);
    s.on_keysym(&mut e, K::KEY_1); // 逐字选"魑"
    s.on_keysym(&mut e, K::KEY_Tab); // 退回整句模式 → 断链
    // 剩余拼音只有 "mei",整词选"魅"(链已在 Tab 时断)
    let r = s.on_keysym(&mut e, K::KEY_1);
    assert_eq!(r.commit.as_deref(), Some("魅"));
    assert!(!e.convert("chimei", 9).iter().any(|c| c.words == ["魑魅"]), "混合选法不造词");
}

#[test]
fn backspace_breaks_chain() {
    let mut e = coin_fixture();
    let mut s = Session::new(true, 9);
    type_input(&mut s, &mut e, "chimei");
    s.on_keysym(&mut e, K::KEY_Tab);
    s.on_keysym(&mut e, K::KEY_1); // 逐字选"魑"
    s.on_keysym(&mut e, K::KEY_BackSpace); // 退格纠错 → 断链
    type_input(&mut s, &mut e, "i"); // 补回 "mei"
    // 单字模式已随退格后的输入保持?BackSpace 不改 char_mode,仍是单字模式
    s.on_keysym(&mut e, K::KEY_1); // 选"魅"——链已断,只剩 1 字,不结算
    assert!(!coined(&e));
}

#[test]
fn single_char_never_coins() {
    let mut e = coin_fixture();
    let mut s = Session::new(true, 9);
    type_input(&mut s, &mut e, "chi");
    s.on_keysym(&mut e, K::KEY_Tab);
    s.on_keysym(&mut e, K::KEY_1); // 整句只选了一个字
    assert!(!coined(&e), "链长 <2 不造词");
}
