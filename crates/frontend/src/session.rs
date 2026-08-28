//! 拼音会话状态机:按键 → 拼音缓冲 → 候选 → 上屏决策。
//! 纯逻辑、不碰 Wayland,便于单元测试。

use glyph_engine::{Candidate, Engine};

mod coin;
mod ctx;
mod punct;
use coin::Coining;
use ctx::Ctx;
use punct::{is_punct_key, is_modifier};

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

pub struct Session {
    /// 当前拼音字母串,如 "nihao";空串 = 未在组字。
    pub buffer: String,
    pub candidates: Vec<Candidate>,
    /// 当前页码(0-based),输入变化/上屏时重置。
    page: usize,
    /// 一页候选数(数字键 1-9 直选范围内)。
    page_size: usize,
    /// 候选池大小(convert 的 limit):翻页数据源,固定为 page_size 的 10 倍。
    pool: usize,
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
    /// Tab 单字模式:true = 候选窗显示首音节单字(逐字定字);false = 整句候选。
    char_mode: bool,
    /// 上屏历史(最近两个尾词,bigram/trigram 上文);跨组字保留,clear 不清。
    ctx: Ctx,
    /// 逐字造词链(见 coin.rs):char_mode 连续单字上屏,选完结算成用户词。
    coin: Coining,
}

impl Session {
    pub fn new(punct_cn: bool, page_size: usize) -> Self {
        let page_size = page_size.clamp(1, 20);
        Self { buffer: String::new(), candidates: Vec::new(), page: 0, page_size, pool: page_size * 10, punct_cn, dquote_open: false, squote_open: false, english: false, shift_down: false, shift_used: false, char_mode: false, ctx: Ctx::default(), coin: Coining::default() }
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
    pub fn on_keysym(&mut self, engine: &mut Engine, sym: u32) -> Reply {
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
                self.char_mode = false; // 单字模式下按字母:退出单字、字母正常入缓冲
                self.coin.clear(); // 退出单字:断造词链
                self.buffer.push(char::from_u32(s).unwrap());
                self.refresh(engine);
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_1..=K::KEY_9 if self.composing() => {
                let idx = (sym - K::KEY_1) as usize;
                match self.candidates.get(self.page * self.page_size + idx) {
                    Some(c) => {
                        let (text, consumed) = (c.text.clone(), c.consumed);
                        self.pick(engine, text, consumed)
                    }
                    None => Reply { consumed: true, ..Default::default() },
                }
            }
            K::KEY_space if self.composing() => match self.candidates.get(self.page * self.page_size) {
                Some(c) => {
                    let (text, consumed) = (c.text.clone(), c.consumed);
                    self.pick(engine, text, consumed)
                }
                None => {
                    // 无候选:上屏原文,避免吞键
                    let text = self.buffer.clone();
                    self.clear();
                    Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
                }
            },
            // Tab 切单字模式:候选窗在整句与首音节单字间切换(逐字定字)。
            K::KEY_Tab if self.composing() => {
                self.char_mode = !self.char_mode;
                if !self.char_mode {
                    self.coin.clear(); // 退出单字:断造词链
                }
                self.refresh(engine);
                Reply { consumed: true, preedit_dirty: true, ..Default::default() }
            }
            K::KEY_BackSpace if self.composing() => {
                self.coin.clear(); // 编辑拼音:纠错场景,断造词链
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
                let moved = (self.page + 1) * self.page_size < self.candidates.len();
                if moved {
                    self.page += 1;
                }
                Reply { consumed: true, preedit_dirty: moved, ..Default::default() }
            }
            // 以词定字:`[` 上屏当前页首选首字、`]` 尾字(词认得、只要其一字,免进单字模式)。
            // 守卫排除 char_mode(单字模式候选本就是单字)与无候选/空闲(落空走默认分支:
            // 有拼音上屏原文、空闲把字面括号键转发给应用)。
            K::KEY_bracketleft if self.composing() && !self.char_mode && !self.candidates.is_empty() => {
                self.pick_word_char(engine, false)
            }
            K::KEY_bracketright if self.composing() && !self.char_mode && !self.candidates.is_empty() => {
                self.pick_word_char(engine, true)
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
                self.commit_punct(engine, p)
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
        self.candidates = if self.buffer.is_empty() {
            Vec::new()
        } else if self.char_mode {
            engine.first_syllable_chars(&self.buffer, self.pool)
        } else {
            engine.convert_ctx(&self.buffer, self.pool, &self.ctx.words())
        };
        self.page = 0;
    }

    fn clear(&mut self) {
        self.buffer.clear();
        self.candidates.clear();
        self.page = 0;
        self.char_mode = false;
        self.coin.clear();
    }

    /// 选中候选:上屏 text;若候选只消耗前缀拼音(首词/逐字选择),截掉已消耗
    /// 部分、剩余拼音重新转换继续组字;否则(整句/无剩余)清空。
    fn pick(&mut self, engine: &mut Engine, text: String, consumed: usize) -> Reply {
        // bigram/trigram 搭配学习 + 上屏历史滑动;在 candidates 变化(clear/refresh)前取选中候选的分词。
        let words = self.candidates.iter().find(|c| c.text == text && c.consumed == consumed).map(|c| c.words.clone());
        if let Some(words) = &words {
            self.ctx.learn_commit(engine, words);
        }
        // 单字模式部分上屏(有剩余拼音)时保持单字模式,连续逐字选下一字;选完走 clear 退出。
        let total = self.buffer.bytes().filter(|&b| b != b'\'').count();
        // 逐字造词链:char_mode 的单字 pick 累积(音节=本次消耗的拼音),其余 pick 断链。
        let is_char_pick = self.char_mode && text.chars().count() == 1;
        if is_char_pick {
            let syll: String = self.buffer.chars().filter(|&c| c != '\'').take(consumed).collect();
            self.coin.push(syll, text.clone());
        } else {
            self.coin.clear();
        }
        if consumed >= total {
            self.coin.finish(engine); // 结算在 clear 之前(clear 会清链)
            self.clear();
        } else {
            // 部分上屏:截掉已消耗拼音(跳过穿插的 '),剩余重新转换继续组字。
            let cut = {
                let bytes = self.buffer.as_bytes();
                let mut n = consumed;
                let mut i = 0;
                while n > 0 && i < bytes.len() {
                    if bytes[i] != b'\'' {
                        n -= 1;
                    }
                    i += 1;
                }
                i
            };
            self.buffer.drain(..cut);
            self.refresh(engine);
        }
        Reply { consumed: true, commit: Some(text), preedit_dirty: true, ..Default::default() }
    }
    /// 以词定字:上屏当前页首选的首字(last=false)或尾字(last=true)。
    /// 整句候选必全长消耗,故定字即选完;造词链经 pick 的非逐字分支自动断开。
    fn pick_word_char(&mut self, engine: &mut Engine, last: bool) -> Reply {
        let Some(c) = self.candidates.get(self.page * self.page_size) else {
            return Reply::default();
        };
        let Some(ch) = (if last { c.text.chars().next_back() } else { c.text.chars().next() }) else {
            return Reply::default();
        };
        let text = ch.to_string();
        let consumed = c.consumed;
        let r = self.pick(engine, text.clone(), consumed);
        // pick 按 (text,consumed) 找整词候选学搭配;定字上屏的单字找不到匹配,
        // 手动把上屏字推入上屏历史,供下一句的 bigram/trigram 上文。
        self.ctx.push(text);
        r
    }
    /// 当前页候选(候选窗渲染的数据源)。
    pub fn page_candidates(&self) -> &[Candidate] {
        let start = (self.page * self.page_size).min(self.candidates.len());
        &self.candidates[start..(start + self.page_size).min(self.candidates.len())]
    }
}

#[cfg(test)]
mod tests;
