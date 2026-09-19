# 技术债清扫台账

> **2026-08-16 合并分诊建立。单一真相源**：三轮自查（07-31 / 08-06 / 09-04）+ 各功能记忆「未手测」标记 + 已知架构尾巴。
> 此后新债直接进本表；每条清偿后更新状态，不删除（划掉区防复活）。全部清零后本表降级为「清扫惯例」页。
>
> 图例：🔴 排期中 ｜ 📋 待办 ｜ ❓ 待验证 ｜ 👁 观察池（默认不做，撞上再修）｜ 🗑 已划掉（含原因）
>
> **2026-08-16 分诊经用户确认**。观察池与安全项的「不做」是有据决定，不是遗漏：抽象由测试痛点驱动（A7/A12）、i18n 等真用户出现再做（U16）、竞品有成熟方案的功能先研读再实施（proposal Phase 2）、本地单机安全债按真实威胁模型排期（K1）。重开任一条须先有新证据。

---

## 批次 V — 未手测验证（需要真机，每簇一次 dev 会话）

代码已落地但从未在真机验证过的功能。验证通过即销项，发现问题即转 🔴。

> **2026-09-09 用户拍板：手测积压整体通过**——本批次及各版本 CHANGELOG「真机测试点」全部不再逐项验证，生产使用驱动（用户日常装机使用即验证），出问题由用户反馈、反馈即转 🔴 修复。下表保留作历史台账与回归参照，勿再催验。

| # | 项 | 来源 / commit | 验证要点 |
|---|---|---|---|
| V1 | **token 预算全分层修复**（本次 2026-08-16） | 本会话 | ① 摘要用量遥测行（`ice_paw.summary`：completion≈cap 且字符≈0 = 思考烧光铁证）② 每轮 prompt 从 120K+ 回落 ③ HUD 胶囊/续期 toast ④ AgentForm 高级区改字段 → diff yaml 仅目标行变。**注：HUD 已于 0.4.1（2026-08-22）改环形迁输入框工具栏中间位，观察点随新形态（环形填充/缓存命中 chip/≥80% warn 变色）** |
| V2 | 视觉能力统一适配（4 入口） ✅ 2026-08-22 用户真机手测通过 | bfcd2ce + 2ce76cb + f054e38 + c10d02e | 上传图 / 历史图 / 工具返图 / 附件代读——非视觉模型不得收到 Image |
| V3 | KB watcher 运行时注册 + 自动续写 8 持久化点 ✅ 2026-08-22 用户真机手测通过 | ec08e17 | 新建 agent 拖文件即索引；自动续写各终止路径恢复 |
| V4 | MCP tools/call 120s 超时 ✅ 2026-08-22 用户真机手测通过 | 05d0c14 | 慢视觉调用（5~67s 实况）不再被掐死 |
| V5 | 对话钩子端到端 ❓ 未手测（实际使用尚未用到钩子场景，撞上再验） | 1c2a1d8 | agent.yaml hooks 四接入点真跑一遍 |
| V6 | 轨迹千轮规模 + live 追加 | 4866e01 + aa96e16 系列 | 千轮会话滚动/搜索不卡；生成中实时追加 |
| V7 | **Phase 2A 读路径切换** ✅ 2026-08-17 收官 | 已 commit | dev 正常对话零变化；日志见 `[read_route] → derive (green)`；DevTools `get_read_route_status`。**2026-08-16 真机首证：Derive green（events=891 diffs=0）**。**2026-08-17 观察期满收官**：用户日常使用 2 天无异常 + 日志复核（08-15/16 共 46 次路由决策，事件会话全 Derive green diffs=0；仅有的 Legacy 全为 `no_events` 旧会话，零 diff 回退）——S0 门槛解除 |
| V8 | 孤儿 tool_use 对称清场 | 已合 main | 异常终止路径不留孤儿 tool_use 卡死 |
| V9 | 远程 MCP 传输（streamable HTTP） ✅ 2026-08-22 用户真机手测通过 | 记忆 remote-mcp-transport | 真 HTTP server 握手 + tools/call |
| V10 | **@ 引用三件（@会话/@agent/@消息，2026-08-17）** ✅ 2026-08-22 用户真机手测通过 | 39165d8..04c9c01（4 commits） | ① @ 弹层三段过滤/键盘导航，选会话/agent/消息各成 chip 可删 ② 气泡引用按钮（用户消息单条 / assistant 组整组）一键成 chip ③ 发送后消息内见引用卡片+展开快照进 LLM（模型回答应「知道」被引内容）④ 长会话引用有 `…（中间省略 N 轮）`；引用已删会话 → `[引用已失效]` 不报错 ⑤ 历史卡片点击跳转（会话切换/消息滚动定位）⑥ 轨迹页看该消息事件 payload 含 Reference 块 + 展开 Text，`get_read_route_status` 仍 green（append-only 零特例验证） |
| V11 | **0.4.1 批**（2026-08-22 发版） | 82b5175 | ① 预算 HUD 环形迁输入框工具栏中间位（环形填充/缓存命中 chip/≥80% warn/续期计数）② 默认头像三级链全展示位（委派卡/@ 弹层/概览成员）③ 项目身份减法（头像/主题色移除后各入口无残留）④ 任务面板单侧单列（仅计划或仅任务）⑤ 60s 静默超时双保险（长静默任务不误判 + 发送失败横幅可重试） |
| V12 | **Agent 质量拍工具层批**（2026-08-23，随下版发布） | 61ab9c6 + 后续 | 四件全 Rust 工具层：① write_file `create_dirs` 默认翻 true（治 8 连败族）② 报错行为契约——read_file/list_directory/read_multiple_files not-found 带 did-you-mean 近似候选（`path_suggest.rs`，同名目录/前缀/子串三档打分）、edit_file 未命中带三档诊断（报行号/空白差异/凭记忆）、write/edit 失败带恢复指引、read_file 读目录显式指路（不再裸 os error 5）③ run_command Windows 前置 `chcp 65001`（治中文输出乱码族）④ doom_loop 检测（连败 3 nudge/6 终止）。**验收 = 复跑同款 DB 诊断**：工具失败率 5.7%→<2%，连败 ≥3 链归零；doom_loop 终止词前端有文案+「继续」按钮 |
| V13 | **质量拍⑤ prompt 两层 + 风格预设**（2026-08-23，随下版发布） | b337d2a + 后续 | ① 平台 base prompt 新稿（`system_prompt.rs`）：风格中立三条纪律——错误纪律（读恢复指引/勿原样重试，与①②④咬合）/诚实边界/语言跟随；「与你的人设叠加生效」锚两层关系；意图确认按用户圈改下沉工程档 ② 风格预设三档（工程/创作/陪伴，`data/stylePresets.ts` 前端常量）：素材不是档位——插入 agent.yaml 即用户文本 ③ 新命令 `set_agent_system_prompt`（`patch_agent_yaml_block` 块级补丁+回读闸+原子写；`set_agent_yaml_field` 仍 u64 标量专用）④ 入口（第三轮定稿）：`StylePresetPicker` 居中弹层——胶囊 tab（工程/创作/陪伴）+ 单片全文内容区 + 底部显式确认（浏览与确认分离：点胶囊只切换，create「使用该风格」随保存进 NewAgent / edit「插入到 agent.yaml」+ 覆盖确认横幅（出生默认句豁免））。手测点：**创建** agent → 选工程档 → 保存后 agent.yaml 见工程档全文（未选则通用句）；**编辑** agent → 风格预设 → 插入 → yaml 多行块正确 + 现有注释/预算字段不动；新会话行为贴档（工程档先结论/陪伴档不催建议） |

> 上下文预算 Phase 0+1+2 的手测已并入 V1（摘要链路本次重建）。
>
> **V1 手测（2026-08-16）✅ 结局一（首档即成功）**：cap=4096、completion=1275、产出 808 字符——额度未被思考烧穿，正文正常产出；滚动折叠工作（462→341 条，covered_until_rowid 前进至 1300）；窗口界 160K 生效。用户战略判断：续期仍偏数字游戏 → S8 治本；AgentForm 高级区多余 → P7。
> 0.3.5 发版手测清单六项与 V2-V7 大面积重叠，以本表为准。

## 批次 S — 结构减法（DeepSeek 式简化，测试数不降为硬约束）

**S0（前置门槛）✅ 2026-08-17 通过**: ~~V7 真机持续绿观察 ≥ 一段日常使用期~~ 用户日常 2 天 + 日志全绿复核（见 V7 行）→ **S1 已解锁；旧会话事件 backfill 已落地（2026-08-17，3 commits 00e9cb1..6eed139，848 passed），S1 前置全清，剩真机验收后即可动工**。

| # | 项 | 内容 | 备注 |
|---|---|---|---|
| S1 | **Phase 2B legacy 读路径退役** ✅ 已执行 2026-08-17 | 删 legacy 拼装整条路径 + 摘要锚点 `covered_until_rowid`→seq + Image base64 双份存储治理 | 三件套四 commits（da63c82 阶段1 / 1915dd2 阶段2 / f996543 阶段3a / d5ab926 阶段3b）：①恒走 `load_history_from_events`（resolve 降级健康监控，非绿 error 后照常派生，messages 双写为回滚底座）②migration 46 `covered_until_seq` + 双写过渡（seq 优先 rowid 兜底）③`PayloadBlock` Full/ImageRef 双形态——写侧 `refify_blocks` 三 emitter + backfill（payload 无 base64），读侧 `hydrate_image_refs` 三路水合（derive/reconcile/conversation_cmd JSON 级）+ `to_content_blocks` 防泄漏闸；BACKFILL_VERSION=2 纯 backfill 会话自愈重写。**不变式**：消息类 payload 禁内联 Image base64；新增 message-kind emitter 必经 refify_blocks，读侧必经水合。lib 858 / clippy 0 / 集成 30 / vitest 153。**真机验收 2026-08-17 五项全绿收官**：backfill（boot 行 sessions=9 events=824 failed=0 epoch_rows=0，version=2）+ 恒 Derive（当日路由决策全 green diffs=0，含 backfill 会话续聊 seq 连续）+ 发图 v2 payload 无 base64（2 条 image_ref 各 162B 指针，本体 851KB/3.8MB 只在行；模型回复描述画面=水合进 LLM 视图实证）+ 摘要折叠 covered_until_seq=726/rowid=1710 双值落库 + 轨迹检查器图片显示（用户真机确认 + 服务端响应模拟复证：两形态 4 图零降级，v2 水合还原与行内本体逐字节等长） |
| S2 | protocol.rs 拆分（A5）✅ 已执行 2026-08-16 | 1161 行混 3 类 + 测试（image_validation / LlmProvider 早已迁出）→ `protocol/` 目录：llm.rs（ContentBlock/ChatMessage/TokenUsage/ChatDelta/ToolDef）+ input.rs（前端输入）+ events.rs（事件负载）+ mod.rs glob re-export **全库导入零改**；两条 legacy 兼容 re-export 保留（image_validation 条目、`harness::provider::LlmProvider`） | 32 个协议测试随迁（5+4+6+17）；831 passed 持平 |
| S3 | chat_cmd send_message 收尾（A1）✅ 已执行 2026-08-16 | 695 行中 1-435 行附件机器（2 consts + materialize_file_blocks + should_store_pdf_vision_bytes / pdf_vision_hint / build_modality_hint）整体迁 `harness/attachments.rs`，6 个相关测试随迁；chat_cmd 瘦身至 ~290 行回归纯编排门面（send_message 本体经 MA-1 早已是 ~160 行编排形态）；两处工具 doc 注释路径同步 | 831 passed 持平（测试只迁移不增删）；clippy -D warnings 0 |
| S4 | LoopConfig 数据袋（A6）✅ 已执行 2026-08-16 | ①「不可变配置」声明修真：auth_registry / auth_session 两个运行时可变件（oneshot 通道配对 / 会话级授权累积+收尾 clear）从 LoopConfig 挪进 LoopContext——自有字段优先于 Deref，全库访问点 `ctx.auth_*` 零改；②spawn_stream_loop 26 参数超长签名 → `StreamLoopInput` 结构体成袋（调用方唯一，字段平移零语义），删 #[allow(too_many_arguments)]×2（LoopConfig 上那枚本就无效——struct 字段不触发该 lint，历史残留） | **明确不做**：24 字段全子结构化 + 147 处访问路径改名（ctx.pool 36 / ctx.app 25 / ctx.budget 21 / ctx.conv_id 19 占大头）——纯审美分组，平铺+注释分组可读性已够，review 成本 > 收益，勿复活。831 passed 持平 / clippy -D warnings 0 |
| S5 | send_message 集成测试（A2）✅ 已执行 2026-08-17 | MockProvider 用起来了（不删）：补 `ToolCallThenText` 场景（首调发 tool_use 流 / 次调文本收尾，AtomicU32 计数）+ `harness/session_runner_e2e.rs` 六场景全链路 e2e（正常/空响应/限流退避中取消/显式预算触顶/流中取消占位 discard/工具轮配对），断言四层：消息行（role/content/blocks）+ 事件序（kind 序 + seq 严格连续 + turn_id 一致）+ UI 瞬态事件（CollectEmitter）+ TurnSummary 完成信号 | 地基照抄 session_event_log_e2e（in-memory SQLite + migrate! + 种子）；**须放 src/ 内部**（run_agent_turn 是 pub(crate)）。e2e 顺带钉死两个行为快照：on_loop_exit 恰两次（finalize cleanup + RAII 守卫双保险）、有真实文本时 budget fallback 不抢占 |
| S6 | loop_engine 去 AppHandle 硬依赖（A3）✅ 已执行 2026-08-17 | `loop/emitter.rs`：`LoopEmitter` trait（emit + on_loop_exit）+ 生产 `TauriEmitter`（emit 失败仅 warn + 自带 conv_id 注销 ChatState）+ `emit_ser` 免费函数（trait 对象方法不能泛型的包装）。主循环链七模块换装（context/events/cleanup/stream_consumer/loop_engine/session_runner/retry_round/tool_executor）：`ctx.app` → `ctx.emitter`，唯一 state 用法（RAII unregister）→ on_loop_exit。逃生舱 `tool_app: Option<AppHandle>`——循环链仅剩的 Tauri 句柄，只透传给 ToolContext（proposal/delegate 工具对 None 已有降级报错）。TurnEnv/StreamLoopInput 保留 emitter 边界（chat_cmd/delegate 构造点传 `tauri_emitter(app, conv_id)`）；chat:start 的 `?` 传播降级为 warn（emit 失败不应让已落库回合整体失败）；spawn RAII 守卫复用 on_loop_exit（幂等双保险） | **不变式**：瞬态 UI 进度走 LoopEmitter（失败仅 warn），可回放事实走 event_log——两条通道勿混。`&Arc<dyn LoopEmitter>` → `&dyn LoopEmitter` 不自动链式 coercion，须显式 `.as_ref()`。838 passed（831 基线 + S5 新增 7）/ clippy -D warnings 0 |
| S7 | 废弃字段清理 ✅ 已执行 2026-08-16（tool_trim_threshold 部分） | `tool_trim_threshold` 全链摘除：models.rs（AgentRow/AgentFileConfig/Agent/NewAgent/AgentUpdate + apply_to×2）/ repo/agent.rs（SELECT×2 + INSERT 17 列 + update 签名与 SQL）/ agent_cmd + 5 处测试夹具 / 前端 types ×2 / client.rs 注释措辞。serde 默认忽略未知字段 → 旧 yaml/JSON 负载向后兼容；migration 06 列按「已发布 migration 不可变」纪律保留 | `enabled_tools` 复核为**活字段**（proposal 分级 Medium 依据 + proposal_tool args + yaml 模板写入 + 前端审批流）——只白名单概念死，字段本身保留；`register_meta_tools` 刻意保留（未来白名单 UI 活口）；831 passed 持平 |
| S8 | **无限续写机制（治本）** ✅ 已执行 2026-08-21 随 0.4.0 发布 | 四件全落地：确定性折叠（摘要失败/熔断降级走工具调用骨架，纯本地永不失败）/ 工具结果瘦身（近区超 2000 字符截头尾+指针，可经 @引用 取回）/ 预算 reminder 注入（剩余 <10% 一次性收敛提醒）/ 触顶文本收尾（续期用尽 +4096 收尾额度输出总结自然 stop）。**三观察点转实际使用中验证（用户拍板策略）：预算 90% reminder 后模型收敛 / 断网摘要失败走骨架不失忆 / 大 shell 输出瘦身指针可回溯** | 设计输入存档：docs/competitor-claude-code.md + 四产品全景笔记（codex reminder 注入 / openclaw 压缩三件套 / opencode 仅文本收尾——全部吸收进四件）；反面守则 #1（LLM 依赖必有确定性兜底）即本条产物 |

## 批次 P — 产品打磨

| # | 项 | 备注 |
|---|---|---|
| P1 | 项目 icon / theme_color 无 UI ✅ 已执行 2026-08-19（f62d322） | UI 升级批 C5 闭：ProjectBasicForm「图标与颜色」行（avatar 新列 migration 48 + emoji 沿用 icon 列 + theme_color swatch），展示位 ProjectSwitcher 胶囊/菜单、ProjectList 卡片、ProjectDetailLayout 头部 |
| P2 | 对话重名检查（U13 空值已修） | |
| P3 | ChatHeader 绕 store 直接改标题（U17） | |
| P4 | 侧栏最后消息预览（U10） | |
| P5 | 消息右键菜单（U15） | |
| P6 | 快捷键文档（U8：Ctrl+N/W/K 已实现） | 设置页一屏说明即可 |
| P7 | 移除 AgentForm 高级设置区（回归极简）✅ 已执行 2026-08-16 | UI/bridge/类型已删（`hasWorkspacePath` 保留，工作区按钮仍用）；后端 `get/set_agent_yaml_field` 命令保留（提案系统 / 未来 in-flow 复用），清扫周期结束仍无消费者则一并删。配套：CLAUDE.md 设计规则「配置放置阶梯」+ 记忆 form-minimalism-principle |
| P8 | **权限模式分档**（借鉴拍，加法拍首位候选） | 会话级档位 + agent.yaml `tool_permissions` 规则。**不是新授权系统**——tool_executor 现有 AuthorizationLevel + Once/ThisDir/ThisTool + workspace 校验之上加前置短路层，模式内放行的跳过弹窗，落空走现有对话。配置阶梯对位：模式=L2 会话旋钮、规则=L3。**codex 输入（02 笔记）：两轴正交表述——能力边界（workspace 校验轴）与审批时机（AuthorizationLevel 轴）分开配置，"换审查者不扩边界"为验收不变式；「审批摩擦=安全威胁」为价值论证（摩擦逼用户全开 Always = 我们的 yolo）；全局 Always 档补「不推荐」标注**。**opencode 输入（04 笔记）：①ask 时工具主动建议安全 pattern（`git status*`）让用户一键放行——治审批摩擦的最小 UX 解法②细粒度对象语法——bash 匹配解析后命令、edit 匹配路径、webfetch 匹配 URL：每工具声明自己的鉴权维度而非单一档位；last-match-wins 规则序语义** |
| P9 | **会话分叉 / rewind**（借鉴拍，与 MA-2 同期——共享事件日志地基）📋 2026-08-23 降回待办——用户判定探索式分叉**暂无使用场景**（原 08-22 定主推即撤）；地基（derive 逐字节可靠 / parent_conversation_id 列 / backfill 逆函数机械）不贬值，真有场景随时拾起 | 消息/轨迹视图任意轮「从这里重开」→ 新会话，前缀 `load_history_from_events` 锚 seq 派生落库，`parent_conversation_id`（MA-1 已建列）记 fork 边。CC 快照方案的坑（污染 git 历史/已删文件不恢复）我们从事件派生天然没有；derive 已证逐字节可靠 = rewind 只是它的新消费者 | 设计要点：①fork 边 = `parent_conversation_id` 复用（kind 是否新增 'fork' 待定）②前缀派生锚 seq（S1 阶段 2 的 covered_until_seq 同款锚定）③入口两处（轮次导航条轮菜单 / 轨迹视图行）④新会话标题「原会话名 · 分叉」⑤事件日志零特例——fork 产物是普通新会话，事件从 seq 1 正常写 |
| P10 | 借鉴小项合集 | ① Edit 失败恢复阶梯写进工具 description（唯一匹配失败→扩宽上下文→replace_all 引导，极小）✅ **已被质量拍超越**（2026-08-23）：恢复阶梯直接落进 edit_file 错误文案本体（`edit_mismatch_hint` 三档诊断：报行号/指认空白差异/指认凭记忆拼串），比写 description 更贴模型 ② 辅助任务（摘要/图片代读）默认走 provider 最便宜档，复用 preferences 的 embedding/vision 独立槽位模式（L1 默认，不配不操心）③ 慢工具 5s 进度行阈值 + 进度草稿上限（maxLines=8 / 120 字符行截断）——openclaw 成熟参数包（03 笔记），前端打磨顺手做 ④ doom_loop 同参重复检测 ✅ 2026-08-23 已落地（Agent 质量拍）——`loop/doom_detect.rs` 按「工具名+错误签名（首行冒号前）」跟踪连败：3 次 nudge（tool_result 尾部注入纠正指令 + hook_injected 入事件日志）、6 次终止（finish_reason=doom_loop，finalize_guard 对称清场）；同工具成功即清零。**比原设计更准**：stuck_detect 漏掉的正是一类（换文件名重试同类失败，轮指纹恒变），故按错误家族而非相同输入计数（生产 8 连败案例：8 个不同文件名同一 os error） |
| P11 | **轮次导航条不稳**（用户 2026-08-17 真机报）✅ 2026-08-17 手测通过收官（e345879 + f4753a4 + 376262c + 60a08e8） | 症状三连：①数字不准 ②定位失误 ③不更新。**C1/C2（e345879）**：跳转落点对齐阅读位 + scrollend 漂移纠正 + 删 guard 60 页上限 + 锚点 SQL 改「content 空 ∧ blocks 含 tool_result」合取（Rust 用例钉死三形态）。**视位语义重设计（用户 2026-08-17 拍板，①残留主犯）**：顶线规则「最小相交轮过线→该轮否则-1」系统性少报一——贴底跟随短轮显示 N-1（最高频面孔）、居中阅读显示不在屏上的前一轮。重设计为**底线语义**：输入框上方 24px 一根判定线（`LINE_FROM_BOTTOM_PX`），线落在哪个轮的区域（[锚i, 锚i+1)）就是哪轮，事件无关（IO 相交集实测 gBCR 重算，跳转/翻页/图片加载零逐事件补丁）；**跳转钉子**——点 tick N 落点在视口顶而线在 N+k 区域，纯线判定跳转显得没到位，钉住 N（`pin/clearPin`），用户亲手滚动（滚轮/触摸/滚动键/跳到最新）解钉交还线判定，锚点重载（切会话/新轮）失效。固有限制：多轮同屏读屏顶轮时显示线下轮（单线规则数学下限，方向恒定）。pickActiveTurn 换签名 (intersecting, lineY) 底线判定 + 钉子接线 5 用例。**切会话贴底横杠/错号两轮收口**：长尾轮会话贴底时末轮锚点滚出视口顶 → IO 集空 + 切会话已重置 null →「保持」永远 null = 横杠；376262c 治三件（`root.contains` 剔除换血元素/锚点重载即失忆/空集 bootstrap）。用户复测仍错 → **60a08e8 贴底确定性**收口真根因：完整竞态链=渲染期 scrollTop=0 顶部锚点先进集合定下小轮号 → restore 瞬移贴底 → IO 迟到一拍旧集合 gBCR 现读全在视口上方=线判定错号 → 集空「保持」锁死中毒值（bootstrap 只救 null 不救中毒）。治本：贴底=内容底对齐视口底，线必然在末轮区域——recompute 在集合/上次值之前检查 `scrollTop+clientHeight ≥ scrollHeight-4` 直接末轮，竞态无从产生；bootstrap 只留比例粗估。测试坑：jsdom 全 0 几何下贴底恒真会劫持全部线判定用例，fixture 必须桩 scrollHeight。14 用例。**2026-08-17 用户真机复测通过（切会话多种情况正常）** |
| P12 | **任务胶囊 + 计划面板重设计**（用户 2026-08-17 提出）✅ 2026-08-17 手测通过收官（437a09b + 一轮 8d179fa + 二轮 2145055 + 对齐 df780f5） | 落地：popover 双栏（左任务/右计划）显式定宽；胶囊文案组合化「任务 N · 计划 D/M」+ dot 脉冲扩为任务在跑或计划推进中；状态翻转行背景轻闪 ~1.1s + plan-mark 平滑过渡。**规模治理二轮定稿（用户实测推翻一轮截断+折叠，拍板 880px/58vh）**：平铺优先——弹窗 `max-height:58vh` 挂窗口高（底部恒留 42%+）经 flex 链分到列身；计划列全量平铺（含 done 划线）列内滚动，不折叠（计划协议已封 30，plan_tool MAX_ITEMS）；任务列按高度预算动态截断——预算 = 开面板实测列身高÷行高，超出才收「还有 N 个」计数行（一行让位），running 恒优先；测不到布局平铺兜底（`budgetDoneRows` 纯函数 + prototype 桩单测）。宽度 has-plan 880 / 单列 420（窄窗 wrap 堆叠 + 堆叠态 columns 整体滚动）。胶囊右移对齐用户气泡右缘（`--msg-col-right` 令牌上提 ChatPage 单一真相源），展开不遮轮次导航条。MA-2 台账仍独立页面，胶囊保持轻量索引定位 |

## 批次 Q — 2026-09-04 质量检查（第三轮自查，六路并行扫描）

> 距 08-06 自查近一月、0.5.0→0.6.3 五版功能堆积后的系统性对码。六维：不变式 / panic 面 / 并发生命周期 / 性能+数据完整性 / 视觉规范 / 安全+测试文档。
>
> **干净面结论**：10 条后端系统不变式全成立（enabled_tools 双写 / Image 四门 / 事件日志纪律 / 错误家族前缀 / 工具列表名序 / usage 归一 / 委派三件 / chcp / 屏幕通道 / 旋钮通道）；dispatch_catch_panic 覆盖全部工具执行入口无绕过；数据完整性全绿（migration 不可变 / append-only / 恒 Derive / 三路水合 / backfill 冻结 / messages 双写底座）；密钥卫生 / D7 禁令 / 路径穿越守卫 / 委派预授权边界干净；panic 前提修正（release=unwind，panic≠闪退=task 死亡）；字体本地化 / 品牌色真相源 / Lucide 包名干净；CI d6c8ff0 绿。
>
> **分诊（2026-09-04 用户拍板：Q1-Q9 四组全随 0.6.4 搭车）**：

| # | 项 | 严重度 | 状态 |
|---|---|---|---|
| Q1 | **copy/move destination 越过授权面**：`extract_path_from_args` 只取 source 做白名单，destination 仅靠 reject_sensitive（不含 workspace 边界）——source 在 ws 内即静默 Allow，可零审批写工作区外任意路径（自启动目录等），与 write_file 越界必 Confirm 不一致。修=双路径 Allow 改 all-match（tool_executor.rs:707/141） | 高 | ✅ 7e374e6 |
| Q2 | **doom 错误签名被 AppError 变体前缀稀释**：Validation Display「参数校验失败:」把同变体所有家族折叠成一签名（混家族 6 连败误终止、nudge 指认失真）。修=error_kind 先剥已知变体前缀再截家族（doom_detect.rs:32 + error.rs Display 层） | 中 | ✅ 7e374e6 |
| Q3 | 外部 MCP `send_request` 超时不清 pending 表（挂死 server 上无界小泄漏，external.rs:369+） | 中 | ✅ 7e374e6 |
| Q4 | event_bus 转发任务遇 broadcast Lagged 永久退出（`session:event-appended` 全局停发到重启；有 5s 轮询兜底故降级不断流，lib.rs:321） | 中低 | ✅ 7e374e6 |
| Q5 | TrajectoryView keep-alive 无 onDeactivated/onActivated 成对（缓存态生成期 5s 轮询+监听空转——4ad70ef 同类，useProjectTrajectory 有正确样板） | 中低 | ✅ 40b164e |
| Q6 | **性能家族「同步重活在 async worker」三件**：Word docx zip 读写/重打包（docx_pkg:62 + docx_tool execute）/ 附件物化 base64 解码+PDF·docx 解析（attachments.rs:98-116，发生在 chat:start 前）/ KB indexer 逐文件同步解析（kb/indexer.rs:89+）。对照样板：pdf_render/screen/mcp_cmd 均已 spawn_blocking。0.6.2 卡顿三修的同病灶扩展面。**已修 8f7d0b7**：attachments/indexer/docx 三处包裹；范围披露=inspect_docx/validate_docx 读侧不包裹（审计未点名，读路径无 base64 解码重活） | 中 | ✅ 8f7d0b7 |
| Q7 | **KB 语义检索每次全量加载 KB 全部 chunk**（content+summary+embedding BLOB 无 LIMIT）+ 逐 chunk 解码余弦在 async 线程（kb.rs:378 + kb_tool.rs:210-238）。修=按 kb_id 缓存已解码向量 + indexer 写入失效 + 检索段 spawn_blocking。**已修 8f7d0b7**：失效实装为**四标量签名**（COUNT(\*)/COUNT(embedding)/MAX(rowid)/SUM(LENGTH(content))，一条 GROUP BY 取全）——原设想的「indexer 写入失效」弃用（写点分散漏一处即脏缓存，签名失效漏不掉）；第四标量治 rowid 回收陷阱（SQLite DELETE 后回收 rowid + indexer 预生成回填 → 三标量全复原，单测实测踩中） | 中 | ✅ 8f7d0b7 |
| Q8 | **视觉规范批**：ConfigProposalCard 🟢🟡🔴 敏感度档（规则点名的原型模式）/ 轨迹 💭·↻ 与 KB ✓·✗ 与错误徽章 ⚠（UI 可见 emoji）/ AuthRequestCard·AuthNoticeStack 手写 lock SVG、rail flyout 复制手写 star SVG（新代码应 import @lucide/vue）/ tokens.css prefers 自动暗区镜像不完整（success·warning·danger·info bg/text/border 全族 + accent-agent + primary-rgb，被 useTheme 恒设 data-theme 掩蔽）/ 审批卡三件 spacing 裸 px | 中 | ✅ 40b164e |
| Q9 | 文档勘误三处：CLAUDE.md 计数漂移（cargo 1317→实际 1321、vitest 341→362、「五处 SECURITY」实为 4 处 screen/mod.rs:130/695/737/819）/ computer-use-roadmap「未 push 未手测」标记滞后（已随 0.6.0 push）/ read_route.rs:200 遗留 warn「回退 legacy」与恒派生行为不符（误导排障） | 中低 | ✅ 7e374e6 + docs 批 |
| Q10 | run_command 间接写 agent.yaml × 委派「命令免问」叠加（Confirm 逐次是唯一防线）。**拍板 2026-09-04：接受现状**——run_command 本就是全能力工具、Confirm 是设计防线、免问档是用户自选信任档，不加脆弱命令串嗅探 | 中（设计） | 👁 |
| Q11 | git 工具 extra-args 无敏感路径守卫（Always 级，`git show --output=<任意路径>` 可写文件——内容非受控 YAML 实际影响低） | 中 | 📋 |
| Q12 | update_agent 命令面未物理封死：AgentUpdate 仍含 system_prompt/temperature/max_tokens/enabled_tools，「只管出生证」靠前端调用侧自律——新调用方可绕 yaml 通道重建双真相（repo 层复用必要，仅命令面开放） | 低 | 📋 |
| Q13 | 测试盲区补课：delegate.rs 授权子流程（grant seed/拒绝/超时分支零断言——刚落地的授权面，优先）/ hud.rs（204 行零测试）/ approval_toast.rs（Rust 侧零测试）/ screen/session.rs 仅 1 测试 | 低 | 📋 |
| Q14 | z-index 局部 1-10 裸数字 ~22 处（ChatHeader/ChatMessages/Trajectory* 等）——令牌阶梯无局部堆叠档属规则与阶梯缺口，建议补 `--ip-z-local` 档后批量收编 | 低 | 📋 |
| Q15 | 杂项低危（观察池，撞上再修）：删会话不脱屏幕通道附着（幽灵 HUD 条目）/ deleteConversation 不清 bgStreams·pendingAuth（后端 panic 无终态事件时残留）/ AuthRequestCard 250ms 倒计时 interval 常开 / thinkingDurations 只增不清 / SSE 消费端消失不早退 / gate 取消臂摘排队位后无 bump（HUD 滞后一拍）/ edit_file 写失败裸 Io 少恢复指引（对照 write_file 三段式）/ 11.5px·12.5px 幽灵档 / ✕✓✦ 文本字形 / 动效时长散点硬编码 / MockAgentCmd lock().unwrap() 风格债 / migration 编号跳号（10 从未存在；47 dev 期建删未发布，heal_dropped_migrations 已锁）/ corpus_tests.rs 硬编码语料修订计数（D7 精神边缘）/ 根 package.json 0.3.0 陈旧 / attachments.rs:304·cleanup.rs:151 expect 谓词分离隐性耦合 / docx unreachable·expect 理论面（工具路径内 catch_panic 兜住）/ channel.rs:198 注释漂移 | 低 | 👁 |

## 批次 R — 2026-09-09 健康检查（五路并行扫描：后端不变式 / ModelProfile 线 / 错误路径并发 / 前端 / 文档对账）

> 0.6.15 发版后全工程体检。基准 HEAD = 6c8b716，扫描只读未修。
>
> **干净面结论**：后端十条系统不变式 9 条成立、1 条弱违反（=既有 Q12，非新伤）；ModelProfile 八条不变式 6 条完全成立 + 1 真接缝（R1）；错误路径 P0/P1 **零发现**——非测试代码 unwrap/expect 仅 ~20 处全属内部不变量、await 持锁七处全短临界区、字节切片全有边界守卫；panic 纪律与「为什么敢这么写」论证注释真实生效。fmt 大重排（6c8b716，84 文件）零语义损伤；ModelProfile 穿线未稀释既有不变式（旋钮透传为 ②-1 时即存在的 Q12，穿线机械重构放大可见性）。
>
> **分诊执行（2026-09-09 用户拍板「建议批次全推进」）**：R1-R5 行为修复五件 + R6-R10 健壮性五件 + R11-R13 视觉三件 + R-D1~D7 文档批全部落地（四路并行修复互不碰文件 + 文档批主线；全量验证 cargo 1428 / vitest 464 / typecheck+lint 零警告；**2026-09-09 用户 dev 环境手测通过，四组已 commit**）。R14-R19 留观察池。
>
> 修复要点存档：R1 守卫扩到「行已处引用态 + model_profile_id 缺席 + 快照族手填」也拒；R2/R3 用闭包捕获发送时 convId（防 sendingConvId 易主）+ await 后 activeConvId 守卫；R6 按失败步骤分流（行查询 NotFound 才降级 legacy，fetch_api_key 一切错误=Corrupted 上抛——crypto 对槽位无记录也返回 NotFound，按错误变体分会误归）；R9 Mock 五+六处挂 #[cfg(test)]；R12 ChatHeader z:1 选 badge 而非 base（.chat-render 双 pane 是 absolute，base 会让 header 落败）。

| # | 项 | 严重度 | 状态 |
|---|---|---|---|
| R1 | **提案卡 update_agent 审批绕过 ModelProfile 引用语义**：ConfigProposalCard.vue:152-160 批准时直传 provider/model/base_url 且不带 model_profile_id；后端守卫（agent_cmd.rs:141-163）只拦「同批设引用」组合——引用态 agent + 提案手填快照族放行入库，下轮聊天 profile 解析整体覆盖回 profile 值：**用户批准的换模型静默失效**，设置页先显示「已改」再回翻，agent.yaml 镜像同步还会把永不生效的值写进文件（镜像目的恰是「不误导排障」，agent_cmd.rs:784-796）。proposal_tool.rs:167 schema 仍鼓励模型提案 provider/model（持续生产这类提案）。修=守卫扩到「行已处引用态（old_has_primary）且 input.model_profile_id 非 Some(None) 且快照族手填」也拒（或前端提案卡对引用态 agent 改写字段分派） | 高（行为违背最小惊讶） ✅ 2026-09-09 |
| R2 | **chat store 加载竞态**：loadMessages（stores/chat.ts:139-163）/loadMoreMessages（:165-182）await 前后无请求序号守卫，快速切会话 A→B 时 A 的晚到响应直接覆盖 B 的 messages + hasMore；loadMore 还会把旧会话 older 前插进新会话。修=await 后补 `if (convId !== activeConvId.value) return`（同库范式 useProjectTrajectory.ts:65） | 中 ✅ 2026-09-09 |
| R3 | **发送失败回滚错键 + bgStreams 幽灵锁死**：sendMessage catch（stores/chat.ts:477-495）错误横幅按失败时刻 activeConvId 写（发送在途切到 B → 横幅挂 B 头上）+ 不清理 bgStreams——切回 A 恢复 `sending=true` 幽灵「生成中」而 catch 已 clearSendTimeout，无机制翻转，输入被锁需手点停止。修=回滚/报错一律以 sendingConvId 为键 + catch 里 `bgStreams.delete(sendingConvId)` | 中 ✅ 2026-09-09 |
| R4 | **LogSettings 自动刷新定时器挂满生命周期**：5s setInterval 只在 onUnmounted 清理（LogSettings.vue:108-115），但该页在路由级 keep-alive 下离开只触发 deactivated——开过自动刷新后每 5s 一次隐藏页 invoke + 全量重渲，永不停止。修=照 McpSettings 先例补 onDeactivated 停 / onActivated 按 autoRefresh 重启 | 中 ✅ 2026-09-09 |
| R5 | **useProjectTasks 监听无 keep-alive 静默门控**：两 Tauri 监听（delegation-started 无条件 refresh + turn_ended，useProjectTasks.ts:43-63）在项目详情页被缓存期间照常触发——2026-08-31 修了孪生 useProjectTrajectory、Q5 修了 TrajectoryView，此件漏网。修=同款 listenerLive gate + activated/deactivated 成对 | 中 ✅ 2026-09-09 |
| R6 | **ModelProfile 运行时解析失败静默回落 legacy**：get_with_credentials（agent_cmd.rs:520-527）把「profile 行已删」与「行在但 Stronghold 槽位损坏」混成同一降级分支（warn + legacy_credentials）——后者在迁移型 agent（旧槽位保留）上静默换回旧 Key 继续跑，健康归因记到主档头上（active_profile_id 未变）污染数据；物化型新建 agent 反而诚实报错。修=resolve_profile_credentials 返回结构化错误（NotFound vs Stronghold），仅行缺失走降级 | 中低 ✅ 2026-09-09 |
| R7 | set_mcp_enabled 吞库错（commands/mcp_cmd.rs:159）：`let _ = repo::mcp_server::update` 失败时内存态已翻转、命令返回成功，重启后开关静默回退旧值且无日志 | 低 ✅ 2026-09-09 |
| R8 | checksum 自愈虚报（db/migrate.rs:276,341）：UPDATE 失败仍 healed+=1 打 info「自愈完成」——实际每次启动重复 warn 重试，日志无法发现自愈失败 | 低 ✅ 2026-09-09 |
| R9 | MockAgentCmd / MockModelProfileCmd 未挂 `#[cfg(test)]`（agent_cmd.rs:889 / model_profile_cmd.rs:514——首处 cfg(test) 在 1169/802 行）：测试 mock 连同 `.lock().unwrap()` 编进生产二进制。ModelProfile 穿线遗漏可能性大 | 低 ✅ 2026-09-09 |
| R10 | agent 建库后 KB 初始索引静默跳过（harness/kb/ensure.rs:223-227）：list_by_scope 失败 `.ok()` 后无日志（对比同文件 add_watch 失败有 warn），直到目录文件变更才被 watcher 补上 | 低 ✅ 2026-09-09 |
| R11 | **暗色主题错误横幅刺眼亮红**：`.chat-error-banner`（ChatMessages.vue:1565）裸 `#fef2f2/#fecaca` 覆盖 ErrorBanner 自身 token（ErrorBanner.vue:92）；同文件 `.user-attachment-card`/`.user-ref-card`（:1358-1398）裸 `#ffffff/#1f2937/#f3f4f6` 硬编码浅色 | 低 ✅ 2026-09-09 |
| R12 | 裸 z-index/字号/间距零头（并入 Q14 面）：跳到底按钮 `z-index:5`（ChatMessages.vue:1480——tokens.css:210 注释点名该用 `--ip-z-raised`）；AgentForm.vue:1454 + ModelSettings.vue:1427 tooltip `z-index:10`；ChatHeader :343/:417/:444；12.5px 字号（ChatMessages.vue:1391）+ 5px 间距（AgentForm:1441/ModelSettings:1417） | 低 ✅ 2026-09-09 |
| R13 | 死样式 `.chat-error-icon/.chat-error-text`（ChatMessages.vue:1566-1567，ErrorBanner 化后遗留零引用） | 低 ✅ 2026-09-09 |
| R14 | boot 存量抽离对 Stronghold 瞬时读失败固化（agent_profile_migration.rs:138 `unwrap_or_default()` → 按无 Key 跳过且标记照落，agent 永停留手动形态，仍可编辑转正；测试 :566-589 固化该语义）。收紧=区分 NotFound（终局跳过）与其他 Err（中止下轮重放） | 低 | 👁 |
| R15 | 物化 list→create 非原子（profile_materialize.rs:60-99）：并发同配置双建理论窗口，桌面单用户难触达 | 低 | 👁 |
| R16 | 4 处本地 `chars().take()` 截断器未走 infra/strings（docx_tool.rs:1214 / search.rs:122 / web.rs:89 / screen/channel.rs:809——panic 安全但非共享通道）+ 6 处裸 `AppError::Io` 出工具层（file_tools.rs:430,558-568,627,725-731,790——doom 签名稳定但缺工具名家族词与三段式） | 低 | 👁 |
| R17 | 三处路径字符串拼接混用 `/`（agent_cmd.rs:622 / session_runner.rs:238 / kb/ensure.rs:194，`format!("{}/agents/{}")`）——当前消费方全走 PathBuf 安全，一旦有人拿去与 canonicalize 输出做字符串比较即翻车 | 低 | 👁 |
| R18 | clearActiveConversation 无差别清 bgStreams（stores/chat.ts:760）：生成中切项目空间再切回，流式快照丢失，chunk 重建自愈但 done 前文本从中间开始 | 低 | 👁 |
| R19 | McpSettings onActivated 仅距上次 >30s 才 reload（McpSettings.vue:57-61）：<30s 返回时不补 startPollIfNeeded——离场时有「初始化中…」的 server 状态冻结到手动操作 | 低 | 👁 |
| R-D1 | **文档批·版本号漂移**：仅 tauri.conf.json 升 0.6.15；packages/app/package.json + src-tauri/Cargo.toml 停 0.6.3、根 package.json 停 0.3.0（安装包版本由 tauri.conf 决定，产物无错，纯文档性漂移；根 package.json 陈旧已列 Q15） | 低 ✅ 2026-09-09 |
| R-D2 | **文档批·CHANGELOG 缺档**：最新条目停 0.6.10——缺 0.6.4 与 0.6.15 两档（0.6.11~0.6.14 版本号未使用，按 0.6.10 头注先例标注） | 低 ✅ 2026-09-09 |
| R-D3 | **文档批·CLAUDE.md 当前状态节停在 0.6.10**（影响最大——不变式真相源）：ModelProfile 三阶段 / 降级链三拦截点 / 白屏二轮 / 工具行三件 / 通用页显式保存整批未写，后续会话会绕过已有实现重造 | 中 ✅ 2026-09-09 |
| R-D4 | 文档批·架构树漂移：profile_* 五件（match/materialize/health/agent_profile_migration/legacy_model_migration）+ loop/fallback.rs + doc/ 整目录 + commands 四文件（model_profile_cmd/provider_cmd/screen_cmd/agent_yaml）不在树上；context/ 画在 harness/ 下实为顶层；**loop_engine 697→1377 行**（拆分红利吃回，观察是否再拆） | 低 ✅ 2026-09-09 |
| R-D5 | 文档批·测试数字统一 1427/460（CLAUDE.md 两处矛盾 1350/416 vs 1332/362；CONTRIBUTING「51/712」差 3~9 倍；docs/architecture.md 同旧数且 Pipeline 清单缺 TokenWindow/ModalCapability/ScreenshotHistory 三 Stage、全文未提 session_events/screen/Word/ModelProfile） | 低 ✅ 2026-09-09 |
| R-D6 | 文档批·照做会失败项：CONTRIBUTING 死链（指 memory/cargo-check-env.md，memory/ 目录已空）+ 根目录跑 `pnpm test:watch` Missing script（script 只在 packages/app） | 低 ✅ 2026-09-09 |
| R-D7 | 文档批·computer-use-roadmap checkbox 与正文自相矛盾（§4.12 步骤 1-5 正文全「已落地」、checkbox 全未勾） | 低 ✅ 2026-09-09 |
| R-D8 | 台账销项三件（本轮核实）：Q13-delegate 已补 13 测试（hud/approval_toast/session.rs 仍缺，保留）；CONTRIBUTING「会话导出」已实现（export_session_trajectory，搜索仍未做）；**即时保存负债清两件**——项目背景编辑区已迁草稿+显式保存（ProjectContextEditor.vue:56-74），负债只剩会话标题行内改名（ChatHeader.vue:235）+ 项目成员 chips 两处豁免区 | — | ✅ 核实 |

## 批次 T — 2026-09-10 MA-3 投递 + append-only 事件链路审计（四路扫描 + 生产库取证）

> 0.7 批③落地后全链路体检：投递功能四路（relay 工具壳 / inbox 引擎 / 消费链 / 前端收件箱）+ 事件日志整线（emitters / derive / reconcile / read_route / backfill / refify）+ 周边（60s 超时 / Esc 栈 / 轨迹词表）。总账 52 检查项 = 48 成立 + 2 存疑全闭合（settled actor='user'×by='auto' = 文档化设计非伤；库内 kind 数 vs 文档 14 = 文档漂移）+ 2 漂移；生产库只读取证全绿。
>
> **分诊执行（2026-09-10 用户拍板「逐个修复」）**：行为三件 + 瀑布图词表 + C 级文档漂移五处落地（明细见下表 ✅ 行）；B 级防御性缺口七条 + D 级轻微六条全部观察池——均为有界/低频/诚实边界类，无用户可见行为伤。

| # | 项 | 严重度 | 状态 |
|---|---|---|---|
| T1 | **Esc 栈空转条目吞事件**：useEscapeStack 模块级栈只触发栈顶 + stopImmediatePropagation——「常驻 setup 注册 + 条件浮层」形态（ChatHeader 收件箱/删除确认条、Sidebar 会话 flyout）的空转回调吞掉本该关浮层的 Esc（confirm 注册晚于 inbox 恒踞栈顶 → 收件箱开着 Esc 关不掉）；标题编辑输入框自身 Esc 处理同被吞。修=Entry 增 `active?: () => boolean` 谓词，Esc 自栈顶向下找首个活跃条目；全不活跃不消费事件放行组件自身 | 中 ✅ 2026-09-10 |
| T2 | **sendMessage 在途回合拦截静默吞输入**：sending 全局单份（消费回合 assistant-start 也置位）下 `if (sending) return` 无反馈——用户看「发送没反应」。修=早退写 send_failed 横幅 + lastFailedSend 重试出口（与后端并发拒绝 catch 路径同款），附件 chips 不消费 | 中 ✅ 2026-09-10 |
| T3 | **MA-3 消费回合无 live 流入**：消费回合 chat:start 的 ucb=None → 前端只 push assistant 占位、不带入 incoming user 行——观看中会话来件卡不出现（切走再切回才有）、回复凭空流出。修=外部回合判据（`!ucb && sendingConvId !== cid`）→ sending 即刻置位 + loadMessages 权威刷新（占位由 DB 行带入防重复）；顺手暴露 store 的 sendingConvId / loadMessages（原为内部成员，事件层不可达会 TypeError） | 中 ✅ 2026-09-10 |
| T4 | 瀑布图词表缺 cross 两 kind：TrajectoryTimeline laneOf/KIND_LABELS 未收编 → 落模型道兜底 + 原始 slug 当 label。修=归 User 道（外来输入侧）+ label 镜像 useTrajectory 表格词表（跨会话来件/来件已消费（by 中文）） | 低 ✅ 2026-09-10 |
| T5 | 文档漂移五处：CLAUDE.md 词表 14→18 kind（derive.rs 实证 3+15）+ drain 参数「2s×15s」→「2s 轮询·30s 上限」（inbox.rs:88-89 实值）；event_log.rs:72「全部 13 kind」+ 头部失效指针（migration 44 头注释只记初始集，改指本文件单一真相源）；models.rs default_inbox_policy "hold"→"accept"（对齐 migration 52 拍板默认）+ 两处注释；inbox.rs:399「hold（默认）」注释；reconcile.rs 模块头 skip 清单补 `incomplete_turn_legacy_rows`（代码 :205 在产文档缺席）。migration 52 文件本体刻意不动（已应用库 checksum 风险） | 低 ✅ 2026-09-10 |
| T6 | 回投失败源会话零告知：expect_reply 回投 deliver 失败（目标拒收/队满）时源会话无任何提示——发起方 agent 与用户都以为已回。观察池最值得留意的一条；修法方向=deliver 失败时向源会话 append 一条 message_error 事实 | 中低 | 👁 |
| T7 | deliver 的 cross_session_message append warn-only 失败仍返回 delivered（inbox.rs append_event 影子定位取舍的下游）：工具结果称已投递、事件无记录——pending 判定（NOT EXISTS settled）看不到它，来件成为「幽灵已投递」。Phase 0 影子定位是全局取舍，单点收紧须整线评估 | 低 | 👁 |
| T8 | settled append 失败可二次消费：防双消费靠 settled 占位（chat_state.start 成功后 append），append 失败则批准/自动触发可再次 spawn——概率极低（同库写失败）且 chat_state 二次拒绝兜底 | 低 | 👁 |
| T9 | MAX_PENDING check-then-act 竞态：并发投递同时过上限检查最多放行到 11 条（上限 10），有界无害 | 低 | 👁 |
| T10 | auto 配额空烧：投递即时消费的配额在 spawn 前计数，spawn 前会话忙失败不回滚——窗口内最多烧掉 6/10min 额度，有界 | 低 | 👁 |
| T11 | 消费 spawn 中删会话：spawn 后目标会话被删，回合照跑烧 token、settled 落到已删会话（事件 append 随会话级联删）。桌面单用户低频 | 低 | 👁 |
| T12 | MANUAL_REPLY_GUARD 软上限：内存 Map 无清理，极长会话高频手动回投理论无界（条目 24B 级，实际无害）；重启清零 | 低 | 👁 |
| T13 | boot 竞态 badge 少 1：list_inbox_counts 与首条事件广播的时序缝，popover 打开权威刷新自愈 | 低 | 👁 |
| T14 | 收件箱 popover 开着切会话浮层残留（不跟随会话切换关闭；点外/Esc 可关） | 低 | 👁 |
| T15 | relay.rs unwrap_or_default 吞 DB 错（list_conversations 内 conv 行读取失败静默空串）——展示层降级可接受，缺一条 debug 日志 | 低 | 👁 |
| T16 | count_pending_inbox_all boot 全表 NOT EXISTS 扫描——会话量万级前无感知，届时加 pending 计数列或内存索引 | 低 | 👁 |
| T17 | pending 积压放大 reconcile incomplete_turn 计数：cross:* turn 无 turn_ended 属设计（pending 不是回合），排查对账报告时排除该前缀（reconcile.rs 模块头已补注） | 低 | 👁 |
| T18 | 失焦 OS 通知无补发（通知恰一次语义刻意取舍——错过即看 badge） | 低 | 👁 |

## 批次 U — 0.8.x 版本目标（2026-09-17 四路审计 + 生产取证 + in-app agent 报告交叉）

> 0.8.2 发版后全工程体检，定为 **0.8.x 大版本整线目标**：U0 快修（0.8.3）→ U1 稳定性（0.8.4）→ U2 数据生命周期（0.8.5）→ U3 架构刀与测试补课。输入 = 四路并行审计（后端架构 / 前端架构与状态 / 工程卫生 / UX 十则）+ 生产取证（日志 / DB / 崩溃信号）+ 用户 in-app agent 自查报告交叉核实。
>
> **干净面**：SQL 注入零 / N+1 零 / 事件 inline-await 铁律零违例 / 前端类型卫生优秀（@ts-ignore 0、any 仅 4 处）/ bridge 零绕过 / TODO·console·死代码近零 / 编辑契约四负债已迁二。风险集中在**门禁没接电、少数静默失败、层级倒置与巨件、视觉令牌存量**。
>
> **in-app agent 报告交叉核实（防复活）**：R1 事实过时（41 commit 已随 0.8.2 推平，ahead=0 实证；仅留「批次内多推 checkpoint」惯例）；R2 半错——ChatMessages 有六份专项测试（channel / frozen-round / process-narrative / stats-fastpath / thinking-aggregate / tool-collapse，mount+DOM 断言），「防线完全缺席」不成立，真问题 = 纯函数困在 2324 行 SFC（→U3-3）；**R3 误报驳回**——vue ^3.5.13，且 Vue3 reactive Map.set 对已有键本就触发 SET 键级依赖，生产流式渲染日常在用即反证；R4 部分采纳（核心 = CI 接电 U0-2；coverage 门槛/四元组单人仓过重→观察）；R6 不采纳（localStorage 写频与体量无实害）；R9 半采纳（.gitattributes 已锁源码 LF，editorconfig 只补编辑器侧，无急迫性）；R5/R7/R8/R10 采纳并入下列相应项。生产取证另给观察池补证据：UE5 重连 UX（9-14 单日断线懒重启 83 次实锤）、回合中途预算复查（DB 膨胀同源）。
>
> **2026-09-19 勘误**：本批各行「已修待 commit」均已落盘——U0/U1/U2 随 0.8.3–0.8.6（cf534e9）与 0.9.0（7a8c2d8）提交发版；U2-1「库从 703MB 瘦回 ~90MB」指逻辑内容外置后的瘦身体量，**物理文件未收缩**（2026-09-19 实测 free pages 534.7MB，回收方案见批次 W3）。

| # | 项 | 严重度 | 状态 |
|---|---|---|---|
| U0-1 | **0.8.2 CI clippy 红**：walk_json_for_images String 臂嵌套 if 触发 clippy 1.98 collapsible_match（本地工具链旧未拦）——match guard 收拢已修（2026-09-17 待 commit push）；教训：本地与 CI clippy 版本漂移，考虑 rust-toolchain.toml 钉版或发版清单加「CI 同款 clippy」一步 | 高（main CI 红） | ✅ 已修待 commit |
| U0-2 | **CI 从未跑过前端 673 个 vitest**：ci.yml frontend job 无 test 步骤（脚本已在根 package.json `pnpm -r test`）——一行级改动，80 个测试文件形同虚设 | 高 | ✅ 已修待 commit |
| U0-3 | CI 缺 session_event_log_e2e / session_reconcile_e2e 两集成测试文件（事件日志与对账守门人本身无门禁） | 中 | ✅ 已修待 commit |
| U0-4 | 发版文档三处：CHANGELOG 缺 0.8.2 档 / README 徽章停 0.4.0 / CLAUDE.md 编辑契约负债清单过时（通用页各卡、项目背景实测已迁完，真剩 = 会话标题 + 项目成员 chips）——发版流程补一步检查 | 中低 | ✅ 已修待 commit |
| U0-5 | **展示字体 LXGW WenKai 死资产**：tokens.css --ip-font-display 引用之、public/fonts/wenkai/ ~90 个 @font-face 分片从未被任何文件 import——展示字体真机恒静默回退宋体。一行 import 接电（验首启离线 + 子集体积）或摘除资产改令牌 | 中高 | ✅ 已修待 commit（import 接电）|
| U0-6 | **会话标题改名 = 编辑契约最后真负债**且双违规：blur 即存（ChatHeader.vue:305）+ 失败仅 console 零反馈（:210）→ 迁草稿 + 显式保存 + 失败可见 | 中高 | ✅ 已修待 commit（含 7 用例回归锁）|
| U0-7 | ProjectMembersChips 点击 chip 即持久化（编辑契约另一残留，组件头注释自认）→ 迁显式 | 中低 | ✅ 已修待 commit（v-model 草稿 + 三测试改写）|
| U0-8 | channel_cmd.rs:211 统筹移除后会话改投 `let _ = set_conversation_agent` 静默吞错——频道路由语义错位且无日志 | 中 | ✅ 已修待 commit |
| U0-9 | GroupedSelect 主输入框 outline:none 无焦点替代（键盘可达性实伤，GroupedSelect.vue:253） | 中低 | ✅ 已修待 commit |
| U0-10 | McpSettings 琥珀警告块六色 hex 与 --ip-warning-* 语义层平行（暗色不跟随，McpSettings.vue:587） | 中低 | ✅ 已修待 commit |
| U0-11 | 顺手包：死依赖 ×3（@fontsource-variable/jetbrains-mono / tauri plugin-fs JS 绑定 / plugin-notification JS 绑定）/ 死导出（avatar.ts compressAvatar×2 ~40 行、useResolvedChain）/ 孤儿文件 src/loop/mod.rs（与真实现 harness/loop/ 同名误导）/ useChatEvents 清理链 race（App.vue onMounted await 前卸载 → 监听悬挂）/ useScrollFollow 短 timer 登记 + anchors Map LRU / stopGeneration 乐观翻转 + 静默 catch / agent_cmd update_model_snapshot 两处 `let _ =` / .editorconfig（不含 end_of_line） | 低 | ✅ 已修待 commit |
| U1-1 | **WebView2 创建失败即 panic 崩溃**：9-15/16 生产三次（HRESULT 0x8007139F → tauri app.rs:1425 expect），无重试/降级——先定位失败窗口（主窗 vs HUD 工具栏窗）再加重试 + 可读错误 | 高 | ✅ 已修待 commit（`.run` 包 catch_unwind：可读三段式对话框 + 兜底日志 + 退出码 1；未做进程内重试——0x8007139F=运行时缺失/损坏非瞬态，重试在对话框「怎么办」引导重装） |
| U1-2 | **sendingConvId 漂移可误杀活回合**：bgStreams 恢复只置 sending 不设 sendingConvId（chat.ts:121）+ 后台会话 chat:done 早退不清 + 超时拿陈旧 id 探测已死会话 → 翻 sending → 第二条消息打进运行中回合（R2/R3 修复的残余缺口；先复现再修） | 高 | ✅ 已修待 commit（bg 恢复同步 sendingConvId + bg chat:done 清陈旧 id + 2 用例） |
| U1-3 | **前端核心路径失败静默族**：会话/消息加载失败仅 console → 空白列表无错误态（chat.ts:59/178/205）/ 新建会话失败点击零反馈（useNewConversation.ts:54）/ 项目成员·归档·永久删除失败无提示（ProjectList.vue:230 等 8 处）→ 统一 loadError/banner 模式（GeneralSettings 已是好样板） | 中高 | ✅ 已修待 commit（convLoadError/msgLoadError/loadMoreError + createError + ProjectList actionError 全链路可见） |
| U1-4 | **read_route 非绿：频道会话 reconcile_diffs 未消化**：9-11 密发 45 条 ERROR（频道 19660d0f diffs:4 ×28 + 3d52a0d1 diffs:2 ×17），此后每读必告警「派生历史可能缺行」——跑 reconcile_session 定 diff 类型（疑频道 sender 打标/sweep 改行 vs 事件回放），emitter 修或文档化容忍 | 中高 | ✅ 已修待 commit（election: 投票行按事件 turn 前缀跳过归因，消假 MISSING_IN_DERIVED） |
| U1-5 | loadMoreMessages A→B→A 往返守卫失效（loadingMore 被 loadMessages 重置后旧分页响应放行，R2 残余，chat.ts:186-210） | 中低 | ✅ 已修待 commit（msgEpoch 计数器守卫 + 用例） |
| U2-1 | **DB 膨胀治理**：一周 212MB→737MB（UE5 截图会话期；图片类 tool_result 无保留/压缩/外置策略）+ AppData *.bak 1.6GB 无清理策略——先关窗只读普查表分布再定策 | 中高 | ✅ 已修待 commit：*.bak 清理（`cleanup_stale_db_backups` boot 扫尾，7 天保留）+ **图片外置**（拍板「外置到文件目录」——`infra/image_store.rs` 内容寻址 lossless 外置到 `<data_dir>/images/`，DB 只留 `image_file` 指针、读侧水合回内联；写侧接入 `update_content_blocks`/`cleanup`、读侧接入 repo 全读函数、boot 后台 sweep `offload_all_images`；软失败保留内联不丢字节；6 单测 + 路径逃逸/坏 base64/坏 JSON/去重幂等全锁） |
| U2-2 | 日志体积卫生：「请求携带 N 工具定义」全量工具名列表截断（198 次/日）+ 内置 server stderr 横幅去重（单日 11.9MB 的主要构成） | 低 | ✅ 已修待 commit（openai 工具名截断前 10 名 + `StderrDeduper` 连续重复行折叠） |
| U3-1 | **agent_yaml.rs 六连「同步 IO + 复制粘贴」**：6 个 async 命令内 std::fs read/write/rename（:287 等 12 处跑在 tokio worker）+ read-modify-atomic-write 全套重复五遍——抽公共原子改写 helper 一次治两病 | 中 | ✅ 已修（c2f6b62）：抽 `atomic_write_yaml` 公共原子改写 helper，6 命令去同步 IO + 收敛五遍复制样板 |
| U3-2 | **层次倒置两处**：① harness 反向依赖 commands（channel.rs:60 / inbox.rs:66 / delegate.rs:48 调 commands::model_profile_cmd::production_fallback_plan——fallback 计划应下沉 harness）；② infra/protocol/mod.rs:31 re-export 上游 LlmProvider 成环（trait 应归 protocol）。与 loop 去 AppHandle 化方向相悖 | 中 | ✅ 已修（7895a02 + 9c11f3e）：① fallback 计划下沉 harness（commands 留 re-export 零改动）② LlmProvider trait 归 infra/protocol 破环 |
| U3-3 | **ChatMessages 第一刀**：三胶囊 + 思考聚合 + 工具折叠 + 过程收纳段抽 MessageGroupCapsules 子组件 + 纯函数下沉 utils（约束：六份专项测试断言面零破坏） | 中 | ✅ 已修（c465063）：抽 MessageGroupCapsules 纯展示子组件（三胶囊+思考堆叠）+ transferKey/thinkSegLabel 下沉 utils/groupCollapse.ts；六份专项测试 60 用例零破坏 + 新增 7 纯函数单测 |
| U3-4 | crypto.rs 474 行零测试（XChaCha20-Poly1305 加解密 + blake2b 密钥派生，与 K1 同域）——补单测 | 中 | ✅ 已修（af562d4）：crypto XChaCha20-Poly1305 加解密 + blake2b 密钥派生补单测 |
| U3-5 | 观察升级候选（0.8.x 视余量，否则 0.9）：loop_engine 1431 行持续回涨（R-D4：697→1431 已超拆分前）再拆 / agent_cmd.rs 1841 God module（trait+SQL+DTO+yaml 镜像+频道级联同居）/ chat.ts 三份流式复位清单手工同步 / composables 两对复制（事件接线脚手架、分页三件套）抽象 | 中 | ✅ 已修（12acade + 35987d6 + 43a0ada + a711863）：① chat.ts 流式复位清单收敛单一真相源 ② composables 两对复制抽象（事件接线脚手架 + 分页三件套） ③ agent_cmd 拆 God module 为目录模块（trait/DTO·校验·yaml 镜像·SQL·mock 五面） ④ loop_engine 再拆终止收尾与 doom 扫描（termination + scan_and_nudge） |
| U3-6 | 视觉令牌存量收编（渐进、一次一个组件域防 CSS 回归无测试网）：间距裸 px 646 处 58 文件（布局级 gap≥4px 174 处）/ hex 47 处 19 文件（#fff×16、AttachmentDetail Tailwind 原色、TrajectoryTimeline cssVar 二参回退）/ 非 token 字号 30 处（ErrorBanner 11.5/12.5px 脱档最刺眼）/ ✕✓✦ 文本字形 6 处——Q14/Q15 计数刷新，轨迹族 z-index 8 处聚集地已定位 | 低（体量大） | ✅ 已修（6df33f7 首批切片 + 本批滚动收口）：✕✓✦▾▸ 文本字形全 Lucide 化 + 轨迹族 z-index 8 处 → --ip-z-* + 非 token 字号 → --ip-text-*（仅 markdown.css:47 `font-size:24px` 九档梯度无 24 档，文档化例外保留）+ hex 残留 → 语义/token 色（hljs 主题色、EntityAvatar 哈希色板、paw-brand.svg、inline SVG `#fff` 等文档化例外保留）+ 布局级间距裸 px → --ip-spacing-*（62 文件 ~500 处；光学微调位 2-4px、负值、calc 混合、`var(--ip-spacing-*, Npx)` 回退二参按设计规则保留字面量） |

### U2-1 只读普查结论（2026-09-17 生产库取证）

- **库体积**：`ice-paw.db` 703MB；`messages.content_blocks` 单列 613MB（~87%）是膨胀主体。
- **元凶**：图片类 tool_result 的 base64 **内联**在 `content_blocks` JSON 里（块形 `{"type":"image","data":"<base64>","media_type":"image/png"}`）——767 条消息含图，最大单条 9MB（UE5 截图），violates `message_attachment_files`「大字节外置、messages 不胀」的既有设计原则。
- **`*.bak` 背份**：5 份 = `ice-paw.db.bak`/`.data_bak`（旧 1MB×2）+ `pre-tool-result-migration.bak`（213MB）+ `pre-inbox-drop.bak`（213MB）+ `pre-56-drop.bak`（735MB），合计 ~1.6GB，均迁移安全网快照、从未清理——**已由 `cleanup_stale_db_backups`（7 天保留）收口**。
- **图片外置（2026-09-17 拍板「外置到文件目录」并落地）**：lossless、可逆、DB 瘦身——内联 base64 字节外置到 `<data_dir>/images/<blake2b-16B-hex>.<ext>`，内容寻址去重（同字节同文件），DB 只留 `image_file` 指针。落库形态 `{"type":"image","data":"","media_type":...,"image_file":"<hash>.png"}`（`data` 保留空串防 `ContentBlock::Image` 反序列化失败）；读侧 `hydrate_json` 读回内联、文件缺失降级 `[图片内容已不可恢复]`、路径逃逸判坏降级。存量由 boot 后台 `offload_all_images` 一次性扫尾（`content_blocks LIKE '%"image"%'` 行，UPDATE 只写变化行）。**权衡**：DB 从 703MB 瘦回 ~90MB，字节零损（未选有损压缩——用户优先数据可逆）；写文件失败软失败保留内联、下次 sweep 重试。

## 批次 W — 2026-09-19 大检查（五路扫描 + Steer 批对抗审查）

> 0.9.0 发版后、Steer 批 commit 前的全工程体检（用户委托「代码卫生、架构设计、用户体验等多维度」）：验证链/CI · 后端卫生 · 前端卫生 · UX 契约 · 生产数据五路并行扫描 + Steer 批对抗审查；发现全部对台账 Q/R/T/U 与观察池去重后入本批。
>
> **干净面**：四条全仓级纪律实测零违规（事件 inline-await 铁律 18+ spawn 点逐一核查 / commands 层生产 unwrap·expect 0 / TODO 真实债务全仓仅 1 条 / process::Command·fs 授权面全量核查）；CI 三连绿（7a8c2d8 / cf534e9 / d2073bc）；生产数据五项零异常（损坏行/孤儿事件/reconcile 污染/语义检索索引/会话引用全零）；UX 十则 7/10 完美；前端 console.log 0 / invoke 单点 / localStorage 键值成对。
>
> **分诊（2026-09-19 用户拍板：Steer 批原样 commit 发版，W1 挂账排期）**：

| # | 项 | 严重度 | 状态 |
|---|---|---|---|
| W1 | ~~**Steer 搁浅族**（对抗审查 P1×2 + P2×2）~~ ✅ **2026-09-19 已修**：①积压边界推断 → **marker 记账**（`list_unconsumed_user_anchors` 三腿谓词：真实用户锚点 ∧ user_message 事件在场 ∧ 无 turn_context/turn_ended 标记；账面即真相源，撞忙即放弃不再丢边界）+ `consume_steer_backlog` busy-retry 退避循环（设计稿 §10.3 承诺兑现）；②`stop()` 无回合身份 → `ChatState::token_of` 单快照设计（快照先于物化、点名取消快照令牌；死令牌 cancel 幂等无害 + 兜底 spawn 消费积压）；③无 boot 扫尾 → `spawn_boot_sweep`（lib.rs 3f-b 挂点，`conversations_with_unconsumed_anchors` 圈 1v1 幂等扫尾）；④零后端测试 → stranding 族回归锁（marker 记账全景 ×2 + token_of 回合身份 ×3 + 简报 ×1） | 高 | ✅ |
| W2 | ~~**错误呈现 last-mile**~~ ✅ **2026-09-19 已修（W 批）**：① 裸 `e.message` ×3 → 新增 `utils/errors.ts`（`msgOf`/`stripInvokePrefix` 剥 invoke 层 `[op] 内部错误:` 壳 + `friendlyError` chat:error provider 原文中文映射）；② 16+ 手写错误横幅收编 `ErrorBanner variant="inline"`（Sidebar 频道开启失败 / ProjectList·ProjectSettings 保存失败 / ModelSettings 创建失败等）；③ 8 处手写 click-outside 抽共享 `useClickOutside`（7 组件消费）；4 测试选择器随迁 `.eb-inline` | 中高 | ✅ |
| W3 | ~~**DB 空间 534.7MB 未回收**~~ ✅ **2026-09-19 已修（W 批）**：`db/space.rs` boot 后台空间回收扫尾（延迟 30s 避开 boot 写峰）——PRAGMA 三读页统计 + **双阈值**判定（free ≥64MB 且占比 ≥25% 才值得整库重写）才跑一次性 VACUUM（busy_timeout 15s 防写峰 SQLITE_BUSY 假失败 + 完成后防御性重设 WAL）；全失败路径仅 warn，VACUUM 幂等下次启动重试；正常库零命中零成本。5 单测（生产实证形态 / 双阈值双向不足跳过 / 零页防御 / 边界 ≥ 含等号） | 中 | ✅ |
| W4 | ~~**轨迹时间格式化 4 处绕过统一通道**~~ ✅ **2026-09-19 已修（W 批）**：4 处（TrajectoryTable/TrajectoryTimeline/TrajectoryInspector/useTrajectory localDate）统一走 `utils/time.ts formatDateLabel`（今天/昨天/M月D日，用户偏好时区单点解析） | 中低 | ✅ |
| W5 | ~~**失败静默族残余**~~ ✅ **2026-09-19 已修（W 批）**：batch_writer sender-drop 退出路径 flush 失败补 warn（对齐显式关闭路径）/ channel.rs 选举换选旧统筹降级失败不再吞（可短暂双 coordinator 面收口）/ kb/ensure.rs 种子写失败如实披露不再无条件「已创建」/ chat.ts 延迟删除失败可见（steer.rs:189 项已随 W1 批 marker 记账消除） | 中低 | ✅ |
| W6 | ~~**杂项包**~~ ✅ **2026-09-19 已修（W 批）**：z-index 裸数字 7 处 → `--ip-z-*`；ProjectSwitcher 焦点环 + TrajectoryTimeline `.tt-earlier` 焦点替代；file_tools MoveFileTool 注释勘误（Q1 修复前授权语义的安全误导）+ channel.rs 旧契约注释清理；**Steer P3 批**（steer/频道插话失败写 `lastFailedSend` 横幅带重试出口 / `steerAbortExpected` 任何 chat:done 无条件消费 + `stopGeneration` 清预告防手动停止被误消音成「插话打断」/ `pruneQueuedSteers` 只摘最旧命中一条对齐后端 FIFO 接手 / 回合 B 简报套话防进相关性检索——`session_runner.rs StreamLoopInput.relevance_query` 检索 query 用用户原话，steer.rs/channel.rs 传值）；ChatMessages·GeneralSettings 8 处 setTimeout 卸载清理 + onUnmounted 补齐；useModelProfiles test-only exports 收编；mm:ss 重复实现抽 `utils/time.ts formatMmSs` 共享 | 低 | ✅ |
| W7 | ~~**文案规范尾巴**~~ ✅ **2026-09-19 已修（W 批）**：formatThinkingMs 数字与单位间空格（「30 s」，复合串内部紧凑「1m 30s」）+ TrajectoryTable 毫秒形态对齐；绝对时直出 3 处（TaskLedger/TaskPanel/ProjectList）改 timeAgo + hover 绝对时 | 低 | ✅ |

## 安全项

| # | 项 | 备注 |
|---|---|---|
| K1 | Stronghold 弱密钥派生（硬编码口令 + 无盐 blake2b） | 本地单机攻击面有限；Phase 2 接 OS keyring。登记不排期 |

## 👁 观察池（默认不做，撞上再修）

- U16 i18n 框架 —— UI 已统一中文，无多语言需求前不做
- U19 userMsg rowid:0 —— 良性
- A7/A12 其他域 trait 化 —— 按测试痛点驱动，不做一次性大抽象（过度抽象本身就是债）
- A9 加命令改 3-4 文件 —— DX 小痛，可脚本化不值得抽象层
- KB 行 directory 不可变 —— workspace 迁移已知局限，agent_cmd 有注释
- 远程 MCP SSE —— 功能路线图项
- 测试覆盖率 ~3% —— 随 S5 与 e2e 渐进，不单独立项
- proposal Phase 2（MCP 域）—— 功能延伸，竞品研读后再定
- **并发工具分组**（借鉴拍）—— 一轮多 tool_use 独立工具并行执行。改造风险最高，动前先出设计小稿：① 需授权弹窗的工具一律降级串行（并行弹窗互踩）② 事件 inline await 纪律下执行可并行但 append_event 汇聚点须串行化 ③ stuck_detect/budget 的每轮边界假设串行需复核。CC 佐证收益：两层并发（并发组+串行组）是长任务延迟主力
- 会话/轨迹导出 Claude Code 兼容格式（借鉴拍，openclaw 03）—— OpenClaw「一键导入 CC 记忆」证明可迁移性是获客杠杆；我们反向做导出兼容即降低用户迁出顾虑。export_session_trajectory 加目标格式适配即可，撞上再做
- skill 渐进披露（借鉴拍，opencode 04）—— name+description 常驻工具清单、正文按需注入；我们工具软裁剪已做相关性排序，此模式可延伸到 KB/help 注入（目录层），撞上再做
- BeforeCompact hook（借鉴拍，opencode 04）—— opencode 压缩提示词可被插件整体替换；我们 hooks 四接入点可远期加第 5 个，压缩策略用户可编程
- **回合中途预算复查**（2026-09-12 UE5 base64 巨块案二阶）——工具输出全局治理已治「单结果体积」（提取+256KB 截断，tool_executor 单点）；更深一层是回合内累积无闸：多轮工具结果合计仍可超窗口（Pipeline 预算/TokenWindow 只在回合起点跑一次）。修法 = loop 每轮 LLM 调用前重估工作上下文、超限触发轮间折叠——动 loop_engine 核心，等真实撞上（单结果闸后概率已大降）再立项
- **docx 四件套 ~16,250 行成最大质量块**（2026-09-19 大检查量化刷新；docx_edit.rs 8456 全仓最大、docx_tool 3480、docx_inspect 2607）——巨件方向拆分候选，随下次 docx 波次顺势切片
- **send_message 回涨 255 行**（chat_cmd.rs；S3 曾瘦至 ~160，channel/steer 双分流叠加附件编排所致）——编排下沉候选；会话存在性校验样板 ×11 同向
- **AppHandle 穿透 kb/migration 旁支未收口**（U3-2 loop/emitter 抽象方向延续，主体已破环、旁支残留）
- **reject_sensitive 无 Windows 设备名/UNC 纵深**（file_tools.rs:41——防御纵深单层；主闸 PathWhitelist/Q1 all-match 无实际旁路，Q1 同域）
- **ChatMessages 2278 行二刀候选**（U3-3 第一刀后仍为前端最大件；全前端 22 文件 >500 行）

## 🗑 划掉区（已核验消亡，勿复活）

- **derive skip 臂漏 `model_switch`** → 既有 bug（不在任何批次条目——批次 R 扫描漏项，MA-3 探查时实锤：`session_runner_e2e.rs` 已在生产写入该 kind，derive 遇未列 kind 记 DeriveIssue 污染对账报告）。2026-09-09 随 MA-3 Commit 1（cacaa70）修复：skip 臂收编 `model_switch` + 新 `cross_session_message`/`cross_session_message_settled` 两 kind。
- **A4 测试跑不起来（sodium DLL）** → 2026-08-13 钉真根因 = lib test harness 缺 comctl32 v6 manifest，build.rs 已修；`cargo test --lib` 831 passed。**可测试性三连的门控钥匙已开。**
- **`.cargo/config.toml` 机器绝对路径** → 文件已不存在（2026-08-16 核验）。
- **08-06 自查 31 项 FIXED** → 2026-08-08 四 agent 并行逐条对 HEAD 复核结清（R1-R9 / F1-F6 / U1-U4 等），明细见记忆 audit-2026-08-06，不再重列。
- **07-31 自查已修项** → P0 三连（会话卡死 / env 泄漏 / 资源清理）+ quick-win 行级修复，2026-08-02 结清。
- 上下文预算 Phase 0+1+2 手测 → 并入 V1。

---

## 清扫循环（方向一工作法）

**清扫 → 借鉴（竞品研读，只积累不实施）→ 加法 → 再清扫。** 每拍有 Done 标准：
- 清扫拍：本表批次清零（V/S/P 逐批销项）
- 借鉴拍：每产品一份**全景**四问笔记（它解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本）——**范围 = 整产品**（架构设计、技术实现、设计理念、各关键功能的特点与优势），不是单痛点切片（2026-08-16 用户纠偏）。已产出：claude code **压缩续跑切片**（docs/competitor-claude-code.md，喂 S8）+ **全景笔记**（docs/competitor-claude-code-panorama.md，2026-08-16：已对齐 11 项 / 要借鉴 7 项【权限分档·并发工具组·rewind 入口最值】/ 结构性更强 3 项 / 不借鉴 5 项 + 未证实存疑清单）；**codex 全景笔记**（docs/competitor-codex-panorama.md，2026-08-16：已对齐 8 项【append-only 日志·深度=1·标记式 rollback——与 CC 交叉验证】/ 要借鉴 6 项【两轴正交权限喂 P8·reminder 注入喂 S8·auto-review 远期】/ 结构性更强 4 项 / 不借鉴 4 项 + 存疑清单）；**codex 增量研读 2026-08-23**（harness 全面开源后源码级，笔记末节）：WorldState 差分注入 / goals continuation 提示词 / guardian 全链 / 「失败=结构化输出+行为指令」错误哲学——落点喂 Agent 质量拍·P8·S8；**openclaw 全景笔记**（docs/competitor-openclaw-panorama.md，2026-08-16：已对齐 7 项【配置哲学·pruning/compaction 分离·拒绝闭环——第三家同构】/ 要借鉴 5 项【压缩三件套喂 S8·持久写者 fencing 喂 MA-3·慢工具 5s 进度行】/ 结构性更强 4 项 / 不借鉴 5 项【无事件日志反证我们差异化·文本协议债】+ 存疑清单）；**opencode 全景笔记**（docs/competitor-opencode-panorama.md，2026-08-16：已对齐 8 项【subagent_depth=1 第三家同构·系统任务也是 agent·UX 优先哲学】/ 要借鉴 6 项【仅文本收尾喂 S8·doom_loop 同参信号喂 stuck_detect·建议 pattern+细粒度对象语法喂 P8·skill 渐进披露】/ 结构性更强 3 项 / 不借鉴 5 项【无事件日志第四家反证·快照式回退第二证·文本解析 allowlist】+ 存疑清单）。**借鉴拍收官：dsh / claude code / codex / openclaw / opencode 五产品齐，候选清零**
- 简化拍约束：测试数不降、clippy 零警告、行为零变化（重构与功能改动不同 commit）

### 借鉴拍反面守则（2026-08-16，claude code 踩坑 → 我们的 设计守则）

1. **LLM 依赖必有确定性兜底**——它压缩熔断后无回退会话搁浅（3272 连败事故）；摘要/代读/自愈类设计都要带「失败也能走」降级路径（S8 即此守则产物）
2. **会话单一写者**——它同会话双终端 resume 消息交错；MA-3 持久通道 / 远程接入前的第一条不变式。openclaw 给出持久化样板（03 笔记）：`activeWriterRunId` 持久领取 + `expectedWriterRunId` fencing 在同步 commit 事务里校验
3. **隔离必须可降级**——它 worktree 隔离对非 git 目录近乎强制；将来多 agent 并行写文件，隔离失败应退化成排队而非拒绝工作
4. **快照式回退天生脏**——它 checkpoint 污染 git 历史、已删文件不恢复；opencode undo/redo 靠内部 git 仓库（要求 git repo、大仓库要关）是第二证；反证我们 rewind 走事件派生路线
5. **钩子否决须防自旋**——它 Stop hook 可拦停致 #55754 死循环；hooks 若扩展否决语义必须配重触发上限
6. **按需加载要进管道不要事后补**——它 skills 渐进加载设计了没落地（#16160）；工具描述/KB 提示瘦身走管道 Stage（S8 L0 思路）
7. **多真相源必有对账债**——codex 的 JSONL rollout + SQLite 索引双侧不一致（#21196/#29083）+ compacted 检查点膨胀到单文件 45%；反证我们「单一 SQLite + 事件日志同库」决策，Phase 2B 摘要锚治理时勿把摘要态混进行表（2026-08-16，codex 研读产出）

### 远期构想存档：graph 工作台（2026-08-16 用户头脑风暴，只存不动）

方向共鸣：并发工具 → agent 图协作演进（CC agent teams / 业界 graph 编排同潮）。**设计立场（克制条款）**：
- **图从事件日志长出来，不引入声明式 DSL**——愿景已锁「agent 间是图可直连」，MA-1 的 parent_conversation_id 边就是图的第一个物化；DSL 路线（先画图再执行）要自带状态机+重放，与「真相在产物/日志无损」重复且冲突
- **项目概览/监控 = session_events 的纯派生投影**（delegation/plan_updated 事件聚合），不是新存储——这是 MA-2 任务台账+项目轨迹的本职，空白真实存在，CC 共享任务清单验证了需求
- **工作台调度前置 = 并行委派 + 单一写者 + 任务台账**；loop 内并发工具（观察池）与 agent 间图是正交两层，不混
- 演进序列：MA-2（台账+轨迹=概览）→ 并行委派 → 图视图基本只剩渲染层；rewind/fork（P9）给树编辑能力
