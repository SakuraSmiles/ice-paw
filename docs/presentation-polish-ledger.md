# 表现层打磨台账

> **2026-08-20 建立376ff。单一真相源**：全量 UI 走查（chat 域亲审 + 三路并行审计：设计系统基底 / 设置域 / 项目与轨迹域）。
> 此后表现层新债直接进本表；每条清偿后更新状态，不删除。与 tech-debt-ledger 平行，批次前缀 **UI**。
>
> 图例：🔴 排期中 ｜ 📋 待办 ｜ 👁 观察池（默认不做，撞上再修）｜ 🗑 已划掉（含原因）
>
> 走查四维（与用户拍板的四目标对齐）：**视觉**（风格统一与升级）/ **手感**（交互效率）/ **状态**（完备性与极端场景）/ **平台**（桌面质感）。
>
> **分诊待用户确认**。观察池的「不做」须有据：与配置放置阶梯冲突、收益/成本错配、或真机未见此症。

---

## 批次 UI-A — 高危交互缺陷（手感，建议最先清）

| # | 项 | 位置 | 症状与建议 | 量级 |
|---|---|---|---|---|
| UI-A1 | **IME 组合期 Enter 误发送** | `ChatInput.vue` handleKeydown（全库 grep 无 isComposing/compositionstart 防护） | 中文拼音输入中按 Enter 确认候选词 → 消息被直接发送。对中文首位的产品是最高频手感伤害。建议：`if (e.isComposing || e.keyCode === 229) return;` 加在 Enter 分支前；@ 弹层的 Enter 选择分支同理。**修复需真机 IME 手测**（拼音/五笔/日文 IME 各一遍） | 极小（一行级） |
| UI-A2 | **生成中输入框整体禁用** | `ChatInput.vue` `:disabled="chat.sending"`（textarea + 附件/@ 按钮全禁） | 生成期间无法预写下一条消息（连续提问工作流被打断）。业界主流（ChatGPT/Claude/DeepSeek 官方）均允许生成中继续输入、仅禁发送。建议：textarea 解除禁用，send 按钮保持禁用/切停止态；draft 持久化机制已就位，零额外成本 | 小 |
| UI-A3 | **全局快捷键 DOM query 脆弱 + 覆盖薄** | `App.vue` handleGlobalKeydown | ① `(document.querySelector(".conv-item-new") as …)?.click()`——类名即契约，样式重构即断且无报错；② 仅 N/W/K 三键（P6 台账已有「快捷键文档」项，此处补实现面）：缺 Cmd+F 聚焦侧栏搜索、Esc 统一关弹层（图片预览/附件详情/@ 弹层各自为政）、Cmd+1..9 切会话。建议：shortcuts 收敛为一个 composable（发布定制的 `icepaw:new-chat` 事件或 store action，DOM 解耦），Esc 栈统一管理 | 中 |

## 批次 UI-B — 视觉一致性（视觉，令牌收敛战役）

| # | 项 | 位置 | 症状与建议 | 量级 |
|---|---|---|---|---|
| UI-B1 | **焦点环颜色魔法数** | `ChatInput.vue:636-638` 三处 `rgba(46,141,100,…)` | 品牌绿的 rgb 值以字面量散布（focus 环/发送态/拖拽态各一），改品牌色必漏。建议：tokens.css 增 `--ip-color-focus-ring`（rgb 三元组变量 `--ip-primary-500-rgb` + `rgba(var(…), α)` 模式），全库替换 | 小 |
| UI-B2 | **markdown.css / global.css 硬编码色集中区** | `assets/styles/markdown.css`（43 处 hex）、`global.css`（40 处） | 代码高亮 mini 主题与全局样式的色值未走令牌——暗色模式切换时这些值靠 `[data-theme]` 选择器手工配对，新增主题即爆炸。建议：hljs 主题色收进 tokens（语义组：`--hl-keyword/--hl-string/…`），global.css 的散值逐个对位既有令牌 | 中 |
| UI-B3 | **组件内残留硬编码色** | `ChatMessages.vue`（22 hex + 11 rgba）、`EntityAvatar.vue`（21 hex，渐变名字哈希色板可豁免）、`ProjectBasicForm.vue`（10）、`AgentSettings.vue`（8）、`ConfigProposalCard.vue`（8） | 建议：EntityAvatar 的哈希色板是算法资产可保留（但应移到 tokens 注释声明豁免理由）；其余逐个对位令牌。**验收 grep 门**：`grep -c '#[0-9a-f]{6}' 组件.vue == 0`（豁免清单外） | 中（分批） |
| UI-B4 | **成功态图标的 fallback 色重复** | `ChatMessages.vue:790,794,995…` `var(--ip-success-base, #16a34a)` 内联 fallback 多处重复 | fallback 值与令牌漂移风险。建议：确认令牌已定义后删除全部内联 fallback（fallback 只在令牌可能缺席的边界处用） | 小 |

## 批次 UI-C — 状态完备性（状态）

| # | 项 | 位置 | 症状与建议 | 量级 |
|---|---|---|---|---|
| UI-C1 | **剪贴板无失败兜底** | `ChatMessages.vue` copyContent：`navigator.clipboard.writeText(content)` 裸调用 | clipboard API 在非聚焦文档/权限异常时 reject → 静默失败，用户以为复制成功。建议：`.catch` 走 `document.execCommand('copy')` 降级或至少把 copy 按钮态改为失败色 | 小 |
| UI-C2 | **错误横幅无重试/关闭动作** | `ChatMessages.vue:674` chat-error-banner | 仅展示 lastError 文本，无「重试」入口也无关闭。建议：网络类错误附重试按钮（重发上一条），其余附关闭 ×。注意 L2 原则：这是状态上屏，不是配置 | 小 |
| UI-C3 | **骨架屏覆盖不均** | 已有：消息列表/侧栏会话（shimmer 质量好）；缺：轨迹页首载、项目详情 tab 首载、设置页部分列表 | 走查 fork 补充定位后汇总 | 待 fork |

## 批次 UI-D — 可访问性与键盘（平台/手感）

| # | 项 | 位置 | 症状与建议 | 量级 |
|---|---|---|---|---|
| UI-D1 | **组件层焦点可见性被抵消** | base.css:109 已有全局 `:focus-visible` 焦点环（FORK-A 证实），但 GroupedSelect `outline:none`（:253）等组件层主动抵消了它 | 建议：审计所有 `outline:none` 出现点（grep 门），逐个补回 `:focus-within`/`:focus-visible` 等效物；热点位（tabbar/侧栏/弹层）把 click-only 元素渐进改 button/tabindex | 中（起步小） |
| UI-D2 | **ChatPage tabbar 无 tablist 语义/方向键** | `ChatPage.vue:44-61` | 对话/轨迹切换是纯鼠标操作。建议：`role=tablist/tab` + 左右方向键 + aria-selected（aria-hidden 已做，说明有意识，补全即可） | 小 |
| UI-D3 | **点击目标非语义元素** | tool-toggle/think-toggle/用户引用卡等 div+@click | 屏幕阅读器不可达、不可 Tab。建议随 UI-D1 顺路渐进改善，不单独开批 | 👁 渐进 |

## 批次 UI-E — 桌面平台质感（平台）

| # | 项 | 位置 | 症状与建议 | 量级 |
|---|---|---|---|---|
| UI-E1 | **Cmd+Enter 备选发送** | `ChatInput.vue` | mac 用户肌肉记忆。一行级：Enter 分支并列 `|| (e.metaKey||e.ctrlKey) && e.key==='Enter'`（与 IME 防护同点处理） | 极小 |
| UI-E2 | **窗口标题随会话名联动** | `ChatHeader.vue` / Tauri window API | 桌面惯例：当前会话名进窗口标题（可选：会话名 — IcePaw）。capabilities 已含 core:window 相关权限面，核实 `core:window:allow-set-title` 后即可 | 小 |
| UI-E3 | **未聚焦窗的流式感知** | 观察池 | app 在后台生成时无任何 OS 级提示（badge/flash/dock）。Tauri 有 setBadgeCount/请求用户注意力 API。**先观察**：单机本地工具的场景是否真需要——撞上用户抱怨再做 | 👁 |
| UI-E4 | **图片预览无缩放/拖拽** | `ImagePreview.vue` | 大图（3.8MB 实测附件）只有原尺寸展示？走查 fork 确认现状后定 | 待 fork |

---

## 优势资产（走查确认，勿动）

1. **滚动工程**：跟随/暂停/阅读位置记忆/跳转钉子/漂移校正（useScrollFollow + useActiveTurn）——同类产品罕见的质量，表现层战役的地基已在
2. **`--msg-col-right` 单一真相源模式**（ChatPage 注释明确）：调轨道宽度只改一个值——把这种模式推广到其他跨组件对位（焦点环色、间距节奏）正是 UI-B 的方法论
3. **附件拒绝聚合反馈**（一条汇总警告 vs toast 轰炸）——符合「状态上屏」的克制美学
4. **@ 引用弹层**：键盘导航/高亮/已引用态/重复闪提示，交互完备度是全库标杆
5. **骨架屏 shimmer**（侧栏）质量好，可作 UI-C3 的模板
6. **暗色模式跟随 `[data-theme]`**：令牌双套已成型，问题只在未令牌化的散值（UI-B2/B3）

## 观察池（有据不做）

- **会话列表虚拟化**：有搜索 + 单机会话量级未达千级；撞上再修
- **全局 IME 深度定制**（如组合期间 @ 弹层抑制）：UI-A1 修复后剩余场景真机评估再议
- **多窗口/悬浮条**（前轮研讨第三层）：架构就绪但属新形态，不在本轮表现层战役内

---

## 走查方法与置信说明

- chat 域（20 组件 + ChatPage）：逐文件亲读（ChatPage/ChatInput/ChatMessages/Sidebar 全文，其余抽读）+ 横切 grep 定量（硬编码色/IME/aria/focus-visible）
- 设计系统基底 / 设置域 / 项目轨迹域：三路并行审计报告（见下节汇总）
- 全部条目以「能指认文件:行号」为准；手测项标注真机验证要求（CLAUDE.md：typecheck/lint/build 不覆盖视觉回归）

## 三路审计汇总（fork 结果填入区）

### FORK-B：设置域（settings/AgentForm/McpForm/KbDocumentList）✅ 2026-08-20

**优势确认（补进上方资产区）**：McpSettings 状态机闭环（starting 轮询/failed 重试/GLM 模板复用真实运行态）、AgentForm 测试连接一次往返两用、Embedding 切换全流程保护、MoreMenu 就地二次确认、三处空态引导一致、单一数据源纪律。

**🔴 高危（全部指向同一根因：设置域无统一错误反馈原语）**

| # | 位置 | 症状 | 建议 |
|---|---|---|---|
| S-1 | AgentSettings.vue:63-65 | 删除 Agent 失败仅 console.error，用户无感 | 行内错误提示 |
| S-2 | McpSettings.vue:96-99,114-117 | 切换启用/删除 server 失败静默 | toast/行内提示附原因 |
| S-3 | GeneralSettings.vue:41-45 | 页面加载失败整体静默（prefs 全空回退无标记） | 页级错误条（复用 LogSettings:149-155 模式） |
| S-4 | GeneralSettings.vue:224-228,269-273,435-439 | embedding/vision blur 保存、时区即存——**保存失败全静默** | 保存中/失败转行内红字 |
| S-5 | AgentSettings.vue:25-29 / McpSettings.vue:28-32 / KbDocumentList.vue:46-48 | 三列表加载失败仅 console.error → 永久 loading 或空列表假象 | 统一「加载失败+重试」态 |

**→ 结论（fork 建议采纳为批次 UI-F）：抽一个轻量 ErrorBanner/保存反馈组件，接通全部 ~8 处静默点。符合 L2「状态上屏」，零新增配置。量级：中（一个组件 + 8 处接线）。**

**🟡 中**：LogSettings:86 自动刷新抢滚动条（上翻查看历史被拉回，聊天页 351ae86 教训未复用）；McpSettings/KbDocumentList 无搜索过滤；LogSettings 无级别筛选；GeneralSettings 保存模式混用（显式保存 vs blur 自动保存，预期不一致）；AgentForm/McpForm ~200 行样式逐字重复 + GeneralSettings 第三套输入样式（30px vs 32px）——**设置域三套同义输入样式**并入 UI-B 战役。

**🟢 低**：表单无键盘提交（Enter/Cmd+Enter）；provider-badge 品牌色硬编码；GeneralSettings 文件头注释失真（1344 行实 vs 937 注，主题区已迁 Sidebar）；删除菜单未用 MoreMenu danger 属性；McpForm「信任」档 hint 无安全警示；视觉读取 tooltip 200+ 字塞 260px；KbDocumentList 重建失败无原因；保存后全量刷新致展开态闪烁；**KB 文档列表只读（只能进不能出）**——功能缺口非表现层，登记观察（或移交产品清单）。

### FORK-A：设计系统基底（tokens + common 组件）✅ 2026-08-20

**优势确认**：tokens.css 分区组织教科书级（引用设计文档章节号）；动效 16 档 + reduced-motion 分级降级；EntityAvatar 三级降级链 + 策展色板纪律；PanelResizeHandle 隐形热区；base.css 已有全局 `:focus-visible` 焦点环（:109——**注意：此发现修正 UI-D1 的部分范围，焦点环基座存在，问题是组件层 `outline:none` 抵消了它**）。

**🔴 高危**

| # | 位置 | 症状 | 建议 |
|---|---|---|---|
| D-1 | **global.css:14 vs tokens.css:34** | **双源品牌色分叉**：tokens.css 冰蓝 #4680C2（注释还是 #2563EB），global.css 后加载覆盖为绿 #44A87A；另有 19 组件 81 处引用平行令牌 `--color-*` 与 `--ip-*` 语义层重复定义 | 二选一收敛：品牌色进 tokens.css 单一真相源（global.css 只做主题 override 或删除），`--color-*` 全部迁移语义层。**量级：大——但这是整个 UI-B 战役的根，不先做它其他收敛都是局部止血** |
| D-2 | GroupedSelect.vue:253,233 | 键盘焦点完全不可见（outline:none 且无 :focus-within），违反 WCAG 2.4.7 | 补 `.gs-control:focus-within` 复用焦点环 | 
| D-3 | Combobox.vue:148-157 / GroupedSelect.vue:138-149 | 选择器无方向键导航、无 combobox ARIA 模式——键盘用户永远只能选第一项 | ArrowUp/Down 游标 + role=combobox/aria-expanded/aria-activedescendant |
| D-4 | Combobox.vue:249 / GroupedSelect.vue:236 | 焦点环硬编码 rgba 绿复制体；**体系内 `--ip-shadow-focus` 全仓库零引用（死令牌）** | 改用 `var(--ip-shadow-focus)`（暗色变体免费获得） |

**🟡 中**：控件高度硬编码 36/30/28px 绕过 `--ip-input-h-*`（且 `--ip-select-option-h` 死令牌）；5 处 `transition: all`（应 `--ip-transition-colors`）；MoreMenu 自造 keyframes 绕过 `ip-popover-in`；**MoreMenu 破坏性确认钮无 danger 语义且 danger hover 红色消失**（与 FORK-B 的「删除未标 danger」同根）；7 处直引原始色阶（缺 `--ip-color-bg-selected` 语义令牌）；字号 10px 低于体系最小 12px；Switch thumb #fff/阴影硬编码；下拉 max-height 各写各的。

**🟢 低**：PanelResizeHandle 无键盘路径（role=separator + 方向键）；Switch 无 ariaLabel 通道、无按压反馈（`--ip-duration-btn-press` 死令牌）；MoreMenu 无 aria-expanded/Escape/焦点移入；tokens.css 断点 sm≡md 疑似笔误；tokens.css 注释与实际色值不符（陈旧文档）；2 处 z-index 硬编码。

**令牌缺口汇总（D-1 的展开）**：①品牌色单一真相源缺失（最大）；②缺选中态语义层；③danger 缺背景/文字悬停层；④**5 个死令牌**（shadow-focus/select-option-h/dropdown-item-h/spacing-1_5/btn-press——恰好对应本次走查发现的硬编码点，证明"令牌先行、消费滞后"）；⑤输入控件尺寸未接入；⑥头像尺寸无令牌。

### FORK-C：项目与轨迹域（ProjectList/project/trajectory 组件）✅ 2026-08-20

**优势确认**：手写虚拟滚动（offsets 前缀和 + 二分 + beginPrepend 高度差补偿）是同类实现罕见的稳；canvas 瀑布图（DPR 感知/像素列聚合/锚定缩放/live 比例复原）；事件驱动 live 增量 + 双拉防护；keep-alive 缓存键纪律；跨会话适配层零侵入；状态面完备（loading/error/empty/搜索无命中/任务空态/成员无消息各就位）。

**🔴 高危（同一根因三处未同步：身份比较 vs key 比较）**

| # | 位置 | 症状 | 建议 |
|---|---|---|---|
| T-1 | ProjectTimeline.vue:118 | moveSelection 用身份比较定位选中行，rows 是 computed 每次重派生产新对象——live 追加/搜索后 findIndex=-1，**键盘 ↑↓ 直接跳首/尾，流式期间反复归零**。同文件 L80-83 selectRow 已修 key 比较并注释，moveSelection 漏改 | 改按 row.key（与 selectRow 同款） |
| T-2 | TrajectoryView.vue:155 | selectRow 仍身份比较——live 追加/搜索后**再点同一行无法取消选中**（检查器关不掉） | 对齐 key 比较 |
| T-3 | （合并 T-1/T-2）三个文件的身份比较一次对齐 | 修复未同步扩散的典型案例 | 一个 commit 全修 |

**🟡 中**：TrajectoryView rows computed 依赖 streamingRows → 流式期间每 chunk 全量重跑 buildRows（与 40ms 聚合目标冲突，基础行应独立 computed）；ProjectList 卡片纯 div 无键盘路径（轨迹域已把键盘当一等公民，标准不一致）；ProjectOverview watch+onActivated 双拉（首访重复请求）；delegationConvsOf 模板内 4 次调用重复排序；轮次 ⓘ 仅 hover 显现（键盘/可发现性弱）；**overview 加载失败静默全显"—"**（无法区分新项目 vs 失败，汇入 UI-F 错误反馈战役）；**TaskLedger 孤儿组件**（生产未挂载，台账重设计中——标注停用或随重设计处理）；TrajectoryInspector 引用卡硬编码 hex（汇入 UI-B）；跨会话流徽章区分弱（行首色点/会话列，MA-3 前再议）；tab 条无方向键 + chevron 仅 hover。

**🟢 低**：瀑布图 run 点击 seq 算术推导可能指向不存在行（静默无选中）；列表刷新无 loading 指示；确认弹窗无 Esc/焦点陷阱；**轨迹切回强制贴底丢阅读位置**（注释明示设计意图——建议真机评估改为首载/发送才贴底）；搜索无防抖（n≤1000 可接受）；scrollbar-width:none 无可视线索；设置页双保存语义并存（汇入 FORK-B 同款问题）；stat-band 窄窗挤压临界；task-agent 直色语义；元数据无复制按钮；跨会话轮次号窗口化（v1 已文档化，MA-3 再议）。

---

## 分诊总表（三路 + 亲审合并，2026-08-20 定稿）

### 战役切分建议（顺序待用户拍板）

**战役 UI-1：输入与键盘手感包**（~半天）
- UI-A1 IME 组合期误发送（中文产品第一优先级，一行级）
- UI-A2 生成中允许预写输入 + UI-E1 Cmd+Enter（同点改动）
- D-2/D-3/D-4 选择器键盘三件套（方向键 + ARIA + 焦点环令牌）
- T-1/T-2/T-3 轨迹键盘导航身份比较三处对齐（一个 commit）
- UI-D2 tablist 语义（ChatPage tabbar + 项目 tab）
- 手测：拼音/五笔 IME、纯键盘走完新建→发送→切 tab→查轨迹→开检查器

**战役 UI-2：错误反馈原语**（~一天，符合 L2 状态上屏）
- 抽 ErrorBanner/保存反馈组件（含加载失败+重试态）
- 接通设置域 ~8 处静默点（FORK-B S-1..S-5）
- UI-C1 剪贴板兜底、UI-C2 聊天错误横幅加重试/关闭
- T 组 overview 失败全显"—"（区分新项目 vs 失败）

**战役 UI-3：品牌色单一真相源**（~1-2 天，地基工程）
- D-1：global.css vs tokens.css 双源收敛（81 处 --color-* 迁移语义层）
- 5 个死令牌激活（shadow-focus/select-option-h/dropdown-item-h/spacing-1_5/btn-press）
- 缺口补齐：选中态语义层、danger 背景/悬停层、输入控件高度接入
- 验收 grep 门：`rgba(46,141,100` 全库归零、死令牌全部有消费

**战役 UI-4：视觉散值收敛**（分批，依赖 UI-3）
- markdown.css（43 hex）/global.css（40 hex）主题化
- 组件残留（ChatMessages 22+11 / ConfigProposalCard 8 / AgentSettings 8 / TrajectoryInspector 引用卡等）
- 设置域三套输入样式合并（FORK-B）+ EntityAvatar 色板豁免声明

**战役 UI-5：平台质感与杂项包**（~半天）
- UI-A3 快捷键 composable 化（去 querySelector 脆弱性 + Esc 栈统一）
- UI-E2 窗口标题联动；MoreMenu danger 语义修复（hover 红色消失 bug）
- LogSettings 抢滚动条修复（复用聊天页教训）；列表搜索过滤

### 观察池（有据不做）
- 会话列表虚拟化、IME 深度定制、多窗口/悬浮条、TaskLedger 重设计（随台账）、跨会话轮次号语义（MA-3 再议）、轨迹切回贴底（真机评估后定）、KB 文档删除入口（功能域非表现层）
