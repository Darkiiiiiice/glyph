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

### M1.5 动态调频 + 用户词库 ✅(2026-08-24 完成)
实现:引擎 convert 叠加 `USER_W·ln(1+count)` 用户词频增量(USER_W=6,选 3 次 ≈ +8.3 对数分);`Engine::learn` 累计整词选词次数,落盘 `~/.local/share/glyph/user_freq.txt`(XDG),启动加载。segment::convert 内 DP 后、take 前叠加增量重排。

### M2 候选窗 ✅(2026-08-24 完成)
实现修正:用 `zwp_input_popup_surface_v2` 而非 layer-shell——compositor 自动把候选窗定位到文本光标(收 `text_input_rectangle` 事件),无需手算坐标。fontdue 光栅化 CJK(Noto Sans CJK)竖排候选列表+首候选高亮;shm buffer 经 memfd 承载(smithay mmap 只认 memfd 型 fd,M1 keymap 同款坑);rustix 纯 syscall 零 C 链接。activate 建 surface、打字重绘、上屏/deactivate 隐藏;preedit 只显拼音(候选交给候选窗,消除双候选框)。
翻页:`-`/`=` 前后翻页(避开 `,` `.` 留给中文标点),候选池 90(10 页),数字/空格选当前页页内第 k 个;输入变化/上屏重置页码。

**M2 关键发现(踩坑记录)**:
- **shm buffer 必须 memfd 型 fd**:smithay 的 mmap 路径只认 memfd——普通临时文件(/tmp,即使 tmpfs)在 `wl_shm.create_pool` 后报 `Failed to mmap fd N` 协议错误直接断连。与 M1 vkb keymap 同一个坑。解法:`rustix::fs::memfd_create`(纯 syscall linux_raw,零 C 链接),`File::from(OwnedFd)` 后 set_len+write 像素。
- **fcitx5 独占 im-v2 activate(M1 已记,M2 再踩)**:候选窗"完全不显示"的表象,根因常是 fcitx5 在后台独占了 activate——glyph 连键盘焦点都收不到。排查候选窗问题第一步先确认 fcitx5 已停。
- **`text_input_rectangle`(光标矩形)事件在 surface attach buffer 并 commit 后才由 compositor 发来**;popup surface 一创建就有,但光标定位要等首个 commit 后才生效。
- **双候选框**:M2 候选窗落地后 preedit 若仍内联候选列表,会出现横向 preedit 候选(应用渲染)+竖向候选窗(popup)两套并存。preedit 只显拼音,候选交给候选窗。

### M3 体验完善 ✅(2026-08-24 完成)
- **简拼**:多字词声母 key 精确匹配(你好→nh、中国→zhg),dict 预生成索引,convert 与全拼 DP 候选统一去重+排序(同 unigram 对数量纲)。纯声母简拼(不含混合拼);歧义靠动态调频补偿。实测 zhg→中国、bj→北京 首位。
- **中文标点**:`,。;:!?()、<>` 全角映射(顿号用反斜杠),组字中=上屏当前页首选+标点、空闲=直接上屏标点;`Ctrl+.` 切换中/英(打代码)。翻页 `-`/`=` 与标点 `,`/`.` 无冲突。
- **配置文件**:`~/.config/glyph/config.conf`(key=value 手写解析,零依赖),font_size/配色(bg/fg/pinyin_fg/hilite_bg)/punct_cn 默认;默认值=原硬编码,缺文件/坏行静默回退。

### M3.5 选词粒度:部分上屏 + 单字模式 ✅(2026-08-24 完成)
- **部分上屏**:数字选第 k 整词、`Shift+数字`选第 k 分词、`Tab` 单字模式选单字;选中部分上屏、剩余拼音继续选,逐词/逐字定长句。引擎 `Candidate.consumed` 标注覆盖字节数。
- **Tab 单字模式**:Tab 进/退,候选=第一音节单字(覆盖所有合法首音节切分,`xian→xi/xian`);长音节优先排序(打 `xuan` 定 xuan 的字,短切分 `xu` 的高频字"需/须"靠后);选字后保持单字模式**连续逐字**,整句选完/取消/Esc/Shift 切英文时回整句模式。
- 修复:部分上屏时候选窗不消失(keyboard commit 分支检查仍在组字则重发剩余拼音 preedit + 重绘候选窗)。

### 增强：bigram 上下文排序
**✅ 冷启动 bigram 2026-08-24 完成**(commit `4b44e06`)。无外部语料(rime-essay 是 LGPL-3.0,不符本项目 MIT-only 数据约束),搭配从用户输入历史积累:`user_bigram`(上文尾词→{当前词→次数});`convert_ctx` 重排叠加 `BIGRAM_W·ln(1+搭配次数)`(BIGRAM_W=6,与 USER_W 同量纲);Session 记 `prev_word`,pick 时 `learn_bigram`(上文尾词→本次首词)并更新上文;落盘 `user_bigram.txt`(与 user_freq 同目录,随 commit 写)。单测覆盖 engine 上浮+持久化、session prev_word 传递。
**验收**:反复"我们→学习"后,打"我们"再打 xuexi,"学习"提前(随使用积累生效)。

**调校结论**(2026-08-24,`BIGRAM_W=6` 不变)。真实词库 34,065 个同音词组的静态 gap 分布:p50=1.39(4x)、p90=4.20(67x)、p99=6.78(884x);W=6 下 1 次搭配翻 p90、3 次翻 ~4000x,响应足够。交替竞争 race(模拟+引擎契约测试双验证):窄 gap 对前 ~W/gap 轮呈"最近所选居首"(翻转限于相邻名次,两词始终前二可见,等价 recency 镜像),之后 Δboost=W·ln(1+1/n) 衰减到 gap 以下,静态先验正确弃权;宽 gap 对交替使用**永不翻盘**,须独占使用——上下文无区分度时 bigram 不干预;翻盘后恢复同样便宜:1 次反向选择即刻夺回(坍缩×3 翻盘后探索×1,Δboost=6·ln2=4.16<gap 6.78,静态锚主场优势),再翻需追到 6:1——误翻成本极低,进一步支持 W=6。否决替代方案:min-count 门槛(与 USER_W 一次即学的既定手感不一致,且双方 count 过门槛后 jitter 原样保留)、count²/total 概率归一(高频上文稀释死锁:`我们`积累 2000 条搭配后新窄 gap 习惯需 ~14 次才学会,比装饰性 jitter 更糟)。契约测试:`bigram_narrow_gap_flips_once_and_recovers`、`bigram_wide_gap_requires_dominant_usage`(segment.rs)。

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
