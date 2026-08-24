//! 中文标点映射与标点/修饰键判定(从 session 拆出,纯函数不碰 Session 状态)。

use xkbcommon::xkb::keysyms as K;

/// 中文标点映射(中文标点模式下,无修饰键的标点键 → 全角标点)。
/// 顿号 `、` 用反斜杠 `\`(中文输入惯例)。引号智能配对复杂,暂不在此列。
pub(super) fn cn_punct(sym: u32) -> Option<&'static str> {
    Some(match sym {
        K::KEY_comma => ",",
        K::KEY_period => "。",
        K::KEY_semicolon => ";",
        K::KEY_colon => ":",
        K::KEY_question => "?",
        K::KEY_exclam => "!",
        K::KEY_parenleft => "(",
        K::KEY_parenright => ")",
        K::KEY_backslash => "、",
        K::KEY_less => "<",
        K::KEY_greater => ">",
        _ => return None,
    })
}

/// 是否标点键(含引号)。无状态检查,供 match guard——punct_of 有状态(翻转引号),
/// 不能在 guard 里调,否则一次按键翻转两次。
pub(super) fn is_punct_key(sym: u32) -> bool {
    cn_punct(sym).is_some() || sym == K::KEY_quotedbl || sym == K::KEY_apostrophe
}

/// 是否修饰键的 press keysym(Shift/Ctrl/Alt/Super/Caps/Meta/Hyper 的 L/R,0xffe1-0xffee 段)。
/// 修饰键只改修饰状态,不应触发上屏或打断组字。
pub(super) fn is_modifier(sym: u32) -> bool {
    (K::KEY_Shift_L..=K::KEY_Hyper_R).contains(&sym)
}
