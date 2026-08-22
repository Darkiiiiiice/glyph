# Glyph

用 Rust 从零开发的 Wayland 中文全拼输入法，运行在 [niri](https://github.com/YaLTeR/niri) 合成器上。引擎与前端全部自研。

**动机排序：学习技术 > 好用 > 代码可完全理解。**

- 深入 Wayland 协议（`zwp_input_method_v2` / `zwp_virtual_keyboard_v1` / text-input-v3)、IME 架构、中文输入法算法（拼写运算、DAG 切分、unigram 排序）
- 解决 fcitx5+Rime 的实际痛点：词关联弱、常打的词排不上来
- 反臃肿：全项目 <10k 行、每个文件 <300 行，每一行都能讲清楚

## 当前状态

**M1 完成（2026-08-22)**——已经能在 niri 下正常打字：

- 全拼输入、候选列表（渲染在 preedit 里）、数字键/空格选词、退格编辑、Esc 取消
- 多音字正确（`chongqing` → 重庆）、`v` 代 `ü`(`nv` → 女)、`xi'an` 隔音符
- 未消费按键（Ctrl/Alt/Super 组合、功能键）原样转发，系统快捷键不受影响
- 17/17 单元测试

实测：`nihao` → 你好、`woaini` → 我爱你、`zhenhaochi` → 真好吃。

## 架构

```
crates/
├── engine/      # 引擎(纯 std,零第三方依赖)
│   ├── syllable #   音节格:字母串 → 合法音节切分(数据驱动音节表)
│   ├── dict     #   音节 trie 词库,词条按词频降序
│   ├── segment  #   字节位置 DP k-best 切分 + 文本去重
│   └── bin/     #   glyph-cli(命令行试用) / glyph-build(词库生成)
└── frontend/    # Wayland 前端
    ├── protocol #   im-v2/vkb 绑定(vendored XML + wayland-scanner 生成)
    ├── globals  #   registry 发现 + 全局状态
    ├── ime      #   im-v2 状态机:activate/grab/preedit/commit_string
    ├── keyboard #   grab 事件:xkb 键码解析、消费或经 vkb 转发
    └── session  #   拼音会话状态机(纯逻辑,可单测)
```

数据流：niri → im-v2 `activate` → `grab_keyboard` → 键事件 → xkb 解析 → 拼音会话 → `set_preedit_string`（拼音+候选）/ `commit_string`（上屏）。未消费键经 virtual-keyboard-v1 转发回 compositor。

## 构建与运行

前置：Rust 1.86+、niri、系统 libxkbcommon。

```bash
# 1. 生成词库(首次,需下载数据源,见 .gitignore 排除的 data/raw/)
cargo run --bin glyph-build          # → data/lexicon.txt(343k 词条)

# 2. 停掉独占 im-v2 的现有输入法
killall fcitx5                        # im-v2 的 activate 只发给一个输入法

# 3. 启动
cargo run --bin glyph                 # 前台;或 target/release/glyph 常驻
```

在任意支持 text-input-v3 的应用(alacritty、Chrome 等)里打字即可。

命令行快速试用引擎(不碰 Wayland):

```bash
echo "nihao" | cargo run --bin glyph-cli    # → 1.你好 2.你号 …
```

## 数据源

全部 MIT 许可：

| 数据 | 来源 | 用途 |
|---|---|---|
| 汉字→拼音 | [mozillazg/pinyin-data](https://github.com/mozillazg/pinyin-data) `kMandarin_8105` | 单字读音 |
| 词语→拼音 | [mozillazg/phrase-pinyin-data](https://github.com/mozillazg/phrase-pinyin-data) | 词语级读音(多音字) |
| 词表+词频 | [fxsjy/jieba](https://github.com/fxsjy/jieba) `dict.txt` | 343k 词条 |

引擎参照 librime 的设计思想（拼写运算、DAG 最短路切分、n-gram 排序），理解后用 Rust 重新表达。

## 路线图

- [x] **M0** 引擎内核：音节格 + trie + DP 切分 + unigram 排序(CLI)
- [x] **M1** Wayland 最小前端：im-v2 激活 → grab → preedit → commit_string 上屏
- [ ] **M1.5** 动态调频 + 用户词库：选择权重 +1、3 次上浮、词频落盘
- [ ] **M2** 候选窗：layer-shell + wl_shm + fontdue,光标跟随、翻页
- [ ] **M3** 体验完善：简拼、中文标点、配置文件;之后 bigram 增强
- [ ] **M4** 毕业项目：给 niri 上游提 PR(若踩到 compositor 侧 bug)

## 已知限制

- 候选渲染在 preedit 里（应用内联），独立候选窗是 M2
- 无翻页（一页 9 候选）、无长按重复、组字中按无关键丢弃缓冲
- Wayland-only，不做 X11 回退；目前只测过 niri
- 与 fcitx5 等输入法互斥（im-v2 单激活），同时只能跑一个

## License

MIT
