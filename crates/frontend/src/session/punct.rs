//! 中文标点处理:映射表、标点/修饰键判定、上屏决策(commit_punct/punct_of
//! 碰 Session 的引号配对状态,故以扩展 impl 放这里,与映射表同文件内聚)。

use xkbcommon::xkb::keysyms as K;

use glyph_engine::Engine;

use super::{Reply, Session};

impl Session {
    /// 上屏标点:组字中 = 当前页首选+标点(无候选则拼音原文+标点);空闲 = 直接标点。
    /// 首选为首词(部分消耗)时上屏首词+标点、剩余拼音继续组字。
    pub(super) fn commit_punct(&mut self, engine: &mut Engine, p: &str) -> Reply {
        if self.composing() {
            let (text, consumed) = self
                .candidates
                .get(self.page * self.page_size)
                .map(|c| (c.text.clone(), c.consumed))
                .unwrap_or_else(|| (self.buffer.clone(), usize::MAX));
            self.pick(engine, text + p, consumed)
        } else {
            Reply { consumed: true, commit: Some(p.to_string()), ..Default::default() }
        }
    }

    /// 标点符号(引号做开闭配对)。仅 punct_cn 模式调用;调用即翻转引号状态。
    pub(super) fn punct_of(&mut self, sym: u32) -> Option<&'static str> {
        Some(match sym {
            K::KEY_quotedbl => {
                let p = if self.dquote_open { "\u{201D}" } else { "\u{201C}" };
                self.dquote_open = !self.dquote_open;
                p
            }
            K::KEY_apostrophe => {
                let p = if self.squote_open { "\u{2019}" } else { "\u{2018}" };
                self.squote_open = !self.squote_open;
                p
            }
            _ => cn_punct(sym)?,
        })
    }
}

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
