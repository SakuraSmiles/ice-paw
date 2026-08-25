# Word 能力演进路线（2026-08-24 拍板开工）

> 状态：**已拍板开工**。范围决策：**Word 优先、拟人水平（读+写+改，含格式调整）**；
> Excel 写/改、pptx 读、docx 渲染预览延后不开工（评估结论见文末「延后项」）。
> 顺序决策（D1）：**改优先**——S0 读地基 → zip 手术引擎 + edit_docx → 生成 write_docx 后补。

---

## 一、「拟人水平」的验收定义

一个熟练 Word 用户能做到的事，agent 全部闭环：

| 维度 | 拟人 = 能做到 |
|---|---|
| 读 | 看懂文档：大纲层级、各处生效的样式/字体/字号、表格结构、图片位置、页眉页脚、页面设置；任意位置精确定位引用 |
| 写 | 从零交付出手即可用的成品（报告/信函/简历）；能沿用用户已有文档的样式风格（模板捐赠） |
| 改 | 指哪打哪：改文本保留周边格式；重格式化（字体/字号/颜色/对齐/行距/缩进）；重构（插删移段落、表格行列、合并单元格）；文档级批量（全部替换、按样式批量改、改样式定义全局生效）；**改完不碰坏任何没让动的东西** |
| 闭环 | 每次修改后知道自己改了什么（模型级 diff 读回）；永不损坏文档 |

拟人 ≠ 覆盖 Word 三十年格式面。定义为**常见文档任务 95% 场景闭环**。

## 二、演进步骤（改优先顺序，2026-08-24 D1 拍板）

| # | 步骤 | 成果（可验收） | 规模 |
|---|---|---|---|
| 0 | 路线落账 + 夹具地基 | 本文件 + 真实语料本地放置（tests/fixtures/docx/，🚫不入库）+ 语料冒烟测试 | 小 ✅ |
| 1 | **S0a 结构模型** ✅ | docx.rs 重写为「解析一次 → 类型树 → 投影」：段落/runs/表格网格/页面设置/样式链有效格式；text 投影由模型派生，现有 read_file/KB/附件行为零回归 | 大 ✅（2026-08-24：xml_dom 极小 DOM + docx_model 类型树落地；全量 986 绿 + clippy 0；零回归双闸见 §五-6） |
| 2 | **S0b inspect_docx 工具** ✅ | outline（大纲+样式层级+摘要）/ format（区间 run 级**有效格式**=样式链合并值）/ text（带块号正文）三档投影 + start/end 区间；块编址 1-based 混排统一编号 = 步骤 3 edit_docx 的地址地基；styles.xml 接入（styleId→显示名/basedOn 链/docDefaults）。真机验收：对话问「这文档大纲/第 N 段什么格式/表格结构」答得准 | 中 ✅（2026-08-24：工具已注册 register_builtin；outline 默认 ≤400 块/format ≤50/text ≤100 token 分级，被 SDP 1780 块语料正当化） |
| 3 | **zip 手术引擎 + edit_docx MVP** ✅ | 手术引擎（只替换目标部件、其余 entry 字节原样重打包 + 字节相等断言）；edit_docx 批量事务 + 地址指纹核对 + 模型级 diff 读回。真机验收：Word/WPS 打开改后文档不报损坏、格式保留 | 大 ✅（2026-08-24，commit c79d0cf）：三操作 MVP——replace_text（保 pPr/rPr/开标签属性字节切片）/ insert_paragraph_after（继承锚格式或样式名反查 ID）/ delete_block；全有或全无 + expect_prefix 指纹 + 错误家族前缀稳定 + 备份/tmp 原子写；untouched entry 逐字节断言 + 三份语料千块级定位器对齐绿（1019 测试/clippy 0）。原词表 v1 里的字符格式/段落格式/表格行列操作让 S3 长尾（MVP 砍定）。**Word 打开验收（部分）✅**：2026-08-24 用户真机打开 replace_text 改后文档无报错——文本手术路径关闭；set_format/set_style/numbering 路径待同标准验收 |
| 4 | 体验小件（可穿插） | 工具结果卡「打开文档」按钮（系统默认应用 + 在文件夹中显示） | 小 |
| 5 | S1a write_docx 生成（后补） | 内容块模型 + 中文好默认样式表 + 备份/原子写；真机验收：agent 生成报告，Word/WPS 打开排版合格 | 中 |
| 6 | S1b 模板捐赠模式 | write_docx 支持样式供体；真机验收：拿用户真实模板生成，样式一致 | 中 |
| 7+ | S3 格式与编号循环（使用驱动，每件一 commit） | **首波四件 ✅（D8 拍板 2026-08-24，全落地未手测）**：① numbering.xml 接入（66080e8）——解析编号定义 + 计数模拟，outline/format 显示自动编号实际值（治「agent 看不见 3.2.1 是几」盲区）② set_format 操作（8faeac1）——字符/段落格式属性级手术：对齐/行距/段前后/缩进 + 粗斜/字号/颜色/字体，spacing/ind 属性合并不覆盖未提及值，作用于块内全部 run ③ set_style 操作（cab70c8）——已有段换样式（标题升降级），3 形态 pStyle 手术 ④ 页眉页脚读（f1e89f9）——headers_footers 投影按节组织，页眉/页脚/类型/内容 + 空标注 + 悬空检测。**真机 agent 复盘修正批（2026-08-24）**：numbering 手调 start 语义修复（df2dace——未现祖先级渲染该级自身 start 而非 1，%N 按被引用级 numFmt 渲染）+ set_style 空转显式 `style_unchanged` 信号（生产样本：agent 把「成功」误读成「没生效」，转而回滚已正确的文档）+ 配套基建 copy_file 工具（此前复制只能 PowerShell 硬凑，引号经 cmd /C 转手连败 12 次）/ write_file .ps1 补 UTF-8 BOM（PowerShell 5.1 无 BOM 按 GBK 解码，中文实参乱码）/ 备份目录嵌套防护（1050 测试/clippy 0）。**二波首件 ✅（D9 拍板 2026-08-24，9b2a2e1，未手测）**：⑤ set_ppr_element 通用 pPr 元素手术——诊断驱动（agent 统一编号时前缀写号全成功、最后摘 numPr 无工具可用）；pPr 子元素是 ~34 个封闭 schema 集，一个通用操作收敛全部段落格式长尾（xml=null 摘除/Some 整体替换，PPR_ELEMENTS 白名单兼当 schema 位插入序；sectPr/pPrChange 受保护；片段校验禁 xmlns/单根/根名一致）+ inspect_docx projection=ppr（块号+原文 pPr XML，never write OOXML from memory）+ 样式链回退诚实警告（段级 numPr 摘除后样式链仍定义编号 → Word 回退显示样式编号）+ 空转信号（element 不存在 →「空转，文档未变」）；cargo 1060 / clippy 0。**诚实边界（文档化非代码）**：统一手写编号后 TOC 域缓存是旧值，需在 Word 里 F9 刷新。**三波表格四件 ✅（2026-08-24 生产驱动 + 用户拍板「四件全做」，未手测）**：生产反馈 agent 明确回复「edit_docx 不能改表格」「真正必须的是把表格写进 docx 的途径」（交付物文档核心内容全是表）。诊断双缺口——写侧全操作拒表格块 + 读侧 blocks_text 格文本无分隔（agent 连格都寻不了址）。落地：⑥ inspect_docx projection=table 网格投影（行r×格c 双 1-based 编址 = 表格操作地址地基；跨列/续/合并头/空/嵌套表标注即手术边界说明；列数 = gridSpan 求和抗整行跨列）⑦ insert_table_after 建表（矩形矩阵 ≤200行×30列；默认表头加粗+tblHeader 跨页重复；100% 宽全边框列均分，列宽按节内容宽派生）⑧ set_cell_text 改格（保 tcPr/首段 pPr/首 run rPr；\n=格内多段；续格拒+指路合并头；嵌套表拒）⑨ insert_table_row_after 克隆增行（整结构克隆含 gridSpan/vMerge——合并格表格唯一正确增行法）。**同块表格批组合**：set_cell_text/insert_table_row_after 同块可多条按序生效（预检做虚拟行模拟——「增行+填新行格」一批完成）；与段落操作/锚互斥；(行,格) 去重。段落操作拒表格块的报错从「不支持」改为指路三件。cargo 1071 / clippy 0 / 词表 0 命中；三份语料表格闭环（untouched 逐字节 + 位移感知断言）。**四波表格格式四件 ✅（2026-08-25 生产驱动 + 用户拍板「四件全做」D11，未手测）**：生产反馈表格**内容**能写了但边框/底纹/字体/样式仍无工具。落地：⑩ 读侧格式可见——模型解析 tblPr/trPr/tcPr 特征（样式/底纹/边框/宽度/vAlign）+ table 投影表属性摘要行与格级标注（(底纹#×)/(垂直=×)/(自定边框)）+ projection=tblpr 三级原文下钻（默认 tblPr / row→trPr / row+cell→tcPr，never write OOXML from memory 的抄写源）⑪ set_table_element 三级通用元素手术（TBLPR_ELEMENTS 17 / TRPR_ELEMENTS 12 / TCPR_ELEMENTS 13 封闭白名单兼 schema 位插入序；gridSpan/hMerge/vMerge 受保护拒改指路 merge_cells/split_cell；容器缺新建/摘空整体清理/自闭合展开；片段校验复用 D9 层）⑫ set_cell_format（ParaFormat+CharFormat 同参面作用于格：段落→格内全部直接段、字符→格内全部 run；入表格批组合（同块 (行,格) 去重））⑬ merge_cells/split_cell（Word 原生语义：纵并 vMerge restart/continue、内容留原格拆分即恢复；横并 gridSpan 求和、内容按序拼进首格、拆分补格继承首段 pPr；网格列区间对齐判据（同显示格号 ≠ 同网格列）；结构重构独占一批（改地址布局，与一切同块操作互斥））。**排障纪要**：xml_grid_span/xml_v_merge 切片原点错位 bug（find_element_span 返回切片内偏移被误用于索引原串——凑巧 tcPr 紧贴开标签时碰巧对，tcW 后跟 vMerge 即读垃圾；修法 = 内层一律用切片变量索引）。cargo 1088 / clippy 0 / 词表 0 命中；语料闭环四操作（set_table_element 幂等逐字节 + merge/split 横纵往返文本守恒/逐字节还原 + untouched）。**五波样式档案与模板个性化·引擎层 ✅（2026-08-25 D12 拍板「引擎通用抽象 + 双轨承载」，未手测）**：⑭ 定义部件手术三操作（def_edit.rs）——create_style 最小出生（type/name/basedOn/qFormat，ID 缺省显示名去空白派生，同批 create→set 可组合：寻址放应用期对累积串解析）/ set_style_element 容器四档（style 直接子级·pPr·rPr·tblPr，STYLE 22/RPR 39/复用 PPR·TBLPR 三白名单兼 schema 位插入序；容器缺新建、摘空清容器、自闭合展开；name 拒摘除=样式身份、pPr 内 rPr 指路容器档）/ set_numbering_element（numId→w:num→abstractNum→w:lvl 解析，LVL 12 白名单；numId 0 拒=显式无编号；lvlOverride 不开刀；共享同一 abstract 的 numId 披露进摘要）。结构寻位 root_children 全元素深度直接子级清单（免疫 tblStylePr 内嵌同名容器误配——前缀碰撞纪律的结构版）；目标定义子树含任一 *Change 修订拒改；latentStyles/docDefaults 永不碰；产物 xml_dom::parse + w:style/w:lvl 计数守恒。⑮ 读侧三投影——styles 清单（ID/显示名/type/basedOn 链/自带特征，200 行顶）+ styledef 原文整段（never-write-from-memory 抄写源，重名显示名拒列全部 ID 指路 ID 寻址）+ numbering 目录（共享披露/正文引用数/逐级摘要，num_id+level 下钻 w:lvl 原文）。⑯ 顺路件——insert_table_after 加 table_style 引用建表（预检存在+type=table）/ clear_body（块数指纹，跳过含 sectPr 块，独占一批）/ merge_cells 矩形区合并（end_row+end_cell 与 span 互斥，逐行横并→结果列纵并两原语组合）。⑰ FamilyOp 分族路由（doc/style/numbering >1 族「部件互斥」拒——同批改定义会使正文预检的 styles 解析态分裂；拆批零成本）。工具 description 增配方两行（隔行底纹/表头高亮组合）+ 模板一行（templates/→copy_file→clear_body→写正文）。cargo 1114 / clippy 0 / 词表 0 命中；语料闭环三测试（styles 手术幂等+其余子级逐字节不变 / numbering 手术 compute_numbers 不变 / *Change 拒改）。**承载层 ✅（批 2，2026-08-25）**：word_style_profile 双轨之一落地——agent.yaml 自由文字块（`set_agent_word_profile` 命令：Some 块写+回读闸 / None·空串块摘除+幂等，BOM/CRLF 兼容）→ hooks 同款纯文件旁路穿透（AgentWithCredentials→AgentTurnInput→PipelineContext，不走 apply_to_row/DB）→ SystemPromptStage 非空注入「## Word 文档样式偏好」小节（原文不解析，委派清单之后）；提案通道纳入 UpdateAgent（`""`=摘除，guard 🟢 Low；schema/description 教 agent 口头偏好→提案）；前端薄接线（bridge.agents.setWordProfile + 审批卡展示项·多行块不入单行编辑·approve 后独立调用）。另一轨 templates/ 目录=约定不创建（无系统写入方，list_directory 自然发现+工具 description 已教 copy_file→clear_body 路径）。**排障纪要**：插入落点须在根闭合标签前（append 串尾会掉到根元素外）+ 自闭合根展开仅在插入路径（空转逐字节不变）+ 去重键用解析后 styleId（显示名与 ID 指同一目标也命中）。后续波：TOC/图片插入/文档级条件批量替换 | 渐进 |
| 远期可选 | S5 脚本化 | Rhai over 文档模型（批量条件操作长尾）；出现高频「迭代式批量变换」场景再上，不预建 | — |

依赖链：1→2 严格串行；3 的手术引擎与 5 共享（生成模板模式复用）。

## 三、决策记录

| # | 决策 | 拍板 | 备注 |
|---|---|---|---|
| D1 | 生成先行 vs 改优先 | **改优先**（用户 2026-08-24） | 手术引擎随步骤 3 落地，生成后补 |
| D2 | 真实语料来源 | **用户提供**：`D:\wcb\test` 真机 Word 产物（三份） | 🚫 严禁版本控制/上传（D7）：文件仅本地 tests/fixtures/docx/（gitignore 排除），corpus 测试运行时读取、缺失自动 skip；WPS 源样本暂缺，待补 |
| D3 | 工具粒度 | **批量事务**（已拍板 2026-08-24，步骤 3 落地形态） | 粗粒度：token 省、轮数少、天然原子；operations 数组 + 整批预检全有或全无 |
| D4 | 模板传递形态 | 对话给路径起步（步骤 6 前终拍） | 零新概念；项目级约定看使用再加 |
| D5 | S3 首波清单 | 样式定义编辑 + 页眉页脚页码（推荐，步骤 3 收官后拍） | 改一次 Heading 1 定义全局生效 |
| D6 | S5 脚本化 | 不预建 | 使用驱动 |
| D7 | 语料保密 | **三份语料严禁版本控制与上传**（用户 2026-08-24 硬禁令） | 语料文件不得出现在任何 commit；**任何来自语料的字符串（含文档标题/正文词/样式名）均不进代码与文档，测试锚点一律结构化**（2026-08-24 二次收紧）；push 冻结已于 2026-08-24 用户明确解除（随 0.5.0 发版推平，历史重写后终验 0 命中）——**语料文件不入库不上传禁令永久有效** |
| D8 | 「可视化」的含义 | **agent 读侧可视化深化**（用户 2026-08-24 澄清） | 用户要的是 agent 把文档结构语义看得更深（自动编号实际值/页眉页脚/分节/TOC），**不是**前端 UI 结果卡——UI 卡不做；对应 S3 首波①④ + 投影补全 |
| D9 | 特殊格式逐个加专用 op vs 通用元素手术层 | **通用优先**（用户 2026-08-24：「我希望往通用型工具，应该优雅设计我们的工具实现」） | 触发：agent 摘自动编号 numPr 无工具可用，暴露专用 op 模式打地鼠的宿命。落地 = set_ppr_element：pPr 子元素 ~34 个封闭 schema 集 → 一个通用 op 永久收敛段落格式长尾，后续 Word 特殊格式不再逐个造轮子。残余专用面收窄且有界：表格单元格属性 / styles.xml·numbering.xml 定义 / 图片 / TOC。**拒绝裸 XML patch**（document.xml 1.4MB 单行 + 无校验 + agent 幻觉 XML = 前科 PowerShell 12 连败重演）；通用 ≠ 裸，是「封闭白名单 + schema 序插入 + 片段校验」 |
| D10 | 表格能力范围 | **四件全做**（用户 2026-08-24 AskUserQuestion 拍板） | 生产实战反馈驱动：交付物文档（功能一览/用例/术语/追踪表）核心内容是表格，agent 无写入途径 = 硬短板。四件 = projection=table 网格投影 + insert_table_after + set_cell_text + insert_table_row_after（native 手术，不走 pandoc/COM 外部工具——违背本地优先与 D9 native-generic 哲学；那是 agent 被逼出来的 workaround 提案，不是产品路径） |
| D11 | 表格格式能力范围 | **四件全做**（用户 2026-08-25 AskUserQuestion 拍板） | 生产反馈驱动：三波补了表格**内容**写入，但边框/底纹/字体/样式仍无工具（set_cell_text 明确不碰格式）。四件 = 读侧格式可见（模型 tblPr/trPr/tcPr 特征解析 + table 投影格式标注 + projection=tblpr 三级原文）+ set_table_element（表/行/格三级通用元素手术，三封闭白名单 TBLPR 17/TRPR 12/TCPR 13 兼当 schema 位插入序，gridSpan/hMerge/vMerge 受保护指路 merge_cells）+ set_cell_format（set_format 的格级版，段落→格内全部段、字符→格内全部 run）+ merge_cells/split_cell（Word 原生语义：纵并 restart/continue 内容留原格拆分即恢复；横并 gridSpan 求和内容按序拼首格；结构重构独占一批）。merge/split 内容策略 = Word 原生（L1 好默认，不另造配置）；insert_table_after 不加格式参数（D9 哲学：默认建好 + set_table_element 后调） |
| D12 | 样式档案与模板个性化 | **引擎通用抽象 + 双轨承载（档案 yaml + 模板目录）**（用户 2026-08-25 拍板） | 用户需求两面：① 工具层尽可能通用抽象（点名「调整自动编号块」——D9 哲学推到 styles.xml/numbering.xml 两个最后的只读部件）② 不同用户有不同 Word 模板需求（表头主题/正文/标题各一套）。核心洞察：「一次定义、处处引用、可统一改」正是 Word 样式系统本职——打通定义部件手术即同时满足两面。引擎层：def_edit.rs 三操作（create_style 最小出生同批可组合 / set_style_element 容器四档 style·pPr·rPr·tblPr 三白名单 STYLE22·RPR39·复用 PPR·TBLPR / set_numbering_element numId→abstract→lvl 解析 LVL12 白名单）+ 三投影（styles 清单带 basedOn 链与自带特征 / styledef 原文整段=never-write-from-memory 抄写源 / numbering 目录+级别下钻）+ 顺路件（insert_table_after.table_style 引用建表 / clear_body 模板复用清场 / merge_cells 矩形区 end_row+end_cell）+ FamilyOp 分族路由（doc/style/numbering 部件互斥——同批改定义会使正文预检的 styles 解析态分裂）。承载层（批 2）：agent.yaml `word_style_profile` 自由文字块注入 system prompt + workspace templates/ 模板目录约定（copy_file→clear_body→写正文），口头偏好走 propose_config_change 提案通道。拒绝 pandoc/COM/外部工具（本地优先 + D9 原生通用哲学） |

## 四、真实语料特征盘点（2026-08-24，步骤 0）

三份均为 **Microsoft Office Word 真机产物**（正式软件工程文档，国标模板风）：

| 特征 | SDP | SRS | INSTALL（2026-08-24 补） |
|---|---|---|---|
| 体量 | 35 页 / 9960 词 / document.xml 1.6MB | 70 页 / 6340 词 / document.xml 1.4MB | 171 块 / 3 节 / 38KB |
| **活动修订** | 无 | 无 | **有（run 级 22 插入 / 16 删除，XML 元素级 w:ins 21/w:del 23）——修订语义真机锚点** |
| 表格 | 41 张 | 21 张 | 修订页 26行×5列 等 |
| 样式表 | 数字 ID，10 处 outlineLvl | 同族模板 | 中文名样式 |

### 前两份语料详表

| 特征 | SDP | SRS |
|---|---|---|
| 体量 | 35 页 / 9960 词 / document.xml 1.6MB | 70 页 / 6340 词 / document.xml 1.4MB |
| **顶层块（S0b 编址口径，sdt 摊平后）** | **1780**（DOM 直接子级 1584 + TOC sdt 内段落） | ~750 |
| 段落（app.xml 统计口径） | 133 段 | 84 段（**表格承重**：70 页仅 84 段） |
| 全部 w:p / w:tbl | 2787 / 41 | 2577 / 21 |
| 图片 | 3 张 PNG | 0 |
| **活动修订 w:ins/w:del** | **0（2026-08-24 更正：初盘点「665/244」系误计——w:instrText/w:insideH 混入；全包无任何修订标记）** | 同左 |
| 域代码 fldChar | 585（TOC/交叉引用） | 204 |
| 超链接 | 194 | 67 |
| 页眉/页脚 | 4 / 5（多节文档） | 4 / 5 |
| numbering.xml | 85KB（重度多级编号） | 74KB |
| 封面标题结构 | 单段 | **同段双 run**（跨 run 切分） |
| 样式表 | 58 个样式（数字 ID '1'-'9'/'a*'，10 处 outlineLvl） | 同族模板 |

**语料意义**：域代码（instrText 1166/TOC）、sdt 包裹目录、多节页眉页脚、表格承重正是手术保真「不敢懂的不碰」的压力面——步骤 3 的字节相等断言直接用这三份文件；SRS 封面标题跨 run 切分是 run 级模型的最小真实案例；INSTALL 补齐**含修订真机样本**（修订语义此前仅合成单测锁定，现 outline 警告头计数 22/16 + format run 级标记 + 零回归逐字节三重锚定）。缺 WPS 源样本（国内兼容性变量），待用户补一份 WPS 产物。🚫 按 D7 硬禁令：文件不入库不上传；代码与文档零语料字符串（corpus 测试锚点全部结构化——规模/块号/首行派生关系/golden 逐字节对比）。

## 五、横切不变式（每步每批 commit 都守）

1. **保真**：编辑只动目标 XML 部件，其余 zip entry 字节相等（不敢懂的不碰）；「解析→重序列化」路线禁用（丢未知部件）
2. **验证后变异**：地址 + 内容指纹核对过才写（edit_file old_string 纪律移植：段落 #N 必须仍以 X 开头，否则整批中止）；整批事务全有或全无
3. **原子写 + 自动备份**：复用 file_tools 备份模式（.tmp → rename）
4. **报错即行为契约**：三段式 + did-you-mean（段落地址错 → suggest 含该文本的段落；与 mcp/path_suggest 同族）
5. **读侧投影分级**：text / outline / format 三档按 token 预算查询，大文档不爆
6. **零回归**：read_file / KB 索引 / 聊天附件三条既有链路的产出在 S0a 重写后逐字节等价（旧测试全绿 + 语料快照对比）。**已知有意例外（S0a 发现并修复的扫描器缺陷）**：旧扫描器把 pPr 内 tab 停靠点**定义**（`<w:tabs><w:tab …/></w:tabs>`，格式元数据）误当内容输出幻影 `\t`（TOC/自定义缩进段落每处停靠点多一个 `\t`）；模型正确不输出。golden 对比前剥离 `<w:tabs>` 定义块对齐两侧语义（`corpus_tests::strip_tab_stops`），测试 `tab_stop_definitions_not_text` 锁定此行为
7. **修订安全**：含活动修订（w:ins/w:del）的文档，编辑操作默认不触碰修订 run；读侧 w:ins 文本计入、w:delText 剔除

## 六、测试基建

- golden corpus = tests/fixtures/docx/（🚫 仅本地放置，gitignore 排除；测试运行时读取、缺失自动 skip——见 D7）+ 合成边缘样本（代码内造 zip，现有模式）
- 每个编辑操作 round-trip 断言：untouched 部件字节相等 + 模型级断言
- 差分器（模型级 diff）= edit_docx 返回值本身，一物两用（agent 读回验证 + 测试断言）
- WPS 双源：待补 WPS 产物后同一套断言跑双份

## 七、界外清单（明确不做）

修订追踪/批注的**编辑**（读侧如实呈现已含）、宏/VBA（安全红线，工具层直接拒绝）、.doc legacy 二进制、Word 图表（依赖 Excel 嵌入，用表格替代）、多人协同、docx 高保真渲染预览（连 LibreOffice 都做不完美，且违背本地优先）。

## 八、延后项（评估结论备查，不开工）

- **Excel 写**：rust_xlsxwriter（Rust 事实标准，纯 Rust）——一个 write_xlsx 工具，几十行
- **Excel 增量改**：umya-spreadsheet（打开-改-保存不丢结构）；Excel 格式操作天然声明式（坐标+属性），可行度较高
- **pptx 读**：同为 ZIP+OOXML，docx.rs 扫描思路可复制
- **渲染预览**：无纯 Rust 可用方案；LibreOffice headless 体积/依赖违背本地优先——工具结果卡「打开文档」按钮（步骤 4）是务实替代
