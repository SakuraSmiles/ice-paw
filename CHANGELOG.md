# Changelog

格式参考 [Keep a Changelog](https://keepachangelog.com/)，版本号遵循 [SemVer](https://semver.org/)。

## [Unreleased]

## [0.5.3] — 2026-08-26

> 从 0.5.2 以来的主要调整：**S3 六波·生产反馈修正三件**（生产 agent 缺口报告，对码核实后定级修复——其一实为跨层语义 bug 非缺功能）。

### Fixed
- **缩进跨层压制**（生产报告缺口 2，升格为 bug）：此前 `set_format` / `set_cell_format` 写首行缩进 0 只删同层 chars 单位变体，**样式层**（如正文样式 `firstLineChars=200`）透出——工具报成功但缩进仍在。修复：chars 变体（`firstLineChars`/`leftChars`）显式写 0（chars 单位优先于 twips，跨层压制唯一正解），`apply_para_formats` 与 `fresh_ppr_inner`（无 pPr 段落的新建路径）双位点；零值连 hanging 系一并四零（任意元素内优先序下都渲染无缩进），非零值只压 chars。

### Added
- **`set_cell_format` 加 `style` 参数**（生产报告缺口 1）：格内全部段落套用段落样式（显示名或 ID，反查与 set_style 同判）——表格格内段落脱离正文样式的正路（如换到无首行缩进的样式），与「缩进写 0」可叠加使用；纯样式操作合法；结果摘要携带解析后样式 ID。
- **`ppr` 投影格内下钻**（生产报告缺口 3）：表格块从一句「(表格块，无段落属性)」改为指路提示；带 `row+cell` 渲染格内逐段 pPr 原文（寻址与 tblpr / set_cell_format 同口径，行/格越界三段式报错）。
- 顺带：`insert_table_after` / `edit_docx` 工具说明把 `table_style` 参数提显眼（五波已建的能力，生产 agent 不知道）。
- 真机测试点：跨机验证——① 正文样式带首行缩进的文档里，格内段落 `indent_first_line_tw=0` 后 Word 打开无缩进；② `set_cell_format style=` 换样式后格内格式正确；③ `projection=ppr row= cell=` 下钻寻址准确。

## [0.5.2] — 2026-08-26

> 从 0.5.1 以来的主要调整：**S3 四波·表格格式四件**（生产反馈——表格内容能写了但边框/底纹/字体/样式无工具）+ **S3 五波·样式档案与模板个性化**（引擎通用抽象 + 双轨承载）。

### Added
- **Word 表格格式四件**（native 手术，D11「四件全做」）：
  - 读侧格式可见：模型解析 tblPr/trPr/tcPr 特征（样式名/底纹 fill/边框/宽度/垂直对齐）+ `projection=table` 表属性摘要行与格级标注（`(底纹#×)/(垂直=×)/(自定边框)`）+ `projection=tblpr` 三级原文下钻（默认 tblPr / row→trPr / row+cell→tcPr）；
  - `set_table_element`：表/行/格三级通用元素手术——TBLPR 17 / TRPR 12 / TCPR 13 封闭白名单兼 schema 位插入序；gridSpan/hMerge/vMerge 受保护拒改并指路 merge_cells；
  - `set_cell_format`：格级段落+字符格式（段落→格内全部段、字符→格内全部 run），入表格批组合；
  - `merge_cells` / `split_cell`：Word 原生语义——纵并 restart/continue 内容留原格（拆分即恢复）、横并 gridSpan 求和内容按序拼首格；网格列区间对齐判据（同显示格号 ≠ 同网格列）；结构重构独占一批。
- **样式档案与模板个性化**（D12「引擎通用抽象 + 双轨承载」）：
  - 引擎层（def_edit.rs 三操作 + 三投影）：`create_style` 最小出生（同批 create→set 可组合）/ `set_style_element` 容器四档（style 直接子级·pPr·rPr·tblPr，STYLE 22 / RPR 39 白名单 + 复用 PPR/TBLPR）/ `set_numbering_element`（numId→abstractNum→lvl 解析，LVL 12 白名单，共享 abstract 的 numId 披露）；读侧 `projection=styles`（清单+basedOn 链）/ `styledef`（原文整段 = never-write-from-memory 抄写源）/ `numbering`（目录+级别下钻）；顺路件——`insert_table_after` 加 `table_style` 引用建表 / `clear_body`（模板复用清场，块数指纹）/ `merge_cells` 矩形区合并；分族路由（doc/style/numbering 同批「部件互斥」）；
  - 承载层：agent.yaml `word_style_profile` 自由文字块——非空时每回合 system prompt 自动带「Word 文档样式偏好」小节（hooks 同款纯文件旁路）；`set_agent_word_profile` 命令（写/摘除双闸）；**提案通道**——对话里口头表达偏好（如「正文宋体小四、表头深蓝底白字」）→ agent 提案 → 批准一次永久生效（`""`=摘除，🟢 非敏感）；前端审批卡展示项；配套 workspace `templates/` 模板目录约定（copy_file→clear_body→写正文 = 整套模板继承正路）。
- 真机测试点：表格格式四件 Word 打开验收（**尤其合并格表、横并后拆分**——结构重构风险最高）；样式定义改写后投影三层合并显示新值、自动编号值不变；word_style_profile 全链路（口头偏好→审批卡→yaml 落块→下回合 prompt 生效）。

## [0.5.1] — 2026-08-24

> 从 0.5.0 以来的主要调整：**S3 三波·表格四件**（生产实战驱动——交付物文档核心内容是表格，agent 此前无任何表格写入途径）。

### Added
- **Word 表格能力四件**（native 手术，不走 pandoc/COM 外部工具）：
  - `inspect_docx projection=table`：表格网格投影——行r×格c 双 1-based 编址 = 表格操作地址地基；`(跨N列)/(续)/(合并头)/(空)/(嵌套表)` 标注即手术边界说明；列数 = gridSpan 求和（抗整行跨列）；
  - `insert_table_after`：锚块后建表——矩形矩阵 ≤200 行 × 30 列，默认表头加粗 + 跨页重复，100% 宽全边框、列宽按节内容宽派生均分；
  - `set_cell_text`：改格文本——保 tcPr（含合并属性）/首段 pPr/首 run rPr；格内 `\n` = 多段；纵向合并续格拒绝并指路合并头；
  - `insert_table_row_after`：克隆模板行增行——gridSpan/vMerge 整结构克隆，合并格表格唯一正确的增行法；
  - **同块表格批组合**：set_cell_text / insert_table_row_after 同块可多条按序生效（预检虚拟行模拟，「增行 + 填新行格」一批完成）；段落操作误指表格块的报错从「不支持」改为指路三件。
- 真机测试点：表格三操作 Word 打开验收（尤其带合并格的表）+ projection=table 寻址准确度。

## [0.5.0] — 2026-08-24

> 从 0.4.1 以来的主要调整：**Word 能力演进整线**（S0a 结构模型 → inspect_docx 读侧投影 → edit_docx 手术引擎 → S3 格式/编号两波 → D9 通用元素手术层）；**Agent 质量拍 Phase 1**（工具层四件 + system prompt 两层设计 + 风格预设交互重做）。

### Added
- **Word 拟人水平能力线**（改优先，路线见 docs/word-capability-roadmap.md）：
  - **读侧地基**：xml_dom 极小 DOM + docx_model 类型树（段落/runs/表格网格/页面设置/样式链有效格式三层合并：直接 > basedOn 链 > docDefaults）；numbering.xml 接入（编号定义解析 + 计数模拟，outline/format 直接显示自动编号实际值——祖先级未现时按该级自身 start 渲染，治「agent 看不见 3.2.1 是几」盲区）；
  - **inspect_docx 工具**：outline（大纲+样式层级+摘要）/ format（区间 run 级**有效格式**）/ text（带块号正文）/ headers_footers（页眉页脚逐节+空标注+悬空检测）/ ppr（原文 pPr XML）五档投影 + start/end 区间分页；块编址 1-based 混排统一编号 = 编辑地址地基；
  - **edit_docx 手术引擎**：只替换目标 XML 部件、其余 zip entry 字节原样重打包（untouched 逐字节断言）；operations 批量事务全有或全无 + 地址指纹核对（expect_prefix）+ 模型级 diff 读回 + 自动备份/原子写。六操作：replace_text（保 pPr/rPr/开标签属性字节切片）/ insert_paragraph_after（继承锚格式或样式名反查 ID）/ delete_block / set_format（字符+段落格式属性级手术，spacing/ind 属性合并不覆盖未提及值）/ set_style（换样式 3 形态 pStyle 手术，空转显式信号）/ **set_ppr_element**（D9 通用元素手术层——pPr 子元素 ~34 封闭 schema 集一个操作永久收敛段落格式长尾：xml=null 摘除/整体替换按 schema 位插入 + 片段校验禁 xmlns/单根/根名一致 + sectPr/pPrChange 受保护 + 摘 numPr 时样式链回退诚实警告）；
  - **Word 打开验收**：replace_text 路径真机 ✅（用户打开改后文档无报错）；其余操作路径待同标准验收。诚实边界：统一手写编号后 TOC 域缓存是旧值，需在 Word 里 F9 刷新；
  - 配套基建：copy_file 工具（此前复制只能 PowerShell 硬凑，引号经 cmd /C 转手连败 12 次的治本）。
- **Agent 质量拍 Phase 1**（诊断驱动，826 次失败样本分析）：
  - **工具层四件**：write_file `create_dirs` 好默认（父目录自动建，不再因缺目录失败）；报错即行为契约——错误文案三段式（发生了什么+为什么+怎么办）+ not-found 必挂近似候选（did-you-mean）+ **错误首行 = 稳定家族前缀**（doom_loop 错误签名依赖，路径混进前缀会把连败检测打散）；run_command Windows 恒前置 `chcp 65001`（中文输出统一 UTF-8）；**doom_loop 检测**（同签名连败 3 次 nudge 纠正 / 6 次终止清场；与 stuck_detect 分工——前者抓「错误签名不变」，后者抓「轮指纹不变」）；
  - **system prompt 两层设计**：平台层只放风格中立纪律三条（错误纪律/诚实边界/语言跟随）；风格归 agent.yaml `system_prompt`；风格预设三档为**素材**（插入即用户文本，零版本纠缠）；写 yaml 多行块走 set_agent_system_prompt（块级补丁+回读闸+原子写）；
  - write_file 生成 .ps1 自动补 UTF-8 BOM（PowerShell 5.1 无 BOM 按 GBK 解码，中文实参乱码）。

### Changed
- **风格预设交互重做**：居中弹层全文展示（胶囊 tab + 单片全文 + 底部显式确认），创建 Agent 流程可选风格。
- 备份目录嵌套防护：从备份目录恢复不再生成二级备份目录。

## [0.4.1] — 2026-08-22

> 从 0.4.0 以来的主要调整：头像系统重塑（AvatarField 统一组件 + vue-cropper 成熟库裁剪器 + 默认头像全语境统一）；项目身份减法（头像/主题色移除）；预算 HUD 环形化迁位；会话可靠性（60s 静默超时双保险 / 发送失败可见回滚）；任务面板列按存在性渲染。

### Added
- **头像系统重塑**（AvatarField 统一组件 + 裁剪器 + 展示链）：
  - **AvatarField 单一形态**：头像框本体即交互（hover 蒙层 + 右上 × 清空），点击/拖入/Ctrl+V 三通道直达裁剪器，无常驻提示文案；源图上限 2 MB → 10 MB（常量统一三处引用）；
  - **裁剪器换 vue-cropper**（若依同款成熟库）：自研取景框数学三轮真机翻车（ready 死锁 → 长图锁死 → 拖不动）后根治；裁剪 1:1 锁、透明 PNG 白底修复（alpha 检测 + PNG 编码通道）；工程原则成文「成熟库优先——指针交互密集领域自研数学易连翻车」；
  - **Agent 默认头像图全语境统一**：EntityAvatar 渲染链升级三级——用户上传图 → 内置默认头像图（所有展示位无图一律默认图）→ 名字哈希渐变 + 首字（遗留兜底，仅默认图也加载失败时出现）；默认图下沉组件内部，新增展示位结构性不可能漏接；
  - **主会话外层 44px 头像**（kind='chat' 且有 avatar 时占 ChatHeader 两行高度；子会话保持 28px 副头像）；agent 表单身份区三行成组 + 头像 GitHub 风描边投影。
- **任务面板列按存在性渲染**：仅计划或仅任务时只显单列（恒右锚 420px 下拉，不留半张空列）；双列齐备才 880px 并排。

### Changed
- **预算 HUD 迁位 + 环形化**：从消息流两处内联位（生成中 cursor-bar / 轮末 finish-reason 行）迁至**输入框底部工具栏中间槽**常驻——有预算数据即占中（快捷键提示让位），发送中与轮末同一位置；横条改 **14px 环图**（12 点起顺时针、80% 刻度点、≥80% warn 态芯环加深、续期后环随上限抬升回落；环径取文字 ~1.17em 视觉等重）。缓存命中 chip / 续期计数 / 计费口径 title 保留。
- **项目身份减法**：项目头像 + 主题色功能前端全移除（表单/列表/侧栏切换器/详情页/概览成员卡）——payload 不再携带，库存值原地保留（后端双 Option 语义，回滚 = revert）；侧栏切换器圆点回归纯状态标记（scoped 主色 / 散落灰）。

### Fixed
- **60s 静默超时双保险**（治「发送没反应」级联误判）：超时触发时前端不再直接翻转 sending，先问后端 ChatState 注册表真相（仍在跑就重新计时）——Pipeline / 多图 OCR / 慢工具等未知静默窗口结构上不可能误判；配套已知重活步骤 emit `chat:processing` 心跳快速路径。**发送失败可见 + 回滚**：per-conv 错误横幅（含重试）+ 乐观用户消息移除（后端已拒不落库的气泡不再切回凭空消失）；并发拒绝（在途生成）专属文案。
- **工具期滚动跳动**修复；**暗色模式 ::selection 浅字浅底不可读**（令牌化）。
- **附件上传改浏览器原生 input**：macOS 启动时权限弹窗根治（方案 A，与拖拽/粘贴同一条 File 通道）。
- **cargo test --lib 提示勘误**：长期误记为「sodium DLL 入口点缺失」，真根因是 lib #[test] harness 缺 Common-Controls v6 manifest（build.rs 已注入修复）。

## [0.4.0] — 2026-08-21

> 从 0.3.9 以来的主要调整：品牌视觉换代（藏青双锚点 + 字体全离线 + 设计系统九档收敛）；错误反馈原语（8 处静默失败消灭）；S8 无限续写四件（确定性折叠/工具结果瘦身/预算 reminder/触顶收尾）；输入手感（IME 防护/生成中预写/tablist 键盘协议）；跨平台打包（macOS Apple Silicon）。

### Added
- **macOS（Apple Silicon）原生支持**：tauri 资源分置（windows/macos 平台配置）+ prepare 脚本跨平台调度器（win→ps1 / mac→sh）+ libsodium 静态链接 + dmg 打包全链路验证（cargo 917→929 / 真机运行通过）。
- **S8 无限续写机制**（治「长任务被预算/摘要失败打断」）：
  - **确定性折叠**：摘要失败/熔断的两条降级路径从裸截断改为骨架化——中段压缩为工具调用骨架（工具名+参数摘要+成败标记+首行预览），纯本地计算永不失败，agent 不再失忆式续跑；
  - **工具结果瘦身**：近区超 2000 字符的 tool_result 截头尾+指针标记（完整结果可经 @引用 取回），上下文最大头的日常削减；
  - **预算 reminder 注入**：剩余 <10% 时一次性向 agent 注入收敛提醒（自管理收尾/分段），与 HUD pill（给人看）互补；
  - **触顶文本收尾**：续期用尽不再硬停——+4096 收尾额度注入收尾指令，模型输出 3-6 句总结（已完成/未完成/手接点）自然 stop；再次触顶才真终止。
- **错误反馈原语（ErrorBanner 组件）**：inline（纯文本+重试链接，条目级）/ banner（图标+语义底+动作组，列表/页级）双形态；接线 8 处静默失败（删除 Agent/切换 MCP/设置页加载/三列表加载/保存失败/聊天错误横幅可重发/剪贴板三级兜底如实红 ✕）；MCP 状态点正常态隐藏（视觉只留给异常）。
- **窗口标题随会话名联动**（`会话名 — IcePaw`，空回退产品名）。
- **tablist 键盘协议**（useTablist）：roving tabindex + ←→ 循环焦点跟随 + Home/End；ChatPage 与项目详情页两处 tab 统一。
- **Esc 全局栈**（useEscapeStack）：Esc 只关栈顶层（防一次按键关多层），图片预览/附件详情/删除确认条接线。

### Changed
- **品牌色：绿 → 藏青双锚点**：浅色主题 #1E4976（L≈31 墨水质感）/ 深色主题 #4E80C0（档位镜像，L≈53 AA 达标）；十一档色阶单一真相源入 tokens.css，global.css 绿色覆盖与 81 处 `--color-*` 平行令牌拆除；info 并入主色（撞色消除）；agent 引用紫令牌化。
- **字体全离线**（本地优先产品哲学）：Google Fonts 引用移除；拉丁三族 @fontsource + Noto Sans SC 简中子集自托管 + 霞鹜文楷 97 官方分片（unicode-range 按需）——首启离线观感达标。
- **设计系统收敛**（走查台账四路审计 68 条 → 五战役）：字号九档（新增 micro-11，86 处 9~11px 幽灵字号清零）；布局间距令牌化；z-index 阶梯化（27 处魔法数归零）；三套输入样式合并；颜色内联 fallback 清零。
- **输入手感**：IME 组合期防护（拼音选词 Enter 不再误发送）；生成中可预写下一条；Cmd/Ctrl+Enter 备选发送；轨迹键盘导航三处身份比较修复（流式期间 ↑↓ 不跳首尾）。
- **视觉规范九则成文**（CLAUDE.md）：色彩三层架构/零 emoji 走 Lucide/字号九档/字体本地化/z-index 令牌/加载态三档/图片规范/文案三段式/无障碍基线。

### Fixed
- **CI 三层修复**：pnpm lockfile 失配（字体试装中断残留）/ provider_test 断言未随 0.3.9 预算诚实化更新（连续红 6 commit）/ clippy 新版 lint（chunks_exact→as_chunks）+ eslint 4 处。
- **暗色主题聊天区令牌缺失**：UI-4 别名升格时暗色块插入静默失败——六令牌补入双暗色区，三主题区交叉审计机制建立。

## [0.3.9] — 2026-08-19

> 从 0.3.8 以来的主要调整：Token 预算诚实化（缓存折扣计量 + 前缀稳定 + 命中上屏 + DeepSeek 兜底，治长任务被预算熔断打断）；生成中全局卡顿系统性修复；预算胶囊改微型进度条。

### Fixed
- **Token 预算按缓存折扣诚实计量**（治生产长任务被预算熔断反复打断需手动「继续」）：预算累计从毛成本 Σ(prompt+completion) 改为**计费口径**——未命中全价 + 命中 1/10 + 输出全价（`budget::billed_tokens`；1/10 取各 provider 缓存定价最贵档，保守方向）。生产实证 96% 缓存命中的长任务曾按全价虚收约 10 倍、烧穿 9M 毛顶被硬停，计费口径实际 ≈1.5M 本可 3M 初始额度内一步跑完；无缓存 provider（Ollama）公式退化为原语义零影响。配套归一化：TokenUsage 钉死规范语义（prompt=总输入含命中、cached≤prompt）+ `into_canonical` 自愈守卫接全 provider 唯一 Usage 汇聚点（一处治累加/上屏/落库三条出口，治 deepseek 兼容端点 prompt 只报 miss 的怪癖）；Anthropic 适配层显式归一（cache_read 与此前完全漏计的 cache_creation 折入 prompt）。
- **DeepSeek 私有缓存字段兜底解析**：`prompt_cache_hit_tokens`/`prompt_cache_miss_tokens` 私有对同时在位时按官方恒等式 prompt = hit + miss 重建总输入（权威，优先于标准字段）——治该端点标准 `prompt_tokens_details` 间歇缺失致缓存计量退化全价。
- **工具列表出口按名排序**：注册表 HashMap 迭代序逐轮漂移会使工具定义（provider 请求前缀最前）字节变化 → 前缀缓存大面积 miss（MiniMax 缓存前缀序 = 工具 → 系统提示词 → 历史）。排序后逐轮字节级一致，缓存从第 2 轮起稳定命中；相关性排序功能不变（只确定基准序与同分并列序）。
- **生成中全局卡顿系统性修复**（低配真机生成时设置页通用加载中/新建智能体模型下拉十几秒/项目概览不出数据）：三层——①每个 SSE delta（≈每 token）无节流直发前端打满 Tauri 主线程与前端 JS 主线程两条单车道，改 40ms 窗口聚合（DeltaAggregator，每通道每秒 ≤25 次，工具调用参数完整性与退出纪律保全）；②`list_providers`/`check_nodejs` 同步命令占主线程，async 化退出；③前端 content_blocks 解析 memo 化（字符串 key，同引用只读）+ toolResultIndex Map 消 O(n²) 全列表重渲染。

### Changed
- **自动续期额度 2→4**：预算改计费口径后高命中路径很难触顶，4 次额度专防冷缓存极端（首轮近全价燃烧）过早真停；总封顶 = 初始 ×5 仍线性有界；失控循环不靠续期兜底（stuck_detect 独立熔断仍在）。
- **预算胶囊改微型进度条**：去常驻绿底，灰轨绿芯 + 80% 处弱刻度线，填充度一眼读出余量、≥80% 转 warn 态（芯条加深 + 字重 600）；数字旁新增「**缓存命中 X%**」chip（无缓存数据隐藏）——看得见系统在省钱、知道预算为什么涨得慢；title 说明计费口径与折扣公式。

## [0.3.8] — 2026-08-18

> 从 0.3.7 以来的主要调整：项目详情页概览「项目成员」卡（token 环图 + 排行 + 聚焦透镜联动）；聊天输入框工具栏重排；三处恢复/直链/环图几何修复。

### Added
- **概览「项目成员」卡**：新维度成员负载分布——SQL 聚合各成员消息数与 token 估算（`list_project_agent_shares`），前端环图（token 占比分段、primary 单色阶、中心总量 `K/M` 惯用计数）+ 四列共享 grid 横条排行（名字/模型 | 归一条 | tokens | 消息小字）；>5 成员截断 Top5 + 「其他 N 位」双口径聚合；token 全零（估算未回填旧库）回退消息口径并隐藏环图，诚实不伪造；**hover 聚焦透镜**——环图段 ↔ 排行行双向联动（聚焦段变粗 11→14.5 + 其余段/行淡出 + 中心切该成员值）。

### Changed
- **聊天输入框工具栏重排**：发送/停止按钮从右上角移入底部工具栏最右（32px，比 24px 工具按钮大一圈，主操作视觉权重）；「Enter 发送 · Shift+Enter 换行」提示从框外移入工具栏居中；输入区因此左右全宽；内容到边框四周统一呼吸间隙，工具栏行底边对齐（大小混排齐底线更稳）。
- 环图段几何收敛为 **butt 方头 + 无间隙连续环**：段所见长 = 占比 × 周长首尾相接，hover 变粗纯径向扩张零重叠；极小段 1 弧长最小可见宽保 hover 命中（曾两轮 round cap 端点延伸造成段衔接错乱，弃）。

### Fixed
- **启动恢复侧栏 scope 错位**：恢复有效会话时 scope 改为忠实记忆的上次侧栏所在（`saved.projectId`），不再跟随会话所属项目——治「内容页恢复项目详情页、侧栏却在散落会话」的错位（scope 与会话早已解耦：详情页不动 scope / 切项目不动会话上下文）。
- **刷新直链误报「项目不存在或已删除」**：project store `load()` 的「loading 中直接 return」会短路详情页的 `load(true)`（Sidebar 请求在飞行、列表仍空）→ 改为**飞行 Promise 共享**，并发调用方等同一个请求落地再下结论。

## [0.3.7] — 2026-08-17

> 从 0.3.6 以来的主要调整：S1——session_events 升格唯一读路径（legacy 拼装退役）+ 摘要锚点 seq 化 + Image 双份存储治理；真机验收五项全绿。

### Changed
- **legacy 拼装退役（S1 阶段 1）**：事件派生 `load_history_from_events` 成为唯一生产读路径（恒 Derive）；read_route 降级为健康监控——非绿（无事件/对账差异/混合纪元）记 error 日志后**照常派生**，写路径 bug 不再被自动回退兜底静默吞掉。messages 表双写持续保留为回滚底座（revert 阶段 1 commit 可整体恢复，零数据损失）。
- **摘要锚点 seq 化（S1 阶段 2）**：migration 46 `covered_until_seq`（被覆盖消息首现事件 seq，与派生排序位严格一致）+ 存量回填；摘要状态双写双读，锚点定位 seq 优先 rowid 兜底——根治 messages 表无 AUTOINCREMENT 的 rowid 复用漂移风险，旧事件零迁移。
- **Image 双份存储治理（S1 阶段 3）**：消息类事件 payload 的 Image 块改轻量引用 `image_ref`（payload v2，字节只在 messages 行）；写侧唯一入口 `refify_blocks`，读侧三路水合（LLM 视图 / 对账 / 前端轨迹与导出），未命中诚实降级文本标记不静默消失；BACKFILL_VERSION=2 纯 backfill 会话 boot 自动重写自愈，v1 内联旧事件永久可读。真机实测：两张图的事件 payload 从潜在 4.7MB 双写降至 326 字节。
- **S1 真机验收五项全绿**：backfill（9 会话 824 事件零失败）／恒 Derive（当日路由决策全 green 零 diff）／发图 payload 无 base64（模型回复描述画面 = 水合实证）／摘要折叠 `covered_until_seq` 落值／轨迹检查器图片 v1/v2 两形态显示正常。

### Fixed
- 终止原因文案收敛 `utils/termLabels` 单一真相源——`backfill` 补「历史补录」标注 + 非异常化呈现。

## [0.3.6] — 2026-08-17

> 从 0.3.5 以来的主要调整：UX 细节轮收官、模型配置重设计、token 预算全分层修复、S 批次结构减法、旧会话事件 backfill。

### Added
- **模型配置重设计（Provider 注册表单一真相源）**：后端 `PROVIDERS` 9 条目录元数据供前后端共用 + `list_providers` / `test_provider_connection` 命令（测试连接与拉取模型合一，一次往返两用）；前端模型选择改 GroupedSelect 分组下拉（Provider 品牌图标、组头不可选）→ combobox 可选可输（手输目录外名字落自定义）；预设厂商 URL 锁定只读，智谱双端点 `alt_urls` 自动匹配固化；空 Key 按 provider 目录判定放宽（Ollama 本机无需 Key）。
- **UX 细节优化清单 12 项 + 修复轮**：审批重做——按注意力路由（输入区上方/消息流内分层）+ 分层授权记忆；可调面板宽度 + 记忆 + 规范化管理；轮次导航条 v2（定容滑动窗口，N/M 徽标跨位不漂移）+ 任务胶囊深化；全局过渡动画统一「淡入+微升」+ `prefers-reduced-motion` 兜底；委派标题去前缀 + agent 名徽标；项目快速新建；头部操作外置。
- **token 预算全分层修复**：摘要自适应额度 4096→16384（连续空结果翻倍、成功回落、3 连空触发熔断）；预算可观测——`chat:budget` 事件 + 预算 pill HUD（≥80% warn 态）+ 续期 toast，终止文案带数字与指引；agent.yaml 定向改写命令（`get_agent_yaml_fields` / `set_agent_yaml_field`，白名单键 + 写前重解析校验 + 原子写）。
- **旧会话事件 backfill（session-event-log Phase 2B 前置）**：boot 幂等扫尾——给零事件旧会话反向合成 `session_events`（reconcile 的逆函数：同 parser / 同空回退 / 同容忍清单 → 构造性零 diff → read_route 自动路由 Derive）；`turn_context` 不合成（旧行无 provider/model 快照，不伪造）；actor=`backfill` 行是派生数据可重跑，termination=`backfill` 诚实标注，created_at 直传行时间戳；版本化重跑自愈（BACKFILL_VERSION 落 preferences，代码>库内 → 纯 backfill 会话删旧重写）+ 冻结规则（混入真实事件后永不可重写）。
- **send_message 全链路 e2e（S5）**：`session_runner_e2e` 六场景（正常 / 空响应 / 限流退避中取消 / 显式预算触顶 / 流中取消占位 discard / 工具轮配对），MockProvider `ToolCallThenText` 驱动，断言消息行 + 事件序 + UI 瞬态事件 + TurnSummary 四层。

### Changed
- **S 批次结构减法（测试数不降硬约束）**：S2 `protocol.rs` 1161 行拆 `protocol/` 目录（llm / input / events，全库导入零改）；S3 chat_cmd 附件机器整体迁 `harness/attachments.rs`（695→~290 行回归编排门面）；S4 LoopConfig 数据袋（auth 运行时件挪 LoopContext、`StreamLoopInput` 成袋删超长签名）；S6 主循环链去 AppHandle 硬依赖——`LoopEmitter` trait + 七模块换装（瞬态 UI 进度与可回放事实两通道分明）；S7 `tool_trim_threshold` 废弃字段全链摘除（schema/repo/命令/前端，serde 容忍旧 yaml）。
- **摘要链路治理**：stream_summary 走默认方法 + GLM 摘要请求注入 `thinking:disabled` + 连续空结果熔断（3 次 10min）；MemoryStage Err 降级不阻塞回合。

### Fixed
- **GLM thinking 烧光摘要额度 → 空摘要 → 历史永不折叠 → 每轮全量重发触顶**：三重治理 + 摘要锚点 SQL 排空占位行 + IO 视位修复；轮次导航条双修（视位冻结）。
- **崩溃后 turn 永远「进行中」死数据**：boot 补记未闭合 turn 的 `turn_ended(interrupted)`（幂等扫尾，历史脏数据自动治好）。
- **父会话委派卡泄漏进子会话**（跨会话流式态复位收敛单一入口）、删除复活竞态、审批卡宽度对齐输入框。
- **workspace 路径判定**：`infra/path_norm` 共享归一判定；前端 DB 时间解析归一 `parseDbTime` + 侧栏后台刷新骨架屏闪现。
- **模型拉取 401 撞脸**：无 Key 短路引导 + 错误翻译 + 存量 Key 不跨 provider 混用。
- CI Linux 编译：首启窗口尺寸的 `hwnd()` 调用补 `cfg` 门。

## [0.3.5] — 2026-08-15

> 从 0.2.7 到 0.3.5 的主要功能调整（合并概括，未逐小版本拆分）。

### Added
- **会话事件日志与轨迹视图**：基于 migration 44 `session_events` 表的 append-only 事件日志基石，统一 session / 多 agent 图协作 / 轨迹可还原。词表 13 kind + typed emitters，事件 inline `.await` 禁 spawn，turn_ended 必须先于 cleanup() unregister 落库；supersede 机制让同一 `message_id` 的多次 assistant_message 自动续写，回放 last-wins；导出命令 `export_session_trajectory` → JSONL。
- **会话事件对账与派生回放（Phase 1）**：`harness/derive.rs` 纯回放（supersede last-wins / 空回退对称 / 坏 payload 记 issue 不吞）+ `harness/reconcile.rs` A 侧 legacy / B 侧事件回放 / turn 锚点走查分组，对账平面 = 行级原始形态。`reconcile_session` 命令只读出口。
- **事件日志读路径切换（Phase 2A）**：`harness/read_route.rs` 按会话路由——有事件 + 对账零 diff + 纯事件纪元 → Derive（`load_history_from_events` 派生 `Vec<MessageRow>`，锚回真 rowid），其余 → Legacy；零风险（派生输出与 legacy 同构同函数）；指纹缓存 `(max_seq, max_rowid)` 追踪新数据；偏好 `session_read_path=legacy` 一键回滚；诊断命令 `get_read_route_status`。
- **文件系统工具集 native 化**：bundled filesystem server（`@modelcontextprotocol/server-filesystem`）下线，6 个核心工具与内置 native 重复，删除；其独有 5 项补成 native 内置工具，授权统一为 `PathWhitelist`：`directory_tree`（递归目录树，跳过 .git/node_modules，限深度 8/节点 2000）、`move_file`（移动/重命名，跨卷回退 copy+delete，源文件自动备份）、`create_directory`（建目录含父目录，幂等）、`get_file_info`（文件元信息）、`read_multiple_files`（批量读 ≤20 文件，单文件 >1MB 跳过）。
- **配置提案 Guardrail（Phase 1）**：`propose_config_change` 工具 + `proposal_guard.rs` + `proposal_registry.rs`，agent 全程无写权限。Guardrail 三档分级：🔴 红线（删除/跨 agent/api_key 非占位符）→ 拒绝；🟡 敏感（带工具 / enabled_tools 变更）→ Medium；🟢 非敏感（名称/温度/system_prompt）→ Low。API Key 走引用槽位 `__SLOT__`，用户在审批卡片亲手填。写保护加固：`reject_sensitive()` 拦硬写 agent.yaml + `register_meta_tools()` 强制注入合法通道。
- **视觉能力统一适配**：4 个 Image 块注入入口统一走"按有效视觉能力适配"，杜绝向非视觉模型塞 Image。`provider/model_info.rs::effective_supports_vision`（OR 关系：agent 显式 =1 权威，=0 按模型表自动探测）；`harness/modal.rs` 统一 `gather_vision_candidates` / `adapt_blocks_for_vision` / `strip_image_blocks_to_marker`；4 入口接线（用户上传 / 工具返图 / 历史 / `view_attachment_image`）。
- **上下文预算与滚动增量摘要**：真实 token 估算（覆盖 tool_use/tool_result/thinking/image 块）+ per-agent `context_window`；`TokenWindowStage`（max_input_tokens 的 80% 硬裁历史）；Phase 2 滚动增量摘要（`covered_until_rowid` 追踪 + fold 55%·40%）。
- **多 Agent 委派与 Loop 拆分**：loop_engine 1343→697，抽 `loop/` 子模块（context/events/reason/retry_round/stuck_detect/token_usage）；chat.ts 843→532（抽 useChatEvents）；Sidebar / ChatMessages 抽 composables。
- **工具名合规化与 OpenAI 适配**：migration 39 `tool_index` 列 + `t{idx}_` 命名 + 历史 sanitize（修工具名违反 `^[a-zA-Z0-9_-]+$`）；OpenAI 适配层 `chat_message_to_openai` 1→N 展开 tool_result 为多条 `role=tool`（OpenAI-only，Anthropic 零改）。
- **内置 WebView2 离线安装器**：Windows 安装包改用 `offlineInstaller` 模式，把 WebView2 Runtime 离线安装器嵌入 MSI/NSIS，纯净 Windows 双击即装。

### Changed
- **内置工具清单动态化**：设置页「内置工具」由后端 `list_builtin_tools` 命令 + `register_builtin` 单一事实来源驱动，前端动态拉取；中文描述降级为本地化文案层，缺失回退后端原文。
- **内置 MCP 运行时**：3 个轻量内置 server（sequential-thinking / memory / filesystem）从 npx 运行时拉取改为安装包自带 Windows-x64 Node + 预打包 `node_modules`，运行时零网络、零系统 node 依赖；filesystem 已随 native 化下线，bundled runtime 仅保留 thinking / memory。

### Fixed
- **外部 MCP 工具调用分发**：`ExternalToolProxy` 把带 `server_name.` 前缀的工具名原样发给 server 导致 -32602，拆成 `name`（带前缀，LLM 侧）+ `server_tool_name`（原始，发 server）两字段。
- **CI 修复**：bundled runtime 起 CI 红——`tauri-build` 在编译期校验 `bundle.resources`，CI 改为创建占位 resources 让校验通过；顺带修 4 处潜伏 clippy 违规。
- **错误横幅跨会话串味**：A 会话出错后切到 B 会话顶部仍显示 A 的错误 → 按 `conversation_id` 隔离（Map + computed）。
- **filesystem server 包名 404**：`@anthropic-ai/mcp-server-filesystem` 已下架 → `@modelcontextprotocol/server-filesystem`。
- **命令行窗口闪现**：Windows 上 `run_command`/`git` 等工具控制台一闪而过 → 统一 `CREATE_NO_WINDOW`。
- **工具打分历史权重失效**：`tool_calls` 空壳表导致 scoring 维度从未生效 → 随审计接入自动恢复。
- 切会话不丢卡片、取消通道、emit→invoke 修正、thinkingTimer KeepAlive 生命周期修复、工具授权弹窗背景点击不再误触拒绝、P0 稳定性修复（crypto Mutex 毒化 / spawn token 残留 / reqwest `expect` 崩溃 / 前端事件监听器泄漏 / TS 预存错误）。

## [0.2.7] — 2026-08-07

### Changed
- **内置 WebView2 离线安装器**：Windows 安装包改用 `offlineInstaller` 模式（`bundle.windows.webviewInstallMode`），把微软 WebView2 Runtime 离线安装器嵌入 MSI/NSIS。纯净 Windows（无 WebView2、无网络）双击即装，彻底告别「缺少 WebView2 Runtime」报错。代价：安装包体积 MSI 41M→241M、NSIS 26M→229M（+约 200MB，微软该安装器实际体积，比 Tauri 源码注释里的 127MB 旧值大）。安装器由 Tauri 打包时自动从微软 CDN 下载并嵌入（编译期不校验文件存在，故 CI 无需为其建占位）；`offlineInstaller` 在 Tauri v2 schema 中**不接受 `path` 字段**（仅 `silent`），与部分教程说法相反。

## [0.2.6] — 2026-08-07

### Fixed
- **内置工具清单动态化**：设置页「内置工具」原为前端硬编码数组，与后端 `register_builtin` 漂移——0.2.5 新增 5 个 native 文件工具时漏改前端，导致设置页一直少显示（工具实际可用，仅展示漏，非数据库问题）。改为后端新增 `list_builtin_tools` 命令（复用 `register_builtin` 单一事实来源）+ 前端动态拉取，计数与清单永远反映真实，新增工具零漂移；中文描述降级为本地化文案层，缺失回退后端原文。
- **CI 修复**：0.2.4 bundled runtime 起 CI 一直红——`tauri-build` 在编译期校验 `bundle.resources`（node.exe / node_modules）存在，但这些由 `prepare:mcp` 下载、gitignore 不入库，CI 从未 prepare，build script 在 cargo check 阶段就炸。CI 改为创建占位 resources 让校验通过（CI 只验编译、不产出安装包，真实打包仍走 `beforeBuildCommand` 的 `prepare:mcp`）。顺带修 4 处潜伏 clippy 违规（此前 CI 在 check 就死，从未暴露）。

## [0.2.5] — 2026-08-07

### Changed
- **移除「工程专家团队」内置工具集**：价值低、依赖系统 node + npx 联网拉取，不再随产品内置。已安装用户的旧记录由 migration 36 自动清除。
- **「文件系统工具集」整合为 native 内置工具**：原 bundled MCP Server（`@modelcontextprotocol/server-filesystem`）的 6 个核心工具与内置 native 工具完全重复且内置更优（自动备份 / 唯一性校验 / 大文件分页 / 噪音目录过滤），予以移除；其独有的 5 个能力补成 native 内置工具，零 node 进程开销、授权模型统一为 `PathWhitelist`：
  - 新增 `directory_tree`——递归目录树（跳过 .git/node_modules 等，限深度 8 / 节点 2000）。
  - 新增 `move_file`——移动 / 重命名（跨卷回退 copy+delete，源文件自动备份）。
  - 新增 `create_directory`——建目录含父目录（幂等）。
  - 新增 `get_file_info`——文件元信息（大小 / 类型 / 只读 / 修改·创建·访问时间）。
  - 新增 `read_multiple_files`——批量读 ≤20 文件（单文件 >1MB 跳过；多路径无法自动放行故每次确认）。
  - `extract_path_from_args` 扩展支持 `source`/`destination`，使 `move_file` 可经 source 走白名单授权。
- 内置 MCP 运行时不再打包 `@modelcontextprotocol/server-filesystem`（thinking / memory 仍需保留 node runtime）。

## [0.2.4] — 2026-08-07

> 0.2.x 线的 beta 阶段第 4 个迭代。

### Added
- **内置 MCP 运行时**：3 个轻量内置 server（sequential-thinking / memory / filesystem）从 npx 运行时拉取改为安装包自带 Windows-x64 Node + 预打包 `node_modules`，运行时零网络、零系统 node 依赖。修复生产 0.2.3 上「深度推理」因 npx 缓存缺传递依赖 `zod` 启动失败。Playwright/maifady 维持 npx。

### Fixed
- **外部 MCP 工具调用分发**：`ExternalToolProxy` 原把带 `server_name.` 前缀的工具名（如 `深度推理.sequentialthinking`）原样发给 server，server 只认原始名 → JSON-RPC -32602 "Tool ... not found"。proxy 拆成 `name`（带前缀，LLM 侧）+ `server_tool_name`（原始，发 server）两字段。潜伏 bug，影响所有外部 MCP 工具调用（非 bundled 专属）。
- **错误横幅跨会话串味**：`lastError` 原为全局 ref，A 会话出错后切到 B 会话顶部仍显示 A 的错误。改为按 conversation_id 隔离（Map + computed）。
- **filesystem server 包名 404**：`@anthropic-ai/mcp-server-filesystem` 已下架，随 bundled 运行时迁到 `@modelcontextprotocol/server-filesystem`。

## [0.2.3] — 2026-08-07

> 0.2.x 线的 beta 阶段第 3 个迭代。

### Added
- **工具调用审计**：`tool_calls` 表接入 `tool_executor`，每次工具调用记录 tool_name/arguments/result/is_error/耗时/起止，可回溯 agent 行为与排查慢命令。

### Fixed
- **命令行窗口闪现**：Windows 上 agent 调用 `run_command`/`git` 等工具时控制台窗口一闪而过（统一 `CREATE_NO_WINDOW`）。
- **工具打分历史权重失效**：`tool_calls` 空壳表导致 `scoring` 的「最近调用加权」维度从未生效，随审计接入自动恢复。

## [0.2.2] — 2026-08-06

> 0.2.x 线的 beta 阶段第 2 个迭代（对应原计划的 beta.2）。自 [0.2.0-beta.1] 起，改用 patch 位编码迭代号，版本号统一为纯数字（MSI 兼容）。

### Added
- **Agent 代配置（提案模式）Phase 1**：`propose_config_change` 工具，LLM 从对话中提出创建/修改 agent 提案，用户审批后生效。Guardrail 校验层（🔴 红线永久拒绝）。前端审批卡片（字段全展开 + API Key 安全输入）。

### Fixed
- **MiniMax 2013**：`sanitize_history` 丢弃孤儿 tool_use 与空消息占位、合并连续同角色消息；LLM 400 错误诊断增强（8421f13）。
- **P0 稳定性修复**：crypto Mutex 毒化、spawn token 残留、reqwest `expect` 崩溃、前端事件监听器泄漏、TS 预存错误（531d6a2、dcfc6ab）。
- thinkingTimer KeepAlive 生命周期：切会话后定时器不再泄漏/错乱（159cc9b）。
- 工具授权弹窗背景点击不再误触「拒绝」（80290fe）。
- **审批/授权可靠性**：切会话不丢卡片、取消通道、emit→invoke 修正（a4f0e5f）。

### Changed
- CI 修复：Phase 1 引入的测试编译错误与前端 lint（734a01f、1e49a43）。

## [0.2.0-beta.1] — 2026-08-05

### Added
- **对话钩子系统**：4 个生命周期接入点（ConversationStart/BeforeLlm/AfterTool/ConversationEnd）+ 3 个内置动作（InjectPrompt/CallTool/Log），配置在 agent.yaml。
- **产品帮助知识库**：6 篇中文帮助文档种子到全局 KB，agent 可通过 search_kb 自服务检索。
- **RAG v2 语义检索修复**：修配置读取 bug + 召回阈值 + RRF 混合检索 + 切换模型自动重建向量 + 可观测性 UI。
- **MCP 架构重设计**：统一 Server Pool 状态机，启动不阻塞，前端简化。
- 项目归档/移动会话/双层 Option workspace_path/N+1 修复/activeProjectId 校验

## [0.1.0-beta.1] — 2026-08-02

### Added

- OpenAI / Anthropic / 智谱 GLM / DeepSeek / MiniMax 多 Provider 支持
- Agent 管理：创建、编辑、删除，独立配 provider/model/system prompt/temperature
- `agent.yaml` 文件配置：放在 agent workspace 里自动读取
- 会话管理：新建、重命名、置顶、删除、搜索、分页加载
- 流式聊天：Markdown 渲染、代码高亮、thinking 和 tool_call 展开
- 消息复制、图片粘贴、链接外链打开
- 项目空间：创建、编辑、切换、归档/恢复、永久删除
- 项目内成员管理
- MCP 内置工具：read_file、write_file、edit_file、list_directory、search_files、run_command、git、web_fetch、search_kb、read_kb_document、save_to_kb、read_agent_config
- 外部 MCP Server：stdio JSON-RPC 连接，global/per_agent scope，trusted/untrusted 权限
- 知识库（RAG v1）：文件自动索引、语义检索、agent/项目/全局三级 scope
- API Key Stronghold 加密存储
- 统一时区系统（设置页改时区即时生效）
- 暗色模式
- 日志查看页（daily rotate 持久化）
- 420 Rust tests + 31 前端 tests

### Changed

- chat 模块从 1568 行单体拆为 context pipeline（Template → OS → SystemPrompt → History → Memory → Final）
- LLM provider 抽象为 trait + 多 adapter 模式
- 引入 LoopBudget + RetryState 替代硬编码常量
- 项目卡片改为内联编辑
- 会话切换不再重挂载组件（消除闪屏）

### Fixed

- 会话卡死：Pipeline 中途失败时 conv_id 永久残留 → scopeguard RAII 守卫
- MCP env 泄漏：子进程继承全部环境变量 → 白名单过滤
- 流式生成中切走再切回内容丢失 → bgStreams 快照
- 侧栏 >30 天旧日期截 UTC 时区错误 → 统一时间系统
- 浅色主题暗色气泡不可读 → tint token 统一
- Base URL path 被截断、MiniMax 400 错误、finish_reason 泄漏等多处小修
- 22 处 dead_code → 清理至 3 处（均为有意保留）
