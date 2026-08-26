use super::*;
use xkbcommon::xkb::keysyms as K;

mod bigram;

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
