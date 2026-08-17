# 技术债清扫台账

> **2026-08-16 合并分诊建立。单一真相源**：两轮自查（07-31 / 08-06）+ 各功能记忆「未手测」标记 + 已知架构尾巴。
> 此后新债直接进本表；每条清偿后更新状态，不删除（划掉区防复活）。全部清零后本表降级为「清扫惯例」页。
>
> 图例：🔴 排期中 ｜ 📋 待办 ｜ ❓ 待验证 ｜ 👁 观察池（默认不做，撞上再修）｜ 🗑 已划掉（含原因）
>
> **2026-08-16 分诊经用户确认**。观察池与安全项的「不做」是有据决定，不是遗漏：抽象由测试痛点驱动（A7/A12）、i18n 等真用户出现再做（U16）、竞品有成熟方案的功能先研读再实施（proposal Phase 2）、本地单机安全债按真实威胁模型排期（K1）。重开任一条须先有新证据。

---

## 批次 V — 未手测验证（需要真机，每簇一次 dev 会话）

代码已落地但从未在真机验证过的功能。验证通过即销项，发现问题即转 🔴。

| # | 项 | 来源 / commit | 验证要点 |
|---|---|---|---|
| V1 | **token 预算全分层修复**（本次 2026-08-16） | 本会话 | ① 摘要用量遥测行（`ice_paw.summary`：completion≈cap 且字符≈0 = 思考烧光铁证）② 每轮 prompt 从 120K+ 回落 ③ HUD 胶囊/续期 toast ④ AgentForm 高级区改字段 → diff yaml 仅目标行变 |
| V2 | 视觉能力统一适配（4 入口） | bfcd2ce + 2ce76cb + f054e38 + c10d02e | 上传图 / 历史图 / 工具返图 / 附件代读——非视觉模型不得收到 Image |
| V3 | KB watcher 运行时注册 + 自动续写 8 持久化点 | ec08e17 | 新建 agent 拖文件即索引；自动续写各终止路径恢复 |
| V4 | MCP tools/call 120s 超时 | 05d0c14 | 慢视觉调用（5~67s 实况）不再被掐死 |
| V5 | 对话钩子端到端 | 1c2a1d8 | agent.yaml hooks 四接入点真跑一遍 |
| V6 | 轨迹千轮规模 + live 追加 | 4866e01 + aa96e16 系列 | 千轮会话滚动/搜索不卡；生成中实时追加 |
| V7 | **Phase 2A 读路径切换** ✅ 2026-08-17 收官 | 已 commit | dev 正常对话零变化；日志见 `[read_route] → derive (green)`；DevTools `get_read_route_status`。**2026-08-16 真机首证：Derive green（events=891 diffs=0）**。**2026-08-17 观察期满收官**：用户日常使用 2 天无异常 + 日志复核（08-15/16 共 46 次路由决策，事件会话全 Derive green diffs=0；仅有的 Legacy 全为 `no_events` 旧会话，零 diff 回退）——S0 门槛解除 |
| V8 | 孤儿 tool_use 对称清场 | 已合 main | 异常终止路径不留孤儿 tool_use 卡死 |
| V9 | 远程 MCP 传输（streamable HTTP） | 记忆 remote-mcp-transport | 真 HTTP server 握手 + tools/call |

> 上下文预算 Phase 0+1+2 的手测已并入 V1（摘要链路本次重建）。
>
> **V1 手测（2026-08-16）✅ 结局一（首档即成功）**：cap=4096、completion=1275、产出 808 字符——额度未被思考烧穿，正文正常产出；滚动折叠工作（462→341 条，covered_until_rowid 前进至 1300）；窗口界 160K 生效。用户战略判断：续期仍偏数字游戏 → S8 治本；AgentForm 高级区多余 → P7。
> 0.3.5 发版手测清单六项与 V2-V7 大面积重叠，以本表为准。

## 批次 S — 结构减法（DeepSeek 式简化，测试数不降为硬约束）

**S0（前置门槛）✅ 2026-08-17 通过**: ~~V7 真机持续绿观察 ≥ 一段日常使用期~~ 用户日常 2 天 + 日志全绿复核（见 V7 行）→ **S1 已解锁；旧会话事件 backfill 已落地（2026-08-17，3 commits 00e9cb1..6eed139，848 passed），S1 前置全清，剩真机验收后即可动工**。

| # | 项 | 内容 | 备注 |
|---|---|---|---|
| S1 | **Phase 2B legacy 读路径退役** ✅ 已执行 2026-08-17 | 删 legacy 拼装整条路径 + 摘要锚点 `covered_until_rowid`→seq + Image base64 双份存储治理 | 三件套四 commits（da63c82 阶段1 / 1915dd2 阶段2 / f996543 阶段3a / d5ab926 阶段3b）：①恒走 `load_history_from_events`（resolve 降级健康监控，非绿 error 后照常派生，messages 双写为回滚底座）②migration 46 `covered_until_seq` + 双写过渡（seq 优先 rowid 兜底）③`PayloadBlock` Full/ImageRef 双形态——写侧 `refify_blocks` 三 emitter + backfill（payload 无 base64），读侧 `hydrate_image_refs` 三路水合（derive/reconcile/conversation_cmd JSON 级）+ `to_content_blocks` 防泄漏闸；BACKFILL_VERSION=2 纯 backfill 会话自愈重写。**不变式**：消息类 payload 禁内联 Image base64；新增 message-kind emitter 必经 refify_blocks，读侧必经水合。lib 858 / clippy 0 / 集成 30 / vitest 153。真机手测四项待验（旧会话续聊/含图轨迹/新图会话 payload/covered_until_seq 落值） |
| S2 | protocol.rs 拆分（A5）✅ 已执行 2026-08-16 | 1161 行混 3 类 + 测试（image_validation / LlmProvider 早已迁出）→ `protocol/` 目录：llm.rs（ContentBlock/ChatMessage/TokenUsage/ChatDelta/ToolDef）+ input.rs（前端输入）+ events.rs（事件负载）+ mod.rs glob re-export **全库导入零改**；两条 legacy 兼容 re-export 保留（image_validation 条目、`harness::provider::LlmProvider`） | 32 个协议测试随迁（5+4+6+17）；831 passed 持平 |
| S3 | chat_cmd send_message 收尾（A1）✅ 已执行 2026-08-16 | 695 行中 1-435 行附件机器（2 consts + materialize_file_blocks + should_store_pdf_vision_bytes / pdf_vision_hint / build_modality_hint）整体迁 `harness/attachments.rs`，6 个相关测试随迁；chat_cmd 瘦身至 ~290 行回归纯编排门面（send_message 本体经 MA-1 早已是 ~160 行编排形态）；两处工具 doc 注释路径同步 | 831 passed 持平（测试只迁移不增删）；clippy -D warnings 0 |
| S4 | LoopConfig 数据袋（A6）✅ 已执行 2026-08-16 | ①「不可变配置」声明修真：auth_registry / auth_session 两个运行时可变件（oneshot 通道配对 / 会话级授权累积+收尾 clear）从 LoopConfig 挪进 LoopContext——自有字段优先于 Deref，全库访问点 `ctx.auth_*` 零改；②spawn_stream_loop 26 参数超长签名 → `StreamLoopInput` 结构体成袋（调用方唯一，字段平移零语义），删 #[allow(too_many_arguments)]×2（LoopConfig 上那枚本就无效——struct 字段不触发该 lint，历史残留） | **明确不做**：24 字段全子结构化 + 147 处访问路径改名（ctx.pool 36 / ctx.app 25 / ctx.budget 21 / ctx.conv_id 19 占大头）——纯审美分组，平铺+注释分组可读性已够，review 成本 > 收益，勿复活。831 passed 持平 / clippy -D warnings 0 |
| S5 | send_message 集成测试（A2）✅ 已执行 2026-08-17 | MockProvider 用起来了（不删）：补 `ToolCallThenText` 场景（首调发 tool_use 流 / 次调文本收尾，AtomicU32 计数）+ `harness/session_runner_e2e.rs` 六场景全链路 e2e（正常/空响应/限流退避中取消/显式预算触顶/流中取消占位 discard/工具轮配对），断言四层：消息行（role/content/blocks）+ 事件序（kind 序 + seq 严格连续 + turn_id 一致）+ UI 瞬态事件（CollectEmitter）+ TurnSummary 完成信号 | 地基照抄 session_event_log_e2e（in-memory SQLite + migrate! + 种子）；**须放 src/ 内部**（run_agent_turn 是 pub(crate)）。e2e 顺带钉死两个行为快照：on_loop_exit 恰两次（finalize cleanup + RAII 守卫双保险）、有真实文本时 budget fallback 不抢占 |
| S6 | loop_engine 去 AppHandle 硬依赖（A3）✅ 已执行 2026-08-17 | `loop/emitter.rs`：`LoopEmitter` trait（emit + on_loop_exit）+ 生产 `TauriEmitter`（emit 失败仅 warn + 自带 conv_id 注销 ChatState）+ `emit_ser` 免费函数（trait 对象方法不能泛型的包装）。主循环链七模块换装（context/events/cleanup/stream_consumer/loop_engine/session_runner/retry_round/tool_executor）：`ctx.app` → `ctx.emitter`，唯一 state 用法（RAII unregister）→ on_loop_exit。逃生舱 `tool_app: Option<AppHandle>`——循环链仅剩的 Tauri 句柄，只透传给 ToolContext（proposal/delegate 工具对 None 已有降级报错）。TurnEnv/StreamLoopInput 保留 emitter 边界（chat_cmd/delegate 构造点传 `tauri_emitter(app, conv_id)`）；chat:start 的 `?` 传播降级为 warn（emit 失败不应让已落库回合整体失败）；spawn RAII 守卫复用 on_loop_exit（幂等双保险） | **不变式**：瞬态 UI 进度走 LoopEmitter（失败仅 warn），可回放事实走 event_log——两条通道勿混。`&Arc<dyn LoopEmitter>` → `&dyn LoopEmitter` 不自动链式 coercion，须显式 `.as_ref()`。838 passed（831 基线 + S5 新增 7）/ clippy -D warnings 0 |
| S7 | 废弃字段清理 ✅ 已执行 2026-08-16（tool_trim_threshold 部分） | `tool_trim_threshold` 全链摘除：models.rs（AgentRow/AgentFileConfig/Agent/NewAgent/AgentUpdate + apply_to×2）/ repo/agent.rs（SELECT×2 + INSERT 17 列 + update 签名与 SQL）/ agent_cmd + 5 处测试夹具 / 前端 types ×2 / client.rs 注释措辞。serde 默认忽略未知字段 → 旧 yaml/JSON 负载向后兼容；migration 06 列按「已发布 migration 不可变」纪律保留 | `enabled_tools` 复核为**活字段**（proposal 分级 Medium 依据 + proposal_tool args + yaml 模板写入 + 前端审批流）——只白名单概念死，字段本身保留；`register_meta_tools` 刻意保留（未来白名单 UI 活口）；831 passed 持平 |
| S8 | **无限续写机制（治本）** | 摘要失败回退链：自适应额度（已有）→ **确定性折叠**（保 system + 首末 N 轮原文，中段压成工具调用骨架，不依赖 LLM 永不失败）；终止语义重排：stuck 无进展为唯一常规终止，budget 降格为失控保护（本地模型 = 真·无限，付费 API = 成本上限） | **设计输入已备：docs/competitor-claude-code.md（借鉴拍第一份）**——补两个新输入：历史工具结果瘦身（预览+指针）排最前、摘要请求自身截头防超长；Claude Code 熔断后无回退会搁浅，我们的确定性折叠恰好强于此。**codex 输入（02 笔记）：turn 级预算 reminder 注入——剩余 <10% 时向 agent 本身注入提醒让它自管理收尾/分段（与 HUD pill 给人看互补，一个治透明度一个治自调度）**。**openclaw 输入（03 笔记）：压缩工程三件套——①压缩前 memoryFlush 静默记忆轮（先落重要信息再折叠）②safeguard 摘要质量护栏（校验结构坏则纠正重试，比熔断放弃多了修复）③keepRecentTokens 热尾参数化**。**opencode 输入（04 笔记）：steps 上限「仅文本收尾」——触顶后注入 system prompt 让模型输出总结再停，给 agent 一次收尾发言权，与 codex reminder 注入（事前自管理）组成前后两半**。待用户拍板动工；现状隐性代价 = 摘要失败时 TokenWindowStage 裸截断丢历史 |

## 批次 P — 产品打磨

| # | 项 | 备注 |
|---|---|---|
| P1 | 项目 icon / theme_color 无 UI | 项目维度自查仅剩这条 |
| P2 | 对话重名检查（U13 空值已修） | |
| P3 | ChatHeader 绕 store 直接改标题（U17） | |
| P4 | 侧栏最后消息预览（U10） | |
| P5 | 消息右键菜单（U15） | |
| P6 | 快捷键文档（U8：Ctrl+N/W/K 已实现） | 设置页一屏说明即可 |
| P7 | 移除 AgentForm 高级设置区（回归极简）✅ 已执行 2026-08-16 | UI/bridge/类型已删（`hasWorkspacePath` 保留，工作区按钮仍用）；后端 `get/set_agent_yaml_field` 命令保留（提案系统 / 未来 in-flow 复用），清扫周期结束仍无消费者则一并删。配套：CLAUDE.md 设计规则「配置放置阶梯」+ 记忆 form-minimalism-principle |
| P8 | **权限模式分档**（借鉴拍，加法拍首位候选） | 会话级档位 + agent.yaml `tool_permissions` 规则。**不是新授权系统**——tool_executor 现有 AuthorizationLevel + Once/ThisDir/ThisTool + workspace 校验之上加前置短路层，模式内放行的跳过弹窗，落空走现有对话。配置阶梯对位：模式=L2 会话旋钮、规则=L3。**codex 输入（02 笔记）：两轴正交表述——能力边界（workspace 校验轴）与审批时机（AuthorizationLevel 轴）分开配置，"换审查者不扩边界"为验收不变式；「审批摩擦=安全威胁」为价值论证（摩擦逼用户全开 Always = 我们的 yolo）；全局 Always 档补「不推荐」标注**。**opencode 输入（04 笔记）：①ask 时工具主动建议安全 pattern（`git status*`）让用户一键放行——治审批摩擦的最小 UX 解法②细粒度对象语法——bash 匹配解析后命令、edit 匹配路径、webfetch 匹配 URL：每工具声明自己的鉴权维度而非单一档位；last-match-wins 规则序语义** |
| P9 | **会话分叉 / rewind**（借鉴拍，与 MA-2 同期——共享事件日志地基） | 消息/轨迹视图任意轮「从这里重开」→ 新会话，前缀 `load_history_from_events` 锚 seq 派生落库，`parent_conversation_id`（MA-1 已建列）记 fork 边。CC 快照方案的坑（污染 git 历史/已删文件不恢复）我们从事件派生天然没有；derive 已证逐字节可靠 = rewind 只是它的新消费者 |
| P10 | 借鉴小项合集 | ① Edit 失败恢复阶梯写进工具 description（唯一匹配失败→扩宽上下文→replace_all 引导，极小）② 辅助任务（摘要/图片代读）默认走 provider 最便宜档，复用 preferences 的 embedding/vision 独立槽位模式（L1 默认，不配不操心）③ 慢工具 5s 进度行阈值 + 进度草稿上限（maxLines=8 / 120 字符行截断）——openclaw 成熟参数包（03 笔记），前端打磨顺手做 ④ doom_loop 同参重复检测——同工具+相同输入连呼 3 次=最强卡死信号，补进 stuck_detect 现有熔断（opencode 04 笔记，小） |
| P11 | **轮次导航条不稳**（用户 2026-08-17 真机报） | 症状三连：①轮号数字读取不准确 ②点击定位经常判断失误（跳错位置）③不更新。组件 = TurnRail.vue + useTurnRail.ts（UX #5 v2 定容窗口）+ ChatMessages.vue 视位侦测/跨页跳转。**背景**：该组件已历两轮修复（540490e 视位冻结/token 爆炸双修 + v2 定容窗口重做）仍不稳——怀疑面在「视位侦测 ↔ 分页未加载页高度不可知」的交互、轮号基准（后端 list_turn_anchors distinct turn_id ↔ 前端分页行序）、窗口/视位刷新时机。**修前先真机复现采集**（哪类会话、什么动作后失准、数字错还是锚点错），勿盲改；必要时考虑诊断模式（tick 悬浮显示 messageId + 视位计算中间值） |
| P12 | **任务胶囊 + 计划面板重设计**（用户 2026-08-17 提出） | 现状痛点：TaskPanel popover 计划/任务上下排布，任务多+计划多时**超出页面**。用户已拍方向：**计划与任务横向两栏展示**。同批要整的：①胶囊本体样式与文本内容优化 ②计划变更/任务状态变更的动画表现（建议 subtle：done 打勾 + 行背景轻闪，勿弹跳）③交互体验整体过一遍。**设计约束**：popover 锚右上角，横向加宽须防窄窗左溢（max-width + 两栏各自 max-height 内部滚动——溢出从页面级收敛到面板内）；MA-2 项目级台账是独立页面，会话内胶囊保持轻量索引定位，**勿把台账功能长进 popover** |

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

## 🗑 划掉区（已核验消亡，勿复活）

- **A4 测试跑不起来（sodium DLL）** → 2026-08-13 钉真根因 = lib test harness 缺 comctl32 v6 manifest，build.rs 已修；`cargo test --lib` 831 passed。**可测试性三连的门控钥匙已开。**
- **`.cargo/config.toml` 机器绝对路径** → 文件已不存在（2026-08-16 核验）。
- **08-06 自查 31 项 FIXED** → 2026-08-08 四 agent 并行逐条对 HEAD 复核结清（R1-R9 / F1-F6 / U1-U4 等），明细见记忆 audit-2026-08-06，不再重列。
- **07-31 自查已修项** → P0 三连（会话卡死 / env 泄漏 / 资源清理）+ quick-win 行级修复，2026-08-02 结清。
- 上下文预算 Phase 0+1+2 手测 → 并入 V1。

---

## 清扫循环（方向一工作法）

**清扫 → 借鉴（竞品研读，只积累不实施）→ 加法 → 再清扫。** 每拍有 Done 标准：
- 清扫拍：本表批次清零（V/S/P 逐批销项）
- 借鉴拍：每产品一份**全景**四问笔记（它解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本）——**范围 = 整产品**（架构设计、技术实现、设计理念、各关键功能的特点与优势），不是单痛点切片（2026-08-16 用户纠偏）。已产出：claude code **压缩续跑切片**（docs/competitor-claude-code.md，喂 S8）+ **全景笔记**（docs/competitor-claude-code-panorama.md，2026-08-16：已对齐 11 项 / 要借鉴 7 项【权限分档·并发工具组·rewind 入口最值】/ 结构性更强 3 项 / 不借鉴 5 项 + 未证实存疑清单）；**codex 全景笔记**（docs/competitor-codex-panorama.md，2026-08-16：已对齐 8 项【append-only 日志·深度=1·标记式 rollback——与 CC 交叉验证】/ 要借鉴 6 项【两轴正交权限喂 P8·reminder 注入喂 S8·auto-review 远期】/ 结构性更强 4 项 / 不借鉴 4 项 + 存疑清单）；**openclaw 全景笔记**（docs/competitor-openclaw-panorama.md，2026-08-16：已对齐 7 项【配置哲学·pruning/compaction 分离·拒绝闭环——第三家同构】/ 要借鉴 5 项【压缩三件套喂 S8·持久写者 fencing 喂 MA-3·慢工具 5s 进度行】/ 结构性更强 4 项 / 不借鉴 5 项【无事件日志反证我们差异化·文本协议债】+ 存疑清单）；**opencode 全景笔记**（docs/competitor-opencode-panorama.md，2026-08-16：已对齐 8 项【subagent_depth=1 第三家同构·系统任务也是 agent·UX 优先哲学】/ 要借鉴 6 项【仅文本收尾喂 S8·doom_loop 同参信号喂 stuck_detect·建议 pattern+细粒度对象语法喂 P8·skill 渐进披露】/ 结构性更强 3 项 / 不借鉴 5 项【无事件日志第四家反证·快照式回退第二证·文本解析 allowlist】+ 存疑清单）。**借鉴拍收官：dsh / claude code / codex / openclaw / opencode 五产品齐，候选清零**
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
