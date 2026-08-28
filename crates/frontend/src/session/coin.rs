//! 逐字造词链：Tab 单字模式下连续单字上屏的（音节, 字）序列，
//! 整句选完时结算成一个用户词，写进引擎 overlay（详见 user_dict.rs）。
//!
//! 判定刻意保守——只有"连续不间断的逐字选择"才造词：
//! - 结算点唯一：最后一字上屏（consumed 覆盖全部拼音）且链长 ≥2；
//! - 断链即作废：整词/标点上屏、BackSpace、Esc/回车/无关键（走 clear）、
//!   字母键或 Tab 退出单字模式。纠错/取消场景不学，防噪声词入库。

use glyph_engine::Engine;

/// 造词链状态：已上屏的 (音节, 字) 序列。空 = 不在链中。
#[derive(Default)]
pub(super) struct Coining {
    seq: Vec<(String, String)>,
}

impl Coining {
    /// 追加一个逐字上屏的字与其消耗的拼音音节。
    pub(super) fn push(&mut self, syll: String, ch: String) {
        self.seq.push((syll, ch));
    }

    /// 断链作废（任何非逐字选择上屏 / 编辑拼音 / 退出单字模式）。
    pub(super) fn clear(&mut self) {
        self.seq.clear();
    }

    /// 整句选完结算：≥2 字学成词（引擎侧做词库查重与幂等）。返回是否造了词。
    pub(super) fn finish(&mut self, engine: &mut Engine) -> bool {
        let seq = std::mem::take(&mut self.seq);
        if seq.len() < 2 {
            return false;
        }
        let sylls: Vec<&str> = seq.iter().map(|(s, _)| s.as_str()).collect();
        let text: String = seq.iter().map(|(_, c)| c.as_str()).collect();
        engine.add_user_word(&sylls, &text)
    }
}
