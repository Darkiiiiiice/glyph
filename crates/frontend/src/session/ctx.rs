//! 上屏历史窗口(最近两个尾词):bigram 用上一词,trigram 用上上文+上一词。
//! 跨组字保留,clear 不清——上下文是跨句的。

use glyph_engine::Engine;

/// 最近优先的上屏历史:`prev1` 上一次上屏尾词,`prev2` 上上次。
#[derive(Default)]
pub(super) struct Ctx {
    prev1: Option<String>,
    prev2: Option<String>,
}

impl Ctx {
    pub(super) fn prev1(&self) -> Option<&str> {
        self.prev1.as_deref()
    }

    pub(super) fn prev2(&self) -> Option<&str> {
        self.prev2.as_deref()
    }

    /// 一次上屏的搭配学习与窗口滑动:`words` 是选中候选的分词路径,
    /// 首词学 bigram(上一词→首词)与 trigram(上上文+上一词→首词),尾词进历史。
    pub(super) fn learn_commit(&mut self, engine: &mut Engine, words: &[String]) {
        if let Some(first) = words.first() {
            if let Some(p1) = self.prev1() {
                engine.learn_bigram(p1, first);
                if let Some(p2) = self.prev2() {
                    engine.learn_trigram(p2, p1, first);
                }
            }
        }
        if let Some(last) = words.last() {
            self.push(last.clone());
        }
    }

    /// 尾词入窗,最旧的滑出(窗口恒为最近两个)。
    pub(super) fn push(&mut self, word: String) {
        self.prev2 = self.prev1.take();
        self.prev1 = Some(word);
    }

    /// convert_ctx 入参:最近优先的上文切片(ctx[0]=上一词,ctx[1]=上上文)。
    pub(super) fn words(&self) -> Vec<&str> {
        [self.prev1(), self.prev2()].into_iter().flatten().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_keeps_last_two_recent_first() {
        let mut c = Ctx::default();
        assert!(c.words().is_empty());
        c.push("甲".into());
        assert_eq!(c.words(), ["甲"]);
        c.push("乙".into());
        assert_eq!(c.words(), ["乙", "甲"]);
        c.push("丙".into());
        assert_eq!(c.words(), ["丙", "乙"], "窗口只留最近两个,最旧的滑出");
    }
}
