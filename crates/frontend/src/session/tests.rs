use super::*;

fn fixture() -> Engine {
    Engine::from_str("ni'hao 你好 10000\nni 你 500\nhao 好 300\n")
}

#[test]
fn letters_accumulate_and_commit_on_number() {
    let e = fixture();
    let mut s = Session::new(true);
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
    let mut s = Session::new(true);
    s.on_keysym(&e, 'n' as u32);
    s.on_keysym(&e, 'i' as u32);
    let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_space);
    assert_eq!(r.commit.as_deref(), Some("你"));
}

#[test]
fn backspace_edits_buffer_then_forwards_when_empty() {
    let e = fixture();
    let mut s = Session::new(true);
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
    let mut s = Session::new(true);
    s.on_keysym(&e, 'n' as u32);
    let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_Escape);
    assert!(r.consumed && r.commit.is_none() && r.preedit_dirty);
    assert!(!s.composing());
}

#[test]
fn unrelated_key_forwards_and_drops_buffer() {
    let e = fixture();
    let mut s = Session::new(true);
    s.on_keysym(&e, 'n' as u32);
    let r = s.on_keysym(&e, xkbcommon::xkb::keysyms::KEY_F1);
    assert!(!r.consumed, "键仍转发给应用");
        assert_eq!(r.commit.as_deref(), Some("n"), "组字中无关键上屏拼音原文,不丢输入");
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
    let mut s = Session::new(true);
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
    let mut s = Session::new(true);
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
#[test]
fn cn_punct_commit() {
    use xkbcommon::xkb::keysyms as K;
    let e = fixture();
    let mut s = Session::new(true);
    // 空闲打 `.` → 中文句号
    let r = s.on_keysym(&e, K::KEY_period);
    assert_eq!(r.commit.as_deref(), Some("。"));
    assert!(r.consumed && !r.preedit_dirty);
    // 组字中打 `.` → 当前页首选 + 句号
    s.on_keysym(&e, 'n' as u32);
    s.on_keysym(&e, 'i' as u32);
    let r = s.on_keysym(&e, K::KEY_period);
    assert_eq!(r.commit.as_deref(), Some("你。"));
    assert!(r.consumed && r.preedit_dirty && !s.composing());
}

#[test]
fn punct_english_mode_passthrough() {
    use xkbcommon::xkb::keysyms as K;
    let e = fixture();
    let mut s = Session::new(true);
    s.toggle_punct(); // 切英文标点
    // 英文模式空闲打 `.`:不在映射生效,转发给应用(不消费)
    let r = s.on_keysym(&e, K::KEY_period);
    assert!(!r.consumed && r.commit.is_none());
    // 切回中文又生效
    s.toggle_punct();
    let r = s.on_keysym(&e, K::KEY_comma);
    assert_eq!(r.commit.as_deref(), Some(","));
}
#[test]
fn quote_pairs_alternate() {
    use xkbcommon::xkb::keysyms as K;
    let e = fixture();
    let mut s = Session::new(true);
    // 双引号开闭交替
    assert_eq!(s.on_keysym(&e, K::KEY_quotedbl).commit.as_deref(), Some("\u{201C}"));
    assert_eq!(s.on_keysym(&e, K::KEY_quotedbl).commit.as_deref(), Some("\u{201D}"));
    assert_eq!(s.on_keysym(&e, K::KEY_quotedbl).commit.as_deref(), Some("\u{201C}"));
    // 单引号独立配对
    assert_eq!(s.on_keysym(&e, K::KEY_apostrophe).commit.as_deref(), Some("\u{2018}"));
    assert_eq!(s.on_keysym(&e, K::KEY_apostrophe).commit.as_deref(), Some("\u{2019}"));
}
#[test]
fn modifier_keys_do_not_disturb_composing() {
    use xkbcommon::xkb::keysyms as K;
    let e = fixture();
    let mut s = Session::new(true);
    s.on_keysym(&e, 'n' as u32);
    s.on_keysym(&e, 'i' as u32);
    // 组字中按 Shift(欲打引号的 Shift+'):不上屏、不丢拼音,键转发
    let r = s.on_keysym(&e, K::KEY_Shift_L);
    assert!(!r.consumed && r.commit.is_none());
    assert!(s.composing() && s.buffer == "ni", "Shift 不应打断组字: buffer={}", s.buffer);
}
