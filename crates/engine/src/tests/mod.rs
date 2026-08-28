use super::*;

    #[test]
    fn learn_accumulates_and_boosts_in_convert() {
        let mut e = Engine::from_str("ni 你 500\nhao 好 300\nni'hao 你好 10000\nni'hao 泥蒿 5\n");
        // 选"泥蒿"3 次 → convert 后它应顶到首位
        for _ in 0..3 {
            e.learn("泥蒿");
        }
        assert_eq!(e.user_freq().get("泥蒿"), Some(&3));
        assert_eq!(e.convert("nihao", 9)[0].text, "泥蒿");
    }

    #[test]
    fn user_freq_survives_roundtrip() {
        let mut e = Engine::from_str("ni 你 500\nhao 好 300\n");
        e.learn("你"); e.learn("你"); e.learn("好");
        let dir = std::env::temp_dir().join("glyph_test_user_freq");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("freq.txt");
        e.save_user_freq(&p).unwrap();
        let loaded = Engine::load_user_freq(&p).unwrap();
        assert_eq!(loaded.get("你"), Some(&2));
        assert_eq!(loaded.get("好"), Some(&1));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn load_user_freq_empty_file_is_ok() {
        let dir = std::env::temp_dir().join("glyph_test_user_freq");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("empty.txt");
        std::fs::write(&p, "").unwrap();
        assert!(Engine::load_user_freq(&p).unwrap().is_empty());
        std::fs::remove_file(&p).ok();
    }

    /// 真实词库集成测试(词库不在时跳过):验证 USER_W 在真实词频尺度下,
    /// 用户选低频同音词 3 次后它应顶到首位。
    #[test]
    fn real_lexicon_user_freq_promotes_picked_word() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/lexicon.txt");
        if !path.exists() {
            return; // 无真实词库时跳过
        }
        let mut e = Engine::load(&path).unwrap();
        let input = "shiji";
        let first = e.convert(input, 9)[0].text.clone();
        assert_eq!(first, "世纪", "静态最高频应首位");
        // 用户连续 3 次选"诗集"(最低频同音词)
        for _ in 0..3 {
            e.learn("诗集");
        }
        let top = &e.convert(input, 9)[0].text;
        assert_eq!(top, "诗集", "选 3 次应上浮到首位,实得 {top}");
    }

    #[test]
    fn bigram_boosts_matching_first_word() {
        let mut e = Engine::from_str("xue'xi 学习 100\nxue'xi 穴息 5000\nwo'men 我们 9000\n");
        // 无上文:词频高的"穴息"在前
        assert_eq!(e.convert_ctx("xuexi", 9, &[])[0].text, "穴息");
        // 记录搭配 我们→学习 5 次后,带上文"我们"时"学习"上浮到首位
        for _ in 0..5 {
            e.learn_bigram("我们", "学习");
        }
        assert_eq!(e.convert_ctx("xuexi", 9, &["我们"])[0].text, "学习");
        // 不同上文(无搭配记录)仍按纯词频
        assert_eq!(e.convert_ctx("xuexi", 9, &["他们"])[0].text, "穴息");
    }

    #[test]
    fn user_word_appears_in_convert() {
        let mut e = Engine::from_str("chi 魑 500\nmei 魅 500\nde 的 100000\n");
        // 高频填充词模拟真实 total 量级:整词 overlay 边 ln(100/T) 须压过单字拼合
        // 路径 2·ln(f/T),否则同文本去重留下拼合路径(words==["魑","魅"])。
        // 词库只有单字:拼合候选分词是 ["魑","魅"]
        assert!(!e.convert("chimei", 9).iter().any(|c| c.words == ["魑魅"]));
        assert!(e.add_user_word(&["chi", "mei"], "魑魅"));
        let c = e.convert("chimei", 9).into_iter().find(|c| c.words == ["魑魅"]);
        assert!(c.is_some(), "造词后整词候选应出现");
        assert!(e.convert("chi", 9).iter().all(|c| c.text != "魑魅"), "中间路径不挂词");
        // 幂等:同路径同文本不重复
        assert!(!e.add_user_word(&["chi", "mei"], "魑魅"));
    }

    #[test]
    fn user_word_skips_existing_lexicon_entry() {
        let mut e = Engine::from_str("ni'hao 你好 10000\n");
        assert!(!e.add_user_word(&["ni", "hao"], "你好"), "词库已有则不进 overlay");
        assert!(e.lexicon.user_words.to_lines().is_empty());
    }

    #[test]
    fn user_dict_survives_roundtrip() {
        let mut e = Engine::from_str("chi 魑 500\nmei 魅 500\nde 的 100000\n");
        e.add_user_word(&["chi", "mei"], "魑魅");
        let dir = std::env::temp_dir().join("glyph_test_user_dict");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("dict.txt");
        e.save_user_dict(&p).unwrap();
        let mut e2 = Engine::from_str("chi 魑 500\nmei 魅 500\nde 的 100000\n");
        assert_eq!(e2.load_user_dict(&p).unwrap(), 1);
        assert!(e2.convert("chimei", 9).iter().any(|c| c.words == ["魑魅"]));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn bigram_survives_roundtrip() {
        let mut e = Engine::from_str("wo'men 我们 9000\nxue'xi 学习 100\n");
        e.learn_bigram("我们", "学习");
        e.learn_bigram("我们", "学习");
        let dir = std::env::temp_dir().join("glyph_test_bigram");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("bigram.txt");
        e.save_bigram(&p).unwrap();
        let loaded = Engine::load_bigram(&p).unwrap();
        assert_eq!(loaded.get("我们").and_then(|m| m.get("学习")), Some(&2));
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn trigram_survives_roundtrip() {
        let mut e = Engine::from_str("wo'men 我们 9000\nai 爱 8000\nxue'xi 学习 100\n");
        e.learn_trigram("我们", "爱", "学习");
        e.learn_trigram("我们", "爱", "学习");
        let dir = std::env::temp_dir().join("glyph_test_trigram");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("trigram.txt");
        e.save_trigram(&p).unwrap();
        let loaded = Engine::load_trigram(&p).unwrap();
        assert_eq!(loaded.get("我们").and_then(|m| m.get("爱")).and_then(|m| m.get("学习")), Some(&2));
        std::fs::remove_file(&p).ok();
    }
