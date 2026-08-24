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
    /// 双/单引号配对状态:true = 下次出闭引号。跨句保留(引号配对是全局输入状态)。
    dquote_open: bool,
    squote_open: bool,
    /// 英文输入模式(单击 Shift 切换):所有键转发,应用直接收原始键,等同无输入法。
    pub english: bool,
    /// Shift 单击检测:press 置 down、期间搭配其他键置 used,release 时未 used = 单击切换。
    shift_down: bool,
    shift_used: bool,
}

impl Session {
    pub fn new(punct_cn: bool) -> Self {
        Self { buffer: String::new(), candidates: Vec::new(), page: 0, punct_cn, dquote_open: false, squote_open: false, english: false, shift_down: false, shift_used: false }
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
        // Shift 单击检测:press 标记;期间搭配其他键则不算单击(release 时判定,见 on_release)。
        if sym == K::KEY_Shift_L || sym == K::KEY_Shift_R {
            self.shift_down = true;
            self.shift_used = false;
        } else if self.shift_down {
            self.shift_used = true;
        }
        // 英文模式:所有键转发(应用直接收原始键,等同无输入法);Shift 上面已标记,
        // 用于单击切回中文。
        if self.english {
            return Reply::default();
        }
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
            // 组字中 `-`/`=` 始终消费:不能翻时(第一页/最后一页)忽略,否则守卫失败会落到
            // 标点/无关键分支误上屏、取消候选。页变了才 preedit_dirty 触发重绘。
            K::KEY_minus if self.composing() => {
                let moved = self.page > 0;
                if moved {
                    self.page -= 1;
                }
                Reply { consumed: true, preedit_dirty: moved, ..Default::default() }
            }
            K::KEY_equal if self.composing() => {
                let moved = (self.page + 1) * PAGE < self.candidates.len();
                if moved {
                    self.page += 1;
                }
                Reply { consumed: true, preedit_dirty: moved, ..Default::default() }
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
            // 中文标点(含引号配对):组字中 = 上屏当前页首选+标点;空闲 = 直接上屏标点。
            _ if self.punct_cn && is_punct_key(sym) => {
                let p = self.punct_of(sym).unwrap();
                self.commit_punct(p)
            }
            // 修饰键本身(Shift/Ctrl/Alt/Super 的 press):只改修饰状态,不影响组字、
            // 不上屏,直接转发。否则组字中按 Shift 欲打引号,会误触发下方的"上屏拼音原文"。
            s if is_modifier(s) => Reply::default(),
            // 其余键:组字中先上屏拼音原文(不丢已敲字母),键本身转发给应用。
            _ => {
                if self.composing() {
                    let text = self.buffer.clone();
                    self.clear();
                    Reply { consumed: false, commit: Some(text), preedit_dirty: true, ..Default::default() }
                } else {
                    Reply::default()
                }
            }
        }
    }

    /// 按键释放:检测 Shift 单击(press 后未搭配其他键)切换中/英文模式。
    /// 返回是否发生了模式切换(调用方据此刷新 preedit/候选窗)。
    pub fn on_release(&mut self, sym: u32) -> bool {
        use xkbcommon::xkb::keysyms as K;
        if (sym == K::KEY_Shift_L || sym == K::KEY_Shift_R) && self.shift_down {
            if !self.shift_used {
                self.english = !self.english;
                if self.english {
                    self.clear(); // 切入英文:丢弃未上屏的拼音缓冲
                }
                self.shift_down = false;
                return true;
            }
            self.shift_down = false;
        }
        false
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
    /// 上屏标点:组字中 = 当前页首选+标点(无候选则拼音原文+标点);空闲 = 直接标点。
    fn commit_punct(&mut self, p: &str) -> Reply {
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

    /// 标点符号(引号做开闭配对)。仅 punct_cn 模式调用;调用即翻转引号状态。
    fn punct_of(&mut self, sym: u32) -> Option<&'static str> {
        use xkbcommon::xkb::keysyms as K;
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
/// 是否标点键(含引号)。无状态检查,供 match guard——punct_of 有状态(翻转引号),
/// 不能在 guard 里调,否则一次按键翻转两次。
fn is_punct_key(sym: u32) -> bool {
    cn_punct(sym).is_some()
        || sym == xkbcommon::xkb::keysyms::KEY_quotedbl
        || sym == xkbcommon::xkb::keysyms::KEY_apostrophe
}
/// 是否修饰键的 press keysym(Shift/Ctrl/Alt/Super/Caps/Meta/Hyper 的 L/R,0xffe1-0xffee 段)。
/// 修饰键只改修饰状态,不应触发上屏或打断组字。
fn is_modifier(sym: u32) -> bool {
    use xkbcommon::xkb::keysyms as K;
    (K::KEY_Shift_L..=K::KEY_Hyper_R).contains(&sym)
}

#[cfg(test)]
mod tests;
