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
| V7 | **Phase 2A 读路径切换** | 已 commit | dev 正常对话零变化；日志见 `[read_route] → derive (green)`；DevTools `get_read_route_status`。**2026-08-16 真机首证：Derive green（events=891 diffs=0）**，观察期累积中 |
| V8 | 孤儿 tool_use 对称清场 | 已合 main | 异常终止路径不留孤儿 tool_use 卡死 |
| V9 | 远程 MCP 传输（streamable HTTP） | 记忆 remote-mcp-transport | 真 HTTP server 握手 + tools/call |

> 上下文预算 Phase 0+1+2 的手测已并入 V1（摘要链路本次重建）。
>
> **V1 手测（2026-08-16）✅ 结局一（首档即成功）**：cap=4096、completion=1275、产出 808 字符——额度未被思考烧穿，正文正常产出；滚动折叠工作（462→341 条，covered_until_rowid 前进至 1300）；窗口界 160K 生效。用户战略判断：续期仍偏数字游戏 → S8 治本；AgentForm 高级区多余 → P7。
> 0.3.5 发版手测清单六项与 V2-V7 大面积重叠，以本表为准。

## 批次 S — 结构减法（DeepSeek 式简化，测试数不降为硬约束）

**S0（前置门槛）**: V7 真机持续绿观察 ≥ 一段日常使用期（2026-08-14 起计）→ 解锁 S1。

| # | 项 | 内容 | 备注 |
|---|---|---|---|
| S1 | **Phase 2B legacy 读路径退役** | 删 legacy 拼装整条路径 + 摘要锚点 `covered_until_rowid`→seq + Image base64 双份存储治理 | 最大一笔减法；给旧会话补事件 backfill 先行 |
| S2 | protocol.rs 拆分（A5）✅ 已执行 2026-08-16 | 1161 行混 3 类 + 测试（image_validation / LlmProvider 早已迁出）→ `protocol/` 目录：llm.rs（ContentBlock/ChatMessage/TokenUsage/ChatDelta/ToolDef）+ input.rs（前端输入）+ events.rs（事件负载）+ mod.rs glob re-export **全库导入零改**；两条 legacy 兼容 re-export 保留（image_validation 条目、`harness::provider::LlmProvider`） | 32 个协议测试随迁（5+4+6+17）；831 passed 持平 |
| S3 | chat_cmd send_message 收尾（A1） | ~280 行单函数 → 委托拼装已做一半，收尾成编排门面 | 热路径 |
| S4 | LoopConfig 数据袋（A6） | 22 扁平字段 → 按不可变配置 / 可变运行时分组 | |
| S5 | send_message 集成测试（A2） | MockProvider 787 行仍无人用（2026-08-16 grep 复核）——补全链路 e2e 或删掉 mock | 做了才配谈 loop_engine 可测性（A3） |
| S6 | loop_engine 去 AppHandle 硬依赖（A3） | 与 S5 锁死，一起动 | |
| S7 | 废弃字段清理 ✅ 已执行 2026-08-16（tool_trim_threshold 部分） | `tool_trim_threshold` 全链摘除：models.rs（AgentRow/AgentFileConfig/Agent/NewAgent/AgentUpdate + apply_to×2）/ repo/agent.rs（SELECT×2 + INSERT 17 列 + update 签名与 SQL）/ agent_cmd + 5 处测试夹具 / 前端 types ×2 / client.rs 注释措辞。serde 默认忽略未知字段 → 旧 yaml/JSON 负载向后兼容；migration 06 列按「已发布 migration 不可变」纪律保留 | `enabled_tools` 复核为**活字段**（proposal 分级 Medium 依据 + proposal_tool args + yaml 模板写入 + 前端审批流）——只白名单概念死，字段本身保留；`register_meta_tools` 刻意保留（未来白名单 UI 活口）；831 passed 持平 |
| S8 | **无限续写机制（治本）** | 摘要失败回退链：自适应额度（已有）→ **确定性折叠**（保 system + 首末 N 轮原文，中段压成工具调用骨架，不依赖 LLM 永不失败）；终止语义重排：stuck 无进展为唯一常规终止，budget 降格为失控保护（本地模型 = 真·无限，付费 API = 成本上限） | **设计输入已备：docs/competitor-claude-code.md（借鉴拍第一份）**——补两个新输入：历史工具结果瘦身（预览+指针）排最前、摘要请求自身截头防超长；Claude Code 熔断后无回退会搁浅，我们的确定性折叠恰好强于此。待用户拍板动工；现状隐性代价 = 摘要失败时 TokenWindowStage 裸截断丢历史 |

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
| P8 | **权限模式分档**（借鉴拍，加法拍首位候选） | 会话级四档旋钮（逐次确认/工作区内自动/只读/全自动）+ agent.yaml `tool_permissions` 规则。**不是新授权系统**——tool_executor 现有 AuthorizationLevel + Once/ThisDir/ThisTool + workspace 校验之上加前置短路层，模式内放行的跳过弹窗，落空走现有对话。配置阶梯对位：模式=L2 会话旋钮、规则=L3。治审批疲劳（详见 panorama 笔记第三节 #1） |
| P9 | **会话分叉 / rewind**（借鉴拍，与 MA-2 同期——共享事件日志地基） | 消息/轨迹视图任意轮「从这里重开」→ 新会话，前缀 `load_history_from_events` 锚 seq 派生落库，`parent_conversation_id`（MA-1 已建列）记 fork 边。CC 快照方案的坑（污染 git 历史/已删文件不恢复）我们从事件派生天然没有；derive 已证逐字节可靠 = rewind 只是它的新消费者 |
| P10 | 借鉴小项合集 | ① Edit 失败恢复阶梯写进工具 description（唯一匹配失败→扩宽上下文→replace_all 引导，极小）② 辅助任务（摘要/图片代读）默认走 provider 最便宜档，复用 preferences 的 embedding/vision 独立槽位模式（L1 默认，不配不操心） |

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
- 借鉴拍：每产品一份**全景**四问笔记（它解决什么问题 / 靠什么架构 / 我们要不要 / 引入成本）——**范围 = 整产品**（架构设计、技术实现、设计理念、各关键功能的特点与优势），不是单痛点切片（2026-08-16 用户纠偏）。已产出：claude code **压缩续跑切片**（docs/competitor-claude-code.md，喂 S8）+ **全景笔记**（docs/competitor-claude-code-panorama.md，2026-08-16：已对齐 11 项 / 要借鉴 7 项【权限分档·并发工具组·rewind 入口最值】/ 结构性更强 3 项 / 不借鉴 5 项 + 未证实存疑清单）；候选：codex / opencode / openclaw
- 简化拍约束：测试数不降、clippy 零警告、行为零变化（重构与功能改动不同 commit）

### 借鉴拍反面守则（2026-08-16，claude code 踩坑 → 我们的 设计守则）

1. **LLM 依赖必有确定性兜底**——它压缩熔断后无回退会话搁浅（3272 连败事故）；摘要/代读/自愈类设计都要带「失败也能走」降级路径（S8 即此守则产物）
2. **会话单一写者**——它同会话双终端 resume 消息交错；MA-3 持久通道 / 远程接入前的第一条不变式
3. **隔离必须可降级**——它 worktree 隔离对非 git 目录近乎强制；将来多 agent 并行写文件，隔离失败应退化成排队而非拒绝工作
4. **快照式回退天生脏**——它 checkpoint 污染 git 历史、已删文件不恢复；反证我们 rewind 走事件派生路线
5. **钩子否决须防自旋**——它 Stop hook 可拦停致 #55754 死循环；hooks 若扩展否决语义必须配重触发上限
6. **按需加载要进管道不要事后补**——它 skills 渐进加载设计了没落地（#16160）；工具描述/KB 提示瘦身走管道 Stage（S8 L0 思路）

### 远期构想存档：graph 工作台（2026-08-16 用户头脑风暴，只存不动）

方向共鸣：并发工具 → agent 图协作演进（CC agent teams / 业界 graph 编排同潮）。**设计立场（克制条款）**：
- **图从事件日志长出来，不引入声明式 DSL**——愿景已锁「agent 间是图可直连」，MA-1 的 parent_conversation_id 边就是图的第一个物化；DSL 路线（先画图再执行）要自带状态机+重放，与「真相在产物/日志无损」重复且冲突
- **项目概览/监控 = session_events 的纯派生投影**（delegation/plan_updated 事件聚合），不是新存储——这是 MA-2 任务台账+项目轨迹的本职，空白真实存在，CC 共享任务清单验证了需求
- **工作台调度前置 = 并行委派 + 单一写者 + 任务台账**；loop 内并发工具（观察池）与 agent 间图是正交两层，不混
- 演进序列：MA-2（台账+轨迹=概览）→ 并行委派 → 图视图基本只剩渲染层；rewind/fork（P9）给树编辑能力
