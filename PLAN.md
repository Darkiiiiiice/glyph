# Glyph 开发计划

> 用 Rust 从零开发运行在 niri 上的 Wayland 中文全拼输入法（引擎 + 前端全部自研）。
> 关联笔记：`/home/mariomang/mariomang/90_计划/归档/Plan_2026-08-22_Kickoff_Glyph.md`（OrbitOS 启动计划，已归档）、`/home/mariomang/mariomang/20_项目/Glyph/Glyph.md`（项目笔记）

## 动机

1. **学习技术**（首要）：从零实现完整输入法，深入 Wayland 协议、IME 架构、中文输入法算法（拼写运算、DAG 切分、n-gram 排序）。
2. **好用**：解决 fcitx5+Rime 的实际痛点——词关联弱、常打的词排不上来。
3. **代码可完全理解**：反臃肿，每一行都能讲清楚。

## 工程约束

- 全 Rust，零 C 依赖，不碰 GTK/Qt。
- 全项目 **<10k 行**，每文件 **<300 行**。
- Wayland-only，不做 X11 回退。
- 数据源全部使用 MIT 许可的开源数据。

## 技术决策

| 决策 | 结论 |
|---|---|
| 平台 | niri（已实现 zwp_input_method_v2 + zwp_virtual_keyboard_v1 + text-input-v3，外部输入法路线可行） |
| 架构 | `engine/`（trie 拼音前缀树 + DP 切分 + 排序）+ `frontend/`（Wayland 协议对接） |
| 协议 | im-v2 收键盘焦点与 preedit；virtual-keyboard-v1 注入上屏；layer-shell 候选窗 |
| 渲染 | wl_shm 软件渲染 + fontdue 绘字 |
| 引擎参照 | librime 源码（拼写运算、DAG 最短路切分、n-gram 排序）——理解后用 Rust 重新表达 |
| 数据源 | 音节表 ~410（自整理，pinyin-data 交叉校验）；汉字→拼音 mozillazg/pinyin-data (MIT)；词表+词频 jieba 词典 (MIT)；M0 用 5 万常用词 |
| 终端 TUI | 延后，engine 预留复用 |

## 里程碑

### M0 引擎内核（纯 CLI）
音节表 + trie 词库 + DP 切分 + unigram 排序。
**验收**：`echo "nihao" | glyph-cli` 输出 `1.你好 2.你号 3.泥蒿`。

### M1 Wayland 最小前端 ✅(2026-08-22 完成)
im-v2 连 niri → 键盘 grab → preedit 发进应用 → **im-v2 commit_string 上屏**(修正:上屏不需要 vkb,im-v2 自带 commit_string;vkb 的正确角色是"未消费按键转发回 compositor"的通道)。
**验收**(alacritty 实测通过):真实键盘 `nihao`+数字 → `你好`,`woaini`+空格 → `我爱你`,`zhenhaochi` → `真好吃`,`chongqing` → `重庆`(多音字正确)。引擎候选渲染、数字/空格选词、Ctrl/Super 转发均验证。

**M1 关键发现(踩坑记录)**:
- im-v2/vkb 协议绑定不在 wayland-protocols crate 里,需 vendor XML + wayland-scanner 生成(im-v2 XML 来自 wlroots 仓库);text-input-v3 用 crate 现成的 `wp::text_input::zv3`。
- **fcitx5 独占冲突**:niri 的 im-v2 activate 只发给一个 input-method 对象;fcitx5 在跑时 glyph 收不到 activate。测试/使用 glyph 前须停 fcitx5。
- **smithay 收 vkb keymap 只认 memfd 型 fd**:普通临时文件静默失败(后续 key/modifiers 报 no_keymap 协议错误);正确做法是把 compositor 给的 keymap fd `try_clone` 直转,零复制零损坏面。
- **niri/smithay 下发的 keymap fd 文件偏移在末尾**:顺序读得 0 字节,必须 mmap(new_from_fd)或 read_at。
- vkb 注入的键被 smithay 发给焦点应用的 wl_keyboard、**绕过 im grab**——vkb 注入器(glyph-type)测不了 IME 路径,已删除;真实键盘才走 grab。
- grab 期间 compositor 不再处理键盘,IME 必须把未消费键经 vkb 转发回去,否则 niri 全局快捷键全灭。

### M1.5 动态调频 + 用户词库
激进动态调频：每次选择权重 +1，选 3 次上浮，词频持久化落盘；用户词记忆。
**提前理由**：这是核心痛点（"常打的词沉底"），且调频改动排序层，必须在候选窗（M2）之前落地避免返工。
**验收**：常打的词 3 次选择后上浮到候选顶部；重启后词频保留。

### M2 候选窗
layer-shell + fontdue 渲染 + 光标跟随 + 数字选词/翻页。
**验收**：候选窗跟随光标、数字选词、翻页可用。

### M3 体验完善
简拼 + 中文标点 + 配置文件。
**验收**：简拼可用；中文标点符合习惯。

### 增强：bigram 上下文排序
unigram 只看词频不看上文，解决不了"打'我们'后'学习'应提前"。M3 之后的首个增强（需语料或输入历史数据支撑）。
**验收**：输入"我们"后"学习"等高频搭配提前。

### M4 毕业项目
niri 侧协议修复 + 上游 PR。输入法 + 合成器两端全掌握，是本项目独有的学习场景。

## 风险

- **niri 生态兼容**：im-v2 为较新协议，niri 实现可能有 bug（候选窗位置、虚拟键盘重置 XKB 布局组等已知 issue）；应对：直接改 niri 提 PR。
- **应用侧 text-input-v3 支持参差**：不同应用 preedit 表现不一致，需逐一验证。
- **数据质量**：jieba 词频分布与输入法场景不完全匹配，靠动态调频补偿。
- **渲染复杂度**：fontdue 中文候选窗（字形、排版、光标跟随）工作量易低估，可能挤压 M2。

## 待解决

- [ ] fcitx5+Rime 具体失败场景清单（哪些词找不到、单字还是词组）→ 定为 M3 验收标准来源
- [ ] 候选窗视觉风格（极简灰底白字？主题化？）
