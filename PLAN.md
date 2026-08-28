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
- 数据源全部使用 MIT 许可的开源数据。**例外**(2026-08-24 用户决定):rime-ice/rime-frost 词库为 GPL-3.0,仅作本地可选数据源(data/raw/ 已 gitignore,永不入库/分发);若将来分发 lexicon.txt 需重新评估此例外。

## 技术决策

| 决策 | 结论 |
|---|---|
| 平台 | niri（已实现 zwp_input_method_v2 + zwp_virtual_keyboard_v1 + text-input-v3，外部输入法路线可行） |
| 架构 | `engine/`（trie 拼音前缀树 + DP 切分 + 排序）+ `frontend/`（Wayland 协议对接） |
| 协议 | im-v2 收键盘焦点与 preedit；virtual-keyboard-v1 注入上屏；layer-shell 候选窗 |
| 渲染 | wl_shm 软件渲染 + fontdue 绘字 |
| 引擎参照 | librime 源码（拼写运算、DAG 最短路切分、n-gram 排序）——理解后用 Rust 重新表达 |
| 数据源 | 音节表 ~410（自整理，pinyin-data 交叉校验）；汉字→拼音 mozillazg/pinyin-data (MIT)；词表+词频 jieba 词典 (MIT)；**可选合并** rime-ice(iDvel 上游)+ rime-frost 词库(GPL-3.0,本地) |
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
- **简拼**:多字词声母 key 精确匹配(你好→nh、中国→zhg),dict 预生成索引,convert 与全拼 DP 候选统一去重+排序(同 unigram 对数量纲)。歧义靠动态调频补偿。实测 zhg→中国、bj→北京 首位。
- **混合拼**(2026-08-26 补,M3 原排除项):每槽位可为完整音节或声母(lij→[li][j]→理解/立即;zhang→[zhan][g]→战国)。实现:枚举"音节格音节|声母"槽位切分(声母含 zh/ch/sh 双字母与零声母 a/e/o;i/u/v 不作声母自然剪枝),声母 key 命中索引桶后按音节槽精确过滤(逻辑 luo'ji 声母同为 lj 但首槽≠li,滤掉);索引条目增存音节序列。纯声母是纯混合拼的全-声母特例,行为不变。
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

### 词库增强:合并 rime-ice + rime-frost ✅(2026-08-24)
glyph-build 增加 rime_merge 步骤(bin 目录化布局):rime dict.yaml 追加进 lexicon.txt,词库 343,582 → **1,392,747** 条(+105 万)。规则:已有 (词,拼音) 保留 jieba 词频;带权重源(ice/frost base、frost ext、ice 8105)按 ln 空间 p50/p90 分位锚定映射到现有词频带并 clamp;平权源常数(ext=30、tencent=5、生僻字=3);ice/tencent 98 万条不并(frost 已是其精选);frost word/chengyu 是双拼码表、corrections 容错注音,均不可用。校验:词全 CJK + 音节在既有音节表。
**效果**(41 个常见输入对比):榜首 41/41 不变;top-9 换血 8.4%,几乎全是"字符拼合垃圾→真词"(旧 kexue 尾部 可学/可血/可雪 换成 咳血 等)。机制:total_freq 膨胀 Δ=ln(T1/T0)=1.02,单词边全体 -1.02 对数分、双段拼合路径 -2.03,拼合垃圾系统性下沉;user_freq/bigram boost 相对增强 ~1 对数分。常数校准合理(ext=30 压过碎片但压不过既有真词)。成本:加载与内存见下节(池化已解决)。

### Trie 池化紧凑化 ✅(2026-08-26)
**成果**:daemon RSS **1.0GB → 0.16GB**(6.4x),加载 **3.2s → 1.4s**(排序记录法构建比逐行插 HashMap 树还快),19 探针候选与旧版逐字节一致,53 测试全过。
**结构**(dict.rs 查询 + dict/build.rs 构建):
- trie 边存 u16 音节编号(音节表仅 ~410 个),节点池/边池/词条池/文本 String arena 四个大分配;词文本不再一词一个堆块。
- 构建零树形结构:每行解析成 20B 定长记录进大 Vec,按音节路径稳定排序(同路径词保持文件序),一遍递归展平——子边先占位再递归保证连续,word_len 在递归子树**前**定格(否则子树词条漏进前缀节点,契约测试 words_stop_at_exact_path 锁定)。
- 简拼/混合拼索引路径化:桶里只存音节路径(Box→池区间),词文本/词频查询时沿路径走 trie 解析,省掉百万份重复字符串;dict 侧 `jianpin_bucket`/`words_at_path`/`syll_str` 三个接口,segment 按 id 比对槽位。
**根因教训(为什么必须动构建而非换分配器)**:旧版压实后 RSS 仍 ~640MB 而实测活数据仅 ~300MB——两百万个 40B 小堆块与构建垃圾交错造成**页稀疏**(每 4K 页都有活块,任何分配器都无法 unmap;glibc/mimalloc 实测同高,malloc_trim 只收堆顶无效)。诊断手法:python 统计节点/词条数估活数据 + /proc/pid/smaps 按区间聚合看 RSS 构成。解法:让构建期也只产生少数大分配,垃圾释放后堆顶连续,trim 自然生效。

### M4 毕业项目
❌ 不做(2026-08-28 用户决定)。原计划 niri im-v2/vkb 修复 + 上游 PR,项目至此收尾。

### 用户造词(逐字定字学成词) ✅(2026-08-28)
- **触发**:Tab 单字模式连续逐字上屏 ≥2 字且整句选完 → 学成新词;断链即作废
  (整词/标点上屏、BackSpace、Esc/回车/无关键、字母或 Tab 退出单字模式)——纠错/取消不学,防噪声词。
- **引擎**:池化 trie 不可运行期插入(节点子边是边池连续段,中间插=全池重排),用户词走 overlay
  (user_dict.rs,嵌套 map 小 trie,千级);convert 时词边与 trie 词边同桶进 DP,去重/排序/
  user_freq/bigram 全复用。静态 freq=USER_WORD_FREQ(100)只保证"能见到",排序由造词伴随的
  user_freq +1 接管。真实词库 total 量级下整词 overlay 边天然压过单字拼合路径赢同文本去重
  (fixture 需高频填充词模拟 total,否则拼合赢、断言不到 words==[整词])。
- **防膨胀**:词库已有同(音节路径,文本)词只走 user_freq 不进 overlay;同词再造幂等。
- **持久化**:user_dict.txt(lexicon 三列行格式),随 commit 全量写;daemon/CLI 启动加载。
- **边界**:overlay 只接全拼 DP,不接简拼/混合拼索引(造词场景第一遍本就是全拼逐字打的)。
- **测试**:引擎 3(候选出现/幂等/词库查重/roundtrip)+ session 5(造词/Esc/整词断链/退格断链/单字不造)。
  lib.rs 行数越线把 tests 拆到 src/tests/mod.rs;punct.rs 收编 commit_punct/punct_of(改定位为"标点处理")。

### 以词定字(`[`/`]` 取词首/尾字) ✅(2026-08-28)
- **行为**:整句模式组字中 `[` 上屏当前页首选首字、`]` 尾字;守卫排除 char_mode(单字模式候选
  本就是单字)与无候选/空闲——落空走既有默认分支(有拼音上屏原文、空闲转发字面键,应用快捷键不受影响)。
- **实现**:纯 session 层,pick_word_char 复用 pick();造词链经 pick 的非逐字分支自动断开(定字不算逐字)。
  上屏单字找不到 (text,consumed) 匹配的整词候选,pick 后手动把该字记为 prev_word(bigram 上文)。
- **事实**:整句 convert 候选全部全长消耗(consumed=输入长),部分消耗只存在于 char 模式
  (first_syllable_chars)——故定字即选完,无"剩余拼音续打"分支。commit_punct 注释里"首词部分消耗"
  是早期设计的残留说法。
- **测试**:word_char.rs 4 个(首/尾字+prev_word、无候选落空、空闲转发、char 模式落空);
  顺手把 tests/mod.rs(301 行越线)按主题拆出 char_mode.rs / paging.rs。

### 整句级学习(trigram 双词上文) ✅(2026-08-28)
- **行为**:上屏历史窗口加到最近两个尾词(session/ctx.rs);候选首词与 (上上文,上一词)
  有搭配记录时上浮 TRIGRAM_W·ln(1+count),TRIGRAM_W=10(1 次双词搭配翻 p99 gap:10·ln2≈6.93>6.78)。
- **max 语义(关键决策)**:bigram/trigram 增量取 max 不相加——同一次上屏同时写两条记录,
  相加是双记一份证据;max 保留各自独立积累中更强者(契约测试 trigram_and_bigram_take_max_not_sum 锁定)。
- **API**:`convert_ctx(ctx: &[&str])` 最近优先切片(原 Option 签名全仓迁移);持久化
  user_trigram.txt(`上上文 上文 当前词 次数` 行),load/save/set 均镜像 bigram 模式。
- **BIGRAM_W=6 与既有公式未动**(调校定案),bigram 契约测试原样通过;纯加一层。
- **测试**:引擎 4(翻 p99/双词缺一不可/max 语义/roundtrip)+ ctx 窗口 1 + session 端到端 1
  (gap 隔离:5.50∈(bigram 4.16, trigram 6.93),user_freq 的 learn 在 keyboard 层、session 测试不涉及)。
  实测:隔离 XDG 造 user_trigram.txt,`--ctx "我们 爱"` 让 shiji 首位 世纪→实际,次序颠倒不触发。

## 风险

- **niri 生态兼容**：im-v2 为较新协议，niri 实现可能有 bug（候选窗位置、虚拟键盘重置 XKB 布局组等已知 issue）；应对：直接改 niri 提 PR。
- **应用侧 text-input-v3 支持参差**：不同应用 preedit 表现不一致，需逐一验证。
- **数据质量**：jieba 词频分布与输入法场景不完全匹配，靠动态调频补偿。
- **渲染复杂度**：fontdue 中文候选窗（字形、排版、光标跟随）工作量易低估，可能挤压 M2。

## 待解决

- [x] X11 应用中文输入:fcitx5 X11-only 共存已落地(2026-08-27)。方案:`~/.config/fcitx5/config` 加
  `[Behavior/DisabledAddons]` 段 `0=waylandim`(fcitx5 ini 的 vector 是编号子键格式,扁平
  `DisabledAddons=waylandim` 和 `[Addons/waylandim] Enabled=False` 均无效);niri 环境只设
  `XMODIFIERS=@im=fcitx`,不设全局 GTK_IM_MODULE/QT_IM_MODULE。验证:fcitx5 日志无 waylandim、
  `xprop -root XIM_SERVERS` = @server=fcitx、glyph 独占 im-v2 activate。ibus/dbus 前端保留(X11 Electron 用)
- [x] ~~M4:niri im-v2/vkb 已知问题修复 + 上游 PR~~(2026-08-28 决定不做,项目收尾)

## 后续候选功能(2026-08-28 整理;用户造词、以词定字、整句级学习已落地,见上)

现状基线:全项目 3520 行 / 25 个 rs 文件,距 10k 行预算余量 ~6500。规模列为粗略行数估计。

### 一档:直击"词排不上来"痛点(与项目初衷最贴)

| 功能 | 说明 | 实现要点 | 规模 |
|---|---|---|---|
| 删词/降权 | Ctrl+数字 拉黑垃圾候选(rime 合并后 139 万词库有噪声;与造词构成一学一删闭环) | user_blacklist.txt,convert 输出前过滤 | ~80 行 |

### 二档:手感补全(主流输入法标配)

| 功能 | 说明 | 实现要点 | 规模 |
|---|---|---|---|
| 模糊音 | z/zh、n/l、an/ang 等价类 | segment 层音节展开 + config [fuzzy] 开关 | ~200 行 |
| Emoji 候选 | kaixin→😄、aixin→❤️ | 额外词表挂 trie(Unicode 数据许可兼容) | ~100 行 |
| 中英混输 | 原始拼音串作末位候选直接上屏,不用 Shift 切英文 | session 加一条候选 | ~80 行 |
| 自定义短语 | user_phrases.txt:邮箱/签名/常用句,高优先级 | 启动加载,convert 头部注入 | ~60 行 |

### 三档:小而美

- 日期时间:rq→2026-08-28、sj→14:30、xq→星期五(搜狗经典,~50 行)
- 计算器:123*45= 出结果候选(自写小 parser,~60 行)
- 笔画辅助码:Tab 后输笔画码 hspnz 过滤同音字(数据许可需先查,~200 行)
- 用户数据备份/导出:glyph-cli 子命令打包 user_freq/bigram/短语(~50 行)

### 明确不做(有否决理由)

- 云输入:联网请求违背"每行可完全理解",隐私差,不可复现。
- 双拼:技术决策定死全拼;要动音节表与 segment 切分假设,ROI 低。

### 推荐组合(若重启)

删词降权(与造词构成一学一删闭环) + 日期时间当甜点。
