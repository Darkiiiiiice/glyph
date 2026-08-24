//! 拼音会话状态机:按键 → 拼音缓冲 → 候选 → 上屏决策。
//! 纯逻辑、不碰 Wayland,便于单元测试。

use glyph_engine::{Candidate, Engine};

/// 一次按键的处理结果。
#[derive(Debug, Default, PartialEq)]
pub struct Reply {
    /// IME 是否消费该键;false 表示应经 virtual-keyboard 转发回 compositor。
    pub consumed: bool,
    /// 需要上屏(commit_string)的文本。
    pub commit: Option<String>,
    /// preedit 内容已变,需要重发 set_preedit_string。
    pub preedit_dirty: bool,
}

/// 一页候选数(数字键 1-9 直选)。
const PAGE: usize = 9;
/// 候选池大小(convert 的 limit):翻页的数据源,支持 POOL/PAGE 页。
const POOL: usize = 90;

pub struct Session {
    /// 当前拼音字母串,如 "nihao";空串 = 未在组字。
    pub buffer: String,
    pub candidates: Vec<Candidate>,
    /// 当前页码(0-based),输入变化/上屏时重置。
    page: usize,
    /// 中文标点模式:标点键上屏全角标点;`Ctrl+.` 切换中/英。
    punct_cn: bool,
}

impl Session {
    pub fn new(punct_cn: bool) -> Self {
        Self { buffer: String::new(), candidates: Vec::new(), page: 0, punct_cn }
    }
    /// 切换中/英文标点模式,返回新模式(true=中文)。
    pub fn toggle_punct(&mut self) -> bool {
        self.punct_cn = !self.punct_cn;
        self.punct_cn
    }

    pub fn composing(&self) -> bool {
        !self.buffer.is_empty()
    }

    /// keysym 路由。sym 为 xkb keysym(已含 shift 等修饰后的结果)。
    pub fn on_keysym(&mut self, engine: &Engine, sym: u32) -> Reply {
        use xkbcommon::xkb::keysyms as K;
        match sym {
            s if (K::KEY_a..=K::KEY_z).contains(&s) => {
                self.buffer.push(char::from_u32(s).unwrap());
                self.refresh(engine);
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_1..=K::KEY_9 if self.composing() => {
                let idx = (sym - K::KEY_1) as usize;
                match self.candidates.get(self.page * PAGE + idx) {
                    Some(c) => {
                        let text = c.text.clone();
                        self.clear();
                        Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                    }
                    None => Reply { consumed: true, ..Default::default() },
                }
            }
            K::KEY_space if self.composing() => match self.candidates.get(self.page * PAGE) {
                Some(c) => {
                    let text = c.text.clone();
                    self.clear();
                    Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                }
                None => {
                    // 无候选:上屏原文,避免吞键
                    let text = self.buffer.clone();
                    self.clear();
                    Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                }
            },
            K::KEY_BackSpace if self.composing() => {
                self.buffer.pop();
                self.refresh(engine);
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            // 翻页:`-` 上一页、`=` 下一页(避开 `,` `.`,留给中文标点)。
            // 拼音不变,仅 preedit_dirty 触发候选窗重绘当前页。
            K::KEY_minus if self.composing() && self.page > 0 => {
                self.page -= 1;
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_equal if self.composing() && (self.page + 1) * PAGE < self.candidates.len() => {
                self.page += 1;
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_Return if self.composing() => {
                let text = self.buffer.clone();
                self.clear();
                Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
            }
            K::KEY_Escape if self.composing() => {
                self.clear();
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            // 中文标点:组字中 = 上屏当前页首选+标点;空闲 = 直接上屏标点。
            // 无候选时组字中标点上屏拼音原文+标点(不吞拼音)。
            _ if self.punct_cn && cn_punct(sym).is_some() => {
                let p = cn_punct(sym).unwrap();
                if self.composing() {
                    let first = self
                        .candidates
                        .get(self.page * PAGE)
                        .map(|c| c.text.clone())
                        .unwrap_or_else(|| self.buffer.clone());
                    self.clear();
                    Reply { consumed: true, commit: Some(first + p), preedit_dirty: true, ..Default::default() }
                } else {
                    Reply { consumed: true, commit: Some(p.to_string()), ..Default::default() }
                }
            }
            // 其余键:若正在组字则丢弃拼音(简化决策),键本身转发
            _ => {
                if self.composing() {
                    self.clear();
                    Reply { consumed: false, preedit_dirty: true, ..Default::default() }
                } else {
                    Reply::default()
                }
            }
        }
    }

    /// 渲染 preedit 文本:仅拼音。候选由 M2 独立候选窗(popup)显示,
    /// 不再内联进 preedit——否则会出现横向 preedit 候选 + 竖向候选窗两套。
    pub fn render_preedit(&self) -> String {
        self.buffer.clone()
    }

    fn refresh(&mut self, engine: &Engine) {
        self.candidates =
            if self.buffer.is_empty() { Vec::new() } else { engine.convert(&self.buffer, POOL) };
        self.page = 0;
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.candidates.clear();
        self.page = 0;
    }
    /// 当前页候选(候选窗渲染的数据源)。
    pub fn page_candidates(&self) -> &[Candidate] {
        let start = (self.page * PAGE).min(self.candidates.len());
        &self.candidates[start..(start + PAGE).min(self.candidates.len())]
    }
}
/// 中文标点映射(中文标点模式下,无修饰键的标点键 → 全角标点)。
/// 顿号 `、` 用反斜杠 `\`(中文输入惯例)。引号智能配对复杂,暂不在此列。
fn cn_punct(sym: u32) -> Option<&'static str> {
    use xkbcommon::xkb::keysyms as K;
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

#[cfg(test)]
mod tests;
