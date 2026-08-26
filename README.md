# Glyph

用 Rust 从零开发的 Wayland 中文全拼输入法,运行在 [niri](https://github.com/YaLTeR/niri) 合成器上。引擎与前端全部自研。

**动机排序:学习技术 > 好用 > 代码可完全理解。**

- 深入 Wayland 协议(`zwp_input_method_v2` / `zwp_virtual_keyboard_v1` / text-input-v3)、IME 架构、中文输入法算法(拼写运算、DAG 切分、unigram/bigram 排序)
- 解决 fcitx5+Rime 的实际痛点:词关联弱、常打的词排不上来
- 反臃肿:全项目 ~2.9k 行、每个文件 <300 行,每一行都能讲清楚

## 当前状态

**M0–M3.5 + bigram 全部完成(2026-08-24)**,日常可用:

- 全拼 + **简拼**(`nh`→你好、`zhg`→中国)+ **混合拼**(`lij`→理解);多音字正确(`chongqing`→重庆)、`v` 代 `ü`、隔音符 `xi'an`
- **动态调频**:选过的词按 `USER_W·ln(1+次数)` 上浮,选 1 次 ≈ 64 倍词频差,3 次 ≈ 4000 倍
- **bigram 上下文排序**:记住"上文→当前词"搭配(`BIGRAM_W=6`),越用越顺
- **候选窗**:`zwp_input_popup_surface_v2` 自动跟随文本光标,fontdue 渲染,`-`/`=` 翻页(10 页候选池)
- **部分上屏**:数字选整词、`Shift+数字`选分词、`Tab` 单字模式逐字定长句
- 中文标点(`,。、;:` 等,`Ctrl+.` 切中英)、Shift 单击切换中英文
- 未消费按键(Ctrl/Alt/Super 组合、功能键)原样转发,系统快捷键不受影响
- 词库 **139 万词条**(jieba 34 万 + 可选合并 rime-ice/rime-frost,见下文数据源)
- 50/50 单元测试通过

## 架构

```
crates/
├── engine/        # 引擎(纯 std,零第三方依赖)
│   ├── syllable   #   音节格:字母串 → 合法音节切分(数据驱动音节表)
│   ├── dict       #   音节 trie 词库 + 简拼声母索引,词条按词频降序
│   ├── segment    #   字节位置 DP k-best 切分 + 用户调频/bigram 重排
│   └── bin/       #   glyph-cli(命令行试用) / glyph-build/(词库生成 + rime 合并)
└── frontend/      # Wayland 前端
    ├── protocol   #   im-v2/vkb 绑定(vendored XML + wayland-scanner 生成)
    ├── globals    #   registry 发现 + 全局状态
    ├── ime        #   im-v2 状态机:activate/grab/preedit/commit_string
    ├── keyboard   #   grab 事件:xkb 键码解析、消费或经 vkb 转发
    ├── session    #   拼音会话状态机(纯逻辑,可单测)+ 标点映射
    ├── popup      #   候选窗 surface(input-popup-v2,光标跟随)
    ├── render     #   wl_shm 软件渲染 + fontdue 绘字
    └── config     #   ~/.config/glyph/config.conf(字号/配色/标点默认)
```

数据流:niri → im-v2 `activate` → `grab_keyboard` → 键事件 → xkb 解析 → 拼音会话 → 候选窗重绘 + `set_preedit_string`(拼音)/ `commit_string`(上屏)。未消费键经 virtual-keyboard-v1 转发回 compositor。

## 构建与运行

前置:Rust 1.86+、niri、系统 libxkbcommon、中文字体(Noto Sans CJK / LXGW WenKai)。

```bash
# 1. 生成词库(首次;数据源需先放入 data/raw/,见 .gitignore)
cargo run --release --bin glyph-build   # → data/lexicon.txt
                                        # data/raw/ 下存在 rime-ice/rime-frost 时自动合并

# 2. 停掉独占 im-v2 的现有输入法
killall fcitx5                          # im-v2 的 activate 只发给一个输入法

# 3. 启动(建议 release:百万级词库 debug 加载太慢)
cargo build --release
WAYLAND_DISPLAY=wayland-1 ./target/release/glyph
```

在任意支持 text-input-v3 的应用(alacritty、Chrome 等)里打字即可。

命令行快速试用引擎(不碰 Wayland):

```bash
echo "nihao" | cargo run --release --bin glyph-cli   # → 1.你好 2.你号 …
echo "nh" | glyph-cli                                 # 简拼 → 1.你好 …
echo "xuan" | glyph-cli --chars                       # Tab 单字模式候选
echo "xuexi" | glyph-cli --ctx 我们                    # bigram 上文排序
```

## 配置与用户数据

| 文件 | 内容 |
|---|---|
| `~/.config/glyph/config.conf` | `font_size`、`bg`/`fg`/`pinyin_fg`/`hilite_bg`(RRGGBB)、`punct_cn`(标点默认模式);key=value,缺省回退内置默认 |
| `~/.local/share/glyph/user_freq.txt` | 动态调频:词 → 选中次数(随上屏即时落盘) |
| `~/.local/share/glyph/user_bigram.txt` | bigram 搭配:上文尾词 → {当前词 → 次数} |

## 数据源

| 数据 | 来源 | 许可 | 用途 |
|---|---|---|---|
| 汉字→拼音 | [mozillazg/pinyin-data](https://github.com/mozillazg/pinyin-data) `kMandarin_8105` | MIT | 单字读音 |
| 词语→拼音 | [mozillazg/phrase-pinyin-data](https://github.com/mozillazg/phrase-pinyin-data) | MIT | 词语级读音(多音字) |
| 词表+词频 | [fxsjy/jieba](https://github.com/fxsjy/jieba) `dict.txt` | MIT | 基础 34 万词条 |
| rime-ice / rime-frost 词库 | iDvel/rime-ice、gaboolic/rime-frost | **GPL-3.0** | 可选合并 → +105 万词条 |

**许可注意**:rime 词库为 GPL-3.0,仅作本地可选数据源(`data/raw/` 已 gitignore,不入库不分发)。带权重源按 ln 空间分位锚定映射到现有词频带,平权源给常数;已有词条保留 jieba 真实词频。若分发生成的 lexicon.txt 需重新评估。

引擎参照 librime 的设计思想(拼写运算、DAG 最短路切分、n-gram 排序),理解后用 Rust 重新表达。

## 路线图

- [x] **M0** 引擎内核:音节格 + trie + DP 切分 + unigram 排序(CLI)
- [x] **M1** Wayland 最小前端:im-v2 激活 → grab → preedit → commit_string 上屏
- [x] **M1.5** 动态调频 + 用户词库(USER_W=6,落盘)
- [x] **M2** 候选窗:input-popup-v2 + fontdue,光标跟随、`-`/`=` 翻页
- [x] **M3** 简拼、中文标点、配置文件
- [x] **M3.5** 部分上屏 + Tab 单字模式;**bigram** 上下文排序(BIGRAM_W=6)
- [ ] **M4** 毕业项目:给 niri 上游提 PR(若踩到 compositor 侧 bug)

后续优化候选:trie 内存紧凑化(全量 rime 词库下 daemon RSS ~1.3GB)、长按重复。

## 已知限制

- Wayland-only;**X11/XWayland 应用架构上管不到**(text-input-v3 到不了 XWayland 客户端),X11 输入由 fcitx5 XIM 共存兜底(只设 `XMODIFIERS=@im=fcitx`,勿全局设 GTK/QT_IM_MODULE)
- 与 fcitx5 等 Wayland 输入法互斥(im-v2 单激活),同时只能跑一个
- 无长按重复;只测过 niri(其他 smithay 系合成器理论上可行)
- 全量 rime 词库加载 ~3.5s、内存 ~1.3GB

## License

MIT(代码;数据源许可见上表)
