# 频道 v1 串行共享流 · 设计稿（0.7 批 C）

> 状态：**已落地（2026-09-10 四批 commit：① 引擎 41d2bdc + ② 选举换帅 cf4b3db + ③ 前端 e129d46 + ④ 文档）**，待用户真机使用驱动反馈。
> 真相源声明：落地后 **CLAUDE.md「频道 v1」节为运行时真相源**（本文保留设计动机与决策链；两处冲突以 CLAUDE.md 为准）。
> 接缝事实全部对码核实（文件:行号为 0.7.0 / commit 5b29b1a 时点；落地批的实现细节以代码为准）。

---

## 0. 定位与理念（已拍板，勿翻案）

**一句话**：频道 = 有主持的串行圆桌——@点名必应答（支持多 @）、广播由统筹者编排、
统筹者由系统自选举或用户指定（投票兼任健康检查、故障保守换帅）、成员可传 @ 但护栏截断风暴、
共享流即全员视野、人始终在场且治理权最大。

**调研四原语的落点**（2026-09-10 调研 + 用户修正定稿）：

| 原语 | 竞品形态 | 频道 v1 落点 |
|---|---|---|
| ① 中心路由 | AutoGen SelectorGroupChat / LangGraph supervisor | **统筹者编排**：广播→统筹者接令→应答或 @成员分派 |
| ② 流程剧本 | MetaGPT SOP / ChatDev chat-chain | **不引入**（僵硬 + token 贵，用户否决） |
| ③ 显式点名 | OpenAI Agents SDK handoffs（受限变体） | **@唤醒**：用户与成员都可 @，被@者必应答 |
| ④ 自选举 | （无直接竞品，用户创造性转化） | **一次性选举**：统筹空缺时首次发送触发，非每条消息的全响应 |

核心洞察：所有竞品共享隐含前提「群聊是自动机、人是外部」——IcePaw 相反（本地优先 + 人在环上），
**人本身就是一种路由器**（@ = 人的路由权；广播 = 让渡给统筹者； refuse/指定/补选 = 治理权兜底）。

---

## 1. 核心决策表

### C1 频道 = 项目 1:1 的串行共享流会话

- 频道是 `conversations` 表里 `kind='channel'` + `project_id` 必填的一行——**零新成员表、零新频道表**
  （migration 45 已建 `kind` 列与 `idx_conversations_kind_project` 索引，45 号文件注释已预留
  「不建 project_members 表，可调度集合复用 project_agents」）。
- 成员 = `project_agents` 现读（`db/repo/project.rs::list_agents`），成员变更即时生效（无快照同步义务）。
- 1:1 由部分唯一索引保证：`CREATE UNIQUE INDEX idx_channel_per_project ON conversations(project_id) WHERE kind='channel'`（migration 54）。
- **灵活性天花板显式锁死**（将来加法演进不推翻 v1）：无跨项目频道、无多频道、无散落频道、全员强制在频道（无旁听成员/子集讨论）。
- 频道创建 = `ensure_channel(project_id)` 幂等命令（事务内 SELECT+INSERT，存在即返回）；
  前端侧栏入口点击时懒建，**项目创建时不预建**（老项目零迁移、空项目不产生空会话）。

### C2 统筹者标记 = project_agents.role

- `project_agents.role` 列已有（migration 45 前置，创建时硬编码 `'member'`，set_members/add_agent 已支持调用方传值）——
  加值 `'coordinator'` **零迁移**。每项目至多一个 coordinator（应用层保证，非 DB 约束）。
- 统筹角色的设置入口 = 项目设置页成员管理（组织事实，不是 agent 旋钮——不违反配置放置阶梯）。
- 平票裁决用 `project_agents.joined_at` 最早（确定性「资历」规则）。

### C3 路由决策表（引擎行为规格）

| 输入 | 行为 |
|---|---|
| 用户消息，无 @（广播） | **统筹者接令**跑回合：应答，或在回复中 @成员 分派（分派即接力） |
| 用户消息，@ 成员 A（≠统筹者） | **A 跑回合**，统筹者不跑（知悉靠结构自动，见 C6） |
| 用户消息，@ 多人 | 按 @ 出现顺序**串行**跑，后者自然看见前者的回应（共享流） |
| 用户消息，@ 统筹者 | 统筹者跑（等同点名，不触发编排语义变化） |
| 统筹空缺 + 首次发送 | 先落流（消息正常落库）→ 触发**自选举** → 新统筹产生后按广播路由消费该消息 |
| 成员回复中 @ 他人 | 回合结束后触发被@者回合（**接力**，护栏见 §6） |
| 接力进行中用户再发消息 | **当前回合跑完 → 剩余待触发接力取消 → 用户消息成为新链头**（见 C8） |
| 委派（成员回合内调 delegate） | 不注册（2026-09-11 翻转 C7 拍板，见下） |

### C4 自选举（统筹空缺时的系统级选举）

- **触发条件**：统筹空缺（新项目无 coordinator / 统筹者被移出成员 / 统筹者故障全失效）且用户在频道发送消息。
- **流程**（全程进共享流与事件日志，可审计——像群聊接龙）：
  1. 用户消息先落流（不阻塞输入；MA-3 hold 同款语义——事实先行，处理延后）。
  2. `channel_election(started)` 事件 → 逐成员跑**投票回合**（串行，chat_state 单写者天然保证）：
     注入候选人清单（成员名+一句话身份，取 agent 名/模型）+ 项目背景（project.md 摘要若有），
     输出一个 agent 名 + 一句理由，`max_tokens` 钳制 128。
  3. **投票回合兼任健康检查**：跑不动的成员自动弃权且出局候选
     （`test_agent_model_chain`「真发验对话权益」哲学复用——列模型是鉴权层动作验不出对话权益）。
  4. 计票（每成员一票，被投者必须在候选存活集内）→ 平票取 `joined_at` 最早。
  5. `channel_election(result)` + `channel_coordinator(elected)` 事件 → role 更新 →
     `conversations.agent_id` 同步 → 按广播路由消费落流的首条消息。
- **全员跑不动** = 诚实报错（横幅指路「模型配置不可用，去设置-模型页检查」），消息留在流中，下次发送重新触发。
- **重启中断**：选举是秒级串行操作，重启窗口极小；诚实处理 = 不恢复半途选举，下次发送重新触发
  （已落流的用户消息不丢；重复选举无副作用——role 幂等覆盖）。

### C5 统筹者身份的两档治理（故障换帅保守化）

- **系统自选的统筹者**：接令回合终态失败（错误终态 / 降级链尽）**连续 K=2 次** →
  自动补选（重新走 C4；故障者投票回合跑不动自然出局候选）+ toast 告知 + `channel_coordinator(failed-over)`
  事件记录换帅原因 + **不自动复辟**（防帅位震荡——被换下的成员要用户手动恢复）。
- **用户手动指定的统筹者**：故障**不自动换**（治理权不越权——MA-3 refuse「用户治理权最大」同款语义）；
  toast 提示统筹者故障 + 广播降级为「需点名」模式（广播消息无人接令，系统事实提示用户 @成员 或换统筹者）
  + 一键「让系统补选」入口（频道头部）。
- 统筹者被移出项目成员：role 清空 → 统筹空缺 → 下次广播触发补选（C4）。

### C6 知悉 = 结构自动（v1 免费答案）

串行共享流每个成员被唤醒时读**同一份会话历史**（经 TokenWindow / 滚动摘要压缩，读路径零新机制）——
统筹者的全局视野是共享流的结构属性，不需要额外机制。**不做**实时旁听回合（每次 @别人 统筹者都醒 =
成本翻倍 + 旁听不行动 = 纯浪费；统筹者要介入走被 @ 或接力）。**不做**主动进展概述
（「时刻更新」比旁听还贵 + 概述与流的漂移一致性问题；给人看的 → 用户 @统筹者「总结近况」按需生成）。

### C7 频道回合的工具面（委派禁用——2026-09-11 翻转拍板）

**原拍板（2026-09-10）是「频道同权」：放开 `delegate` 注册为 `chat | channel`，理由「任务分发与频道接力正交」。**
次日三轮生产实测翻转该拍板（频道 19660d0f 两轮对话全走委派：统筹者 02:01 先发三路委派探测，子会话读不到
频道历史、跑在隔离工作区，回传「成员看不到代码」的隔离答案 → 错误信念固化 → 02:25 委派 args 原话
「目前我们四位成员都说看不到项目代码」→ @ 接力被整体绕空）。**委派在频道里是信息断流通道**：
它给模型一个「问了但拿不到共享流知识」的岔路，答案回不到流里、其他成员也看不见。定案：

- **频道回合不注册 `delegate`（也不注入可调度清单 delegation_hint）**——`session_runner.rs` 组装期
  kind 闸收成 `conv.kind == "chat"`。频道内成员协作唯一通道 = @ 接力，共享流承载知识传播
  （被 @ 成员读同一份频道全量历史——包括其他成员报告过的路径与结论）。
- 委派深度护栏语义不变：仍按 kind 注册（delegation 子会话拿不到工具 → 深度恒 1），只是频道也进了
  「拿不到」的集合。
- 仍真的事实（原 C7 的机制部分）：委派是同步工具（结果经 tool_result 回流）；频道回合预算归执行成员
  （`LoopBudget` 每回合局部变量，取自 agent 行配置，无 per-conversation 累计）；降级链继承与 L2 委派
  预授权对频道回合自动失效的仅 delegate 一件——relay 本就仅 1v1。
- **回滚路径**：若未来出现「频道内确实需要同步子任务」的真实场景，恢复注册闸为 `chat | channel`
  即可（一行），但须同时恢复 delegation_hint 注入闸与 CHANNEL_HINT 措辞——三处同闸，勿单边放开。

### C8 用户插话 = 新链头（多条积压，如实落流；不做排队）

MA-3 式排队（pending 队列）在频道语义下不对味：C 还没回答，用户插话会改变 B@C 的语境，串行流里
「先来后到」的排队会让插话悬空。**定案（2026-09-10 用户追问后深化）**：

- **频道的发送拆两步**：`send_message` 频道路径 = **物化 user 消息（无条件成功，立即落流）** +
  **尝试触发回合**（在途则不触发、只登记积压链头）。1v1 会话的在途拦截（T2 批横幅 + 重试）**不适用频道**——
  用户插话是事实，事实必须进流；前端在途发送不再弹错误横幅，消息气泡照常出现。
- **新链头 = 积压消息全量**：回合结束触发点发现「链头之后有新 user 消息」→ 取消剩余待触发接力 →
  短静默窗口内无新用户消息 → 以**积压全部消息**为新链头走 C3 路由。核心机制是白送的：被唤醒成员读
  共享流全量历史，天然看到「跑偏的回复 → 用户连发纠正/催停/新指示」的完整序列与因果——**历史顺序
  就是事实原貌**，无需合并打包。
- **mentions 合并视野**：新链头的路由输入 = 积压各条消息 mentions 的**顺序并集去重**（先 @A 又 @B
  则按序两跳）；全部无 @ 则广播（统筹者接令——它读历史时同样看到全部三条与跑偏现场）。
- **触发前短静默**：`CHAIN_HEAD_QUIET`（~3s）无新用户消息才启动新链——防连发被拆成多条碎链；
  静默期后又有消息到达则重置窗口（「说完一起听」）。
- **立即停诉求走现有停止按钮**：终止当前成员回合——turn_ended 在自然完成与用户终止两种终态都广播，
  触发点统一接管（取消剩余链 + 积压成新链头）。**诚实边界**：在途回合的上下文在回合开始时冻结，
  它当时不知道用户的纠正——这本身就是事实原貌（它跑偏时用户确实还没说）；要它立刻知道，点停止。
- 实现面：无新队列结构——「积压」就是流里的 user 消息行 + 引擎内存的链头标记（重启丢失则下次
  发送重新触发，消息不丢）。

### C8b 时间戳与执行区间（原貌可分析的地基，2026-09-10 追问后补）

**问题**：插话场景的时间交织真相 = 「M2 发出于 R 执行期间」（t1 < t2 < t3），但消息列表只有单点
时间戳（R 行 created_at = 开始 t1），M2 排在 R 之后看起来像「答完了才插话」——不知道 R 的完成
时间 t3 就无法还原「为什么用户会连发」。**原料已全部在产，零新 schema**：

| 数据 | 落点（已存在） | 语义 |
|---|---|---|
| 用户消息发出时刻 | messages.created_at / user_message 事件 created_at | C8 物化即落库 ≈ 发出时刻 |
| 回合开始 | assistant 占位行 created_at | 回合启动创建（在案事实：占位行同回合创建） |
| 回合完成 | assistant_message 事件 created_at | 终文 append 时刻 |
| 回合时长 | payload `duration_ms`（event_log.rs，旧事件 None） | 完成−开始 双保险 |

**排序语义维持现状**：`(created_at, rowid)` 复合序 = 按开始时间排（与用户设想一致）——不需要改
排序，缺的是**区间信息**让「用户消息 ∈ 上一回合 [开始, 完成]」可判定。两个补件：

1. **derive 透传**：`MessageRow` 增 `#[serde(skip)]` 的回合时长字段（源 = AssistantMessagePayload.
   duration_ms，supersede last-wins 取最后一续写）——完成时间 = 行 created_at + 时长，前端消息页
   可判定（此前完成时间只活在事件侧，到不了消息页）。
2. **呈现 + 注入**：
   - 频道页 assistant 消息组显示「开始 → 完成」（相对时 + hover 绝对时）；**用户消息 created_at 落
     在前一回合区间内 → 气泡标注「生成中发出」**——连发重复指示的原因在时间轴上自解释。
   - LLM 侧：新链头消费积压时一次性注入事实「以下 N 条用户消息发出于上一回合执行期间，按时间序
     理解为递进指示」（频道事实简报注入，C10b；引擎触发点本来就在比对链头后的新 user 消息，判定免费）。

### C9 @ 寻址：用户侧结构化、成员侧文本解析

频道内 @ 是**纯寻址**（共享流内无需引用快照——被@者读的就是全量历史），与 1v1 会话的 @ 引用
（快照注入，`harness/references.rs::materialize_reference_blocks`）是**两种语义，不共用数据通路**：

- **用户侧（结构化，零解析歧义）**：频道会话的 ChatInput @ 弹层只列**当前项目成员 agent**
  （复用弹层骨架，候选源换 `project_agents`）；选中后同样 chip 形态，但 send 时组装
  `mentions: [{agent_id, display}]` 数组随消息传后端（**不生成 reference 块、不注入快照**）。
  文本中保留 `@名字` 字样（人可读），路由只认结构化数组。
- **成员侧（文本解析，护栏兜底）**：成员回复中的 @ 靠后端解析——`@` 后跟**成员名单精确匹配**
  （最长匹配优先；agents.name 无 UNIQUE 约束，**重名歧义不触发** + 系统事实消息说明歧义，诚实不猜）。
  误触发（提到名字非指令）的代价 = 多跑一回合（护栏截断，可接受）；频道礼节段教成员
  「需要某成员行动时在回复开头 @它，仅提及无需 @」。

### C10 频道发言 = 共享流一等消息（不立新 kind）

- 用户消息与成员回复都走 `user_message` / `assistant_message` 既有 kind（derive 读路径零改动——
  全员「看见彼此」靠的就是这条派生链）。**pending 排队语义（MA-3 cross_session_message）不引入**——
  C8 已定插话打断，无排队。
- **发送者元数据双轨**（incoming_source 三写同款手法）：
  - `messages.sender_agent_id` 列（migration 54；NULL = 用户消息或 1v1 隐含会话 agent；
    二段 UPDATE 写入，`NewMessage` 不扩字段防全量构造点爆破——`set_incoming_source` 同款）。
  - `AssistantMessagePayload.sender: Option<SenderMeta{agent_id, agent_name}>`（serde default 零迁移，
    旧事件可读）+ derive 透传 → `MessageRow` 增 `#[serde(skip)]` 字段。
  - 前端渲染行级发送者（头像/名字）优先读列，事件兜底。
- 用户被 @ 唤醒的成员回合：物化一条带触发标注的 user 消息？**不物化**——被@成员的回合直接读共享流历史
  （@它的用户消息/统筹者分派就在流里），引擎只需以成员身份起回合。回合的 `EventCtx.agent_id` = 执行成员
  （事件层归因已正确）。**不造「系统代写 user 消息」**——共享流里消息只有两种权威：用户说的、成员说的。

### C10b 系统事实的 append-only 分层原则（2026-09-10 拍板：从「日志是系统最核心功能」角度重审）

> 用户质询原话：「appendonly 日志是我们系统最核心的一项功能点，质量和设计必须都要保证是最高级的
> 水平」。据此把「系统事实不物化消息行」从省事判断升格为**分层原则**。

**分界线：内容性事实（谁说了什么）→ 物化 messages 行（带 sender 权威）；行为性事实（引擎做了
什么决定）→ 只进 session_events。**

为什么这条线是 append-only 质量的保证（而非仅仅是省事）：

1. **单一事实源不可重复**。系统事实若物化成消息行，同一事实就有两个真相（messages 行 + 事件行）——
   对账平面（reconcile：行级 vs 事件回放）从「构造性零 diff」退化为「多出一类永远对不齐的行」，
   对账的信噪比被永久污染。MA-3 已验证同款分界：pending 投递只进事件不写 messages，消费物化的是
   真 user 消息（run_agent_turn 的自然产物）而非「系统通知」。
2. **共享流的发言权威只有两种**（用户说的、成员说的）——成员读历史时的信任模型简单且完整；
   「系统说的」若混进流里，成员引用/回应系统消息的语义无定义（@系统？回复系统？）。
3. **对 agent 的告知走「频道事实简报」= 事件的派生投影**：回合启动时从相关事件派生一段事实文本
   注入、用完即弃——**派生物不落库**（「项目概览=纯派生投影」同原则：落库的派生副本会过时，
   append-only 系统里过时副本 = 谎言）。v1 简报三场景：① 插话积压事实（C8b 的「N 条用户消息发出于
   上一回合执行期间，按时间序理解为递进指示」）；② 护栏拦截事实（被拦成员知道为什么接力没发生，
   防困惑循环重试）；③ 广播降级状态（统筹空缺/故障时指路「@成员 或换统筹者」）。
4. **选举投票回合物化的正当性由这条线确立**：投票是成员的真话（内容性事实——成员表达的选择），
   不是引擎叙事；用户拍板「投票过程进共享流」与原则自洽（引擎的计票/裁决/换帅决定仍只进
   `channel_election` / `channel_coordinator` 事件）。
5. **前端可见性同源**：ChannelNotice 提示条从事件总线增量渲染（useInbox 同款
   `session:event-appended` 过滤三 kind）——人看到的与日志里记的是同一份事实，无第二真相。
6. **日志完整性论证**：三新 kind 全部 append-only + derive skip 臂同步 + **归档频道不删除**
   （§2——日志永不丢段，即使项目消亡）；回放（derive 零改动）、对账（skip 臂覆盖新 kind）、
   导出（JSONL 自然带新 kind）全链路零破坏。

---

## 2. 数据模型

### migration 54（`54_channel_v1.sql`，唯一一个迁移）

```sql
-- 频道 v1：发送者归属（共享流多成员发言的行级归因）
ALTER TABLE messages ADD COLUMN sender_agent_id TEXT REFERENCES agents(id) ON DELETE SET NULL;

-- 频道归档态（项目永久删除时频道软删除为只读归档，聊天记录保留——2026-09-10 拍板）
ALTER TABLE conversations ADD COLUMN archived_at TEXT;

-- 项目 ↔ 频道 1:1 唯一性（部分唯一索引；ensure_channel 幂等的 DB 兜底。
-- 归档频道 project_id 被 FK SET NULL 清空后，NULL 不参与唯一判重——同项目可建新频道）
CREATE UNIQUE INDEX idx_channel_per_project ON conversations(project_id) WHERE kind='channel';
```

零新表、零 conversations 其余改动（`kind` 列与复合索引 migration 45 已备）、零 project_agents 改动（role 列已备）。

### 频道归档（项目删除的软删除路径，2026-09-10 拍板）

项目生命周期已有两档：**归档**（软删除，会话不动——`project_cmd.rs:121` 现状天然保留频道）与
**永久删除**（`permanent_delete_project` 带 `delete_conversations` 处置参数）。频道的处置矩阵：

| 路径 | 普通会话 | 频道 |
|---|---|---|
| 项目归档 | 不动 | 不动（现有语义零改动） |
| 永久删除，`delete_conversations=false` | FK SET NULL 转散落 | **归档保留**（置 `archived_at`，不转散落——散落频道违反 C1） |
| 永久删除，`delete_conversations=true` | 物理删除 | **仍归档保留**（频道是多人协作史，价值密度高——Modal 文案披露「会话将删除；频道记录将转为只读归档」） |

- **归档频道形态**：只读（前端隐藏输入框，后端 send 拒绝并三段式说明）；头部标「已归档 · 原项目已
  删除」；成员/统筹者显示靠 sender 元数据快照（messages 列 + 事件 payload 均带 agent_name，成员表
  随项目删了也不影响回看）。判定「活频道」= `kind='channel' AND project_id IS NOT NULL AND archived_at IS NULL`。
- **侧栏入口**：散落 scope 的频道区块位置显示归档频道行（带「已归档」标）——入口不断链，用户可回看
  与导出；活频道仍在项目 scope。
- **清理出口**：归档频道行提供删除（两步确认武装态）→ 物理删除（与 1v1 会话删除同级语义，
  session_events 级联）；活频道（挂项目的）无手动删除入口，生命周期跟随项目。
- **agent_id 守卫统一**：归档频道同样受 §2 删除守卫约束（删统筹者先迁 agent_id，全删光拒删指路）。

### conversations.agent_id 的归属语义（关键风险位）

`agent_id` NOT NULL + `ON DELETE CASCADE`（migration 01）。频道会话的 agent_id **填当前统筹者**
（显示位自然：侧栏频道行 / ChatHeader 主头像用统筹者；运行时回合不受它影响——频道引擎显式传执行成员，
`AgentTurnInput.agent` 是独立字段）。换帅时同步 UPDATE。**两个级联陷阱必须守卫**：

1. **删除 agent 守卫**（agent_cmd 删除路径）：被删 agent 若是某频道的 agent_id →
   先把频道 agent_id 迁到同项目任一剩余成员（保 NOT NULL + 防级联删频道历史）；
   无成员可迁（最后成员被删）→ 拒绝删除，三段式指路「先删除该项目或移除频道成员」。
2. **删除项目守卫**（项目删除路径）：`conversations.project_id` FK 是 SET NULL——频道转散落会破坏
   C1 边界（散落频道不存在）。**项目永久删除时频道一律归档保留**（置 `archived_at`，见上节处置矩阵
   ——聊天记录是多人协作史，不随项目陪葬）；项目归档路径不动频道（现有语义天然满足）。

---

## 3. 事件词表（18 → 21，+3 kind）

新 kind 集中在 `event_log.rs::kind` 模块 + typed emitters（词表单一真相源），三处同步：
derive skip 臂、前端 `EV_KIND_TO_FILTER` + `summarizeEvent`、轨迹瀑布图 `laneOf`（归 User 道外来输入侧，
cross 词表先例）。

| kind | actor | turn_id 归组 | payload 要点 |
|---|---|---|---|
| `channel_election` | `system` | `election:{发起消息id}` | phase=started/vote/result；vote 带 voter/被投者/理由摘要/弃权原因；result 带票数表/胜者/平票裁决 |
| `channel_coordinator` | `system`（用户指定时 `user`） | 无（轮外事件，backfill 同款容忍位） | action=elected/appointed/removed/failed-over；agent_id；reason（换帅带失败计数） |
| `channel_mention` | `agent:{发起者id}`（用户触发时 `user`） | `chain:{链头消息id}` | from/to agent_id、hop_index、chain_remaining；护栏拦截时带 blocked_reason（pair_repeat/chain_limit/frequency/ambiguous_name/user_preempted） |

⚠️ 不变式：① 轮外事件（channel_coordinator）挂 turn_id 难题 → 走 fork 创世/轮外容忍先例，
derive skip 臂必须含三 kind（否则 DeriveIssue 污染对账）；② `chain:{id}` 三侧同归组键（事件/引擎/前端轨迹）；
③ Image 块经 `refify_blocks` 照旧（emitter 字段式签名内部做）。

---

## 4. 频道引擎（`harness/channel.rs` 新建，镜像 inbox.rs 骨架）

```
发送入口（chat_cmd send_message 检测 kind='channel' 分流）
  → 物化 user 消息（无条件成功，立即落流——C8 两步拆分的第 1 步）
  → route_user_message(conv, mentions, content)
      ├─ mentions 非空 → 校验成员资格 + 上限 → 触发第一跳（被@者串行队列）
      ├─ mentions 空 → 统筹者在位 → 统筹者接令
      │                统筹空缺 → spawn_election() → 完成后消费
      └─ 会话在途（chat_state 占用）→ 不触发、只登记积压链头
          （回合结束触发点接管：取消剩余链 + CHAIN_HEAD_QUIET 静默 → 新链头消费）

回合结束触发点（EVENT_BUS 订阅 TURN_ENDED，drain watcher 同款模式——自然完成与用户终止两态同接）
  → on_turn_ended(conv)
      ├─ conv 非 channel → return
      ├─ 等静默（2s 轮询 is_streaming，30s 上限——inbox drain 常量同款）
      ├─ 用户抢占检查（C8：链头之后有新 user_message → 取消剩余链，channel_mention 记 user_preempted
      │   → 再等 CHAIN_HEAD_QUIET 静默 → 以积压全部消息为新链头走 C3 路由，
      │   mentions = 各条顺序并集去重）
      ├─ 解析本回合 assistant 终文的 @（C9 成员侧规则）→ 逐个触发下一跳（channel_mention 记 hop）
      └─ 统筹者失败计数维护（C5：接令回合终态失败 streak++，成功清零，达 K 触发换帅档）

run_channel_turn(member, conv, chain_state)
  → get_with_credentials(member)（凭据解析同 inbox.rs:468 先例）
  → chat_state.start(conv_id) 仲裁（忙 → 等静默重试，永不并发）
  → run_agent_turn(TurnEnv, AgentTurnInput{ conv, agent: member, ... })   ← 全链路复用
  → done_rx 回合后台跑（onespot 模式同 inbox.rs:538-580）
```

- **引擎状态**：链状态（链头 msg_id → 已跳数 / 对重复表 / 剩余额度）与唤醒频率窗口为**内存态**
  （`auto_consume_reserve` 同款；重启清零 = 链死，诚实可接受——链是分钟级生命周期）。
  落库可观测性全靠 `channel_mention` 事件（重启后轨迹仍完整）。
- **系统事实**（护栏拦截 / 歧义说明 / 广播降级提示）：**不物化 messages 行，只 append 事件**
  （C10b 分层原则——行为性事实只进日志，对账平面保持构造性零 diff）；前端 ChannelNotice 从事件
  总线增量渲染；对 agent 的告知 = 频道事实简报（事件派生投影，回合启动注入不落库，v1 三场景见 C10b）。
  选举的投票回合例外——投票是成员的真话（内容性事实），物化 user/assistant 消息对进共享流
  （可审计拍板「投票过程进共享流」；计票/换帅决定仍只走事件）。
- **工具面**：`delegate` **不注册给频道回合**（2026-09-11 翻转，见 C7——信息断流通道，@ 是唯一成员协作通路）；
  `send_message_to_session` / `list_conversations` **不注册给频道回合**（v1 砍法——频道内 @ 是原生通讯方式，
  跨会话投递从频道发起留 v2；`inbox.rs:314` deliver 拦 `kind != "chat"` 与 drain watcher `kind != "chat"`
  判定**维持现状**——频道不收投递，防侧信道绕过频道护栏，delegation 同款逻辑）。
  `read_reference` 同闸收口（2026-09-11 ⑧ 批）：@ 引用快照钻取是 1v1 语义——频道 @ 纯寻址（C9）不产快照、
  委派子会话无用户 @，调用结构性必败（生产实案：被唤醒成员困惑时反复调它 hunting「@ 我的那段」两连败）；
  该工具在 `register_builtin` 全局注册（与 delegate 的按 kind 注册路径不同），收口走组装期 `snap.remove`
  ——路径不同、闸相同。

### system prompt 注入（频道礼节段）

`SystemPromptParts` 增第 6 段 `channel_hint`（`context/system_prompt.rs`——拼接序在 delegation_hint 之后、
word_style 之前；**stable_hash 覆盖段清单同步加**，否则 miss 归因恒真噪声）。内容（给所有频道回合注入）：

- 你在项目频道中，与 N 位成员共享同一对话流：{成员清单 = 名字 + 一句身份}。
- 当前统筹者：{名字}。你是/不是统筹者（分档措辞：统筹者段多一句「广播消息由你接令，可在回复中 @成员名 分派任务」）。
- 需某成员行动时在回复开头 @其名字；仅提及无需 @。用户 @你 = 点名必应答。
- 你能看到全部频道历史（含其他成员的发言）——引用时直接指名，无需请求转述。

选举回合注入（一次性，不进 channel_hint）：候选人清单 + 项目背景 + 输出格式指令（一个 agent 名 + 一句理由）。

---

## 5. 护栏常量（题 4 定稿）

原理：串行共享流下不存在并发风暴（chat_state 单写者结构性保证一时刻一成员发言），风暴只剩两种形态——
**接力链无限延长**与**循环对互 @**。护栏全部落在「回合间触发链」，与既有熔断分工：
预算归执行成员（per-agent 天然限制单回合成本）、doom_loop / stuck_detect 管回合内、频道护栏管链。
哲学同 `MANUAL_REPLY_GUARD`：**护栏不依赖模型听话**（系统级计数拦截）。

`harness/channel.rs` 模块顶（对齐 inbox.rs:78-89 形态）：

```rust
/// 单条用户消息触发的连续成员回合上限（@3 人 = 3 跳起算，统筹者分派继续计）。
/// 达到上限不再触发，系统事实提示「接力已达上限，请用户推进」。
const MAX_CHAIN_TURNS: usize = 8;
/// 同一有序对（发起者→被@者）在一条链内出现次数上限——A@B、B@A、A@B 第三次拦（防乒乓）。
const MAX_PAIR_REPEAT: usize = 2;
/// 单条消息解析出的 @ 数量上限，超出截断 + 诚实提示；
/// 多条积压消息合并为新链头时按「去重并集」适用同一上限（防拆分绕过）。
const MAX_MENTIONS_PER_MSG: usize = 5;
/// 同成员唤醒频率窗口（对齐 MA-3 AUTO_WINDOW/AUTO_CONSUME_MAX 同源语义：
/// 「外部触发的回合频率」）。超限本窗口内不再自动唤醒，系统事实说明。
const MENTION_WINDOW: Duration = Duration::from_secs(600);
const MENTION_MAX_PER_MEMBER: usize = 6;
/// 统筹者接令回合连续终态失败次数 → 换帅档触发（C5）。
const COORD_FAIL_STREAK: usize = 2;
/// 选举投票回合输出钳制。
const ELECTION_MAX_TOKENS: i64 = 128;
/// 用户消息积压 → 新链头启动前的静默窗口（C8）：窗口内又有新用户消息则重置
/// ——「说完一起听」，防连发被拆成多条碎链。
const CHAIN_HEAD_QUIET: Duration = Duration::from_secs(3);
```

取值依据：MAX_CHAIN_TURNS=8 覆盖「@3 人 + 统筹者 2 轮分派 + 1-2 跳成员间协作」的合理编排余量，
同时把最坏成本钉在 ~8 个成员回合；MENTION_* 直接对齐 MA-3 配额（同为防 agent 自主互发的频率闸，
真机 A↔B 双投递实案标定的值）；其余为语义最小值。**全部数值为 2026-09-10 初版拍板值——上线后按
实际使用优化**（触发频率/误拦率驱动调参，观察项在案）。**全部常量不进配置**（L1 好默认——
调参需求出现时再上 yaml，勿预建旋钮）。

---

## 6. 前端

### 6.1 侧边栏改版（Sidebar.vue，两棵树同步改）

拍板顺序：**LOGO → 项目操作栏 → 项目级别频道 → 新建会话 → 会话列表**。现状是
LOGO → 搜索 → 新建对话 → ProjectSwitcher → 分隔线 → 列表（`Sidebar.vue:271-333` 展开树），改法：

- `.sidebar-top` 内部重排：ProjectSwitcher 提到最前 → **频道区块**（新）→ 新建对话按钮。
- **搜索框整体移除（2026-09-10 拍板）**：目前没有使用它的场景——展开树头部搜索与 rail 态 flyout
  内搜索（「与展开态同款、共用过滤逻辑」）一并删，过滤逻辑随搜索 UI 一并退役；将来真有需求再立项
  （勿预建）。
- **频道区块跟随 scope（2026-09-10 拍板确认）**：项目 scope = 该项目频道一行（无则显示「开启频道」
  懒建入口）；散落 scope = **归档频道列表**（§2 归档方案的入口，行带「已归档」标；无归档频道则区块
  隐藏——与列表 scope 过滤一致的心智）。频道行形态参照会话行（`conv-item` 结构：标题 + 统筹者 tag +
  生成中指示/时间），前置 Lucide `Hash` 图标区分。
- **rail 态**（`Sidebar.vue:411-525` 独立 v-else 树）：`ProjectSwitcher collapsed` 图标钮下方加频道图标钮
  （散落 scope 无活频道时隐藏）；两棵树互锚「同款同步改」注释纪律照旧。
- `isUserChat` 谓词（`Sidebar.vue:192`）排除 `kind === 'channel'`——频道不进普通会话列表（单一改点）。

### 6.2 频道页（复用 Home 路由 + kind 驱动分支）

无新路由（会话选择纯 store 驱动，`selectConv` 现有路径零改动）。`kind='channel'` 分支：

- **ChatHeader**：kind 徽章（「项目频道」，delegation 徽章先例 `ChatHeader.vue:279-290`）+
  统筹者胶囊（名字 + shield 图标，点开小 popover：成员清单 + 每人角色 + 「设为统筹者」/
  「让系统补选」入口）+ 收件箱入口隐藏（频道不收投递）。
- **ChatMessages**：`MessageGroup` merge 条件（`ChatMessages.vue:697-701` 现为 `assistant && 同 model`）
  **加 sender 维度**——不同成员的回复不互相吞并；每消息组行级 `EntityAvatar`（sm）+ 成员名字
  （caption）+ 统筹者 shield 微标 + **组级时间「开始 → 完成」**（相对时 + hover 绝对时，C8b derive
  透传的时长字段）；**用户消息落在前一回合区间内 → 气泡标注「生成中发出」**（C8b）。系统事实提示条
  （护栏拦截/选举叙事/歧义说明）从事件侧渲染——轻量居中条形态（新组件 ChannelNotice，Lucide `Info`
  + 单行文案，不占消息权威位）。
- **ChatInput**：@ 弹层候选源换项目成员（三段候选砍成一段；排除逻辑沿用）+ 发送钮旁常驻
  caption 提示「无 @ 时由统筹者接令」；mention chip 形态沿用 pendingRefs 视觉（数据通路独立，C9）。
- **归档频道只读态**（§2）：ChatInput 隐藏、头部标「已归档 · 原项目已删除」、成员/统筹者显示靠
  sender 元数据快照；统筹者 popover 治理入口隐藏（组织已随项目消亡）；后端 send 拒绝三段式兜底。
- **在途发送行为差异（C8）**：频道会话 sending 期间发送**不再走 T2 在途拦截横幅**——消息成功落流
  （后端物化无条件成功），气泡照常出现，输入框上方出一条轻提示「当前回合结束后处理」
  （ChannelNotice 同形态，回合结束即消）；1v1 会话行为不变（拦截 + 重试横幅照旧）。
- **streaming 流入**：外部回合判据管道现成（`useChatEvents.ts:66-75` `!ucb && sendingConvId !== cid` →
  sending 置位 + 权威 loadMessages）——频道回合（含选举投票回合）自动 live 流入，零新机制；
  在途落流的用户消息同经 `session:event-appended` → loadMessages 权威刷新进列表（不走本地占位 push）。

### 6.3 bridge 增量

`channels` 组：`ensure(projectId)` / `get(projectId)`（频道行 + 成员 + 统筹者 + 链状态投影）/
`setCoordinator(projectId, agentId | null)`；`chat.sendMessage` 扩 `mentions` 参数；
`conversations.create` 不动（频道只经 ensure 建）；conversations 列表出参透传 `archived_at`
（新列）——前端归档频道行与活频道判定。

---

## 7. 测试策略

**Rust（channel.rs 内联 + 集成）**：
- 路由纯函数：mentions 解析/成员校验/上限截断/串行序（表驱动）。
- 护栏：链计数 / 对重复 / 频率窗口 / 用户抢占取消（内存态构造驱动）。
- @ 文本解析：精确匹配 / 最长匹配 / 重名歧义不触发 / 非成员名忽略。
- 选举：计票 + 平票 joined_at 裁决 / 全员失败诚实报错 / role 幂等覆盖（伪造库 fixture，
  沿 53 号伪造先例补 project_agents 最小表）。
- e2e（session_runner_e2e 同款 MockProvider）：广播→统筹者接令→分派 @→成员接力→链上限截断全链断言
  （消息行 + 事件序 + channel_mention hop 序列）；用户新链头打断接力场景。
- 守卫：删 agent 迁频道 agent_id / 最后成员拒删 / **项目永久删除频道归档**（`delete_conversations`
  两旗标下频道均归档保留）+ 归档频道 send 拒绝三段式 / 活频道判定 SQL。
- 事实简报：三场景注入断言（积压/护栏拦截/广播降级——回合 prompt 含事实块，且不产生 messages 行）。

**vitest**：Sidebar 频道区块双形态 + scope 跟随 + isUserChat 排除 + **搜索框移除回归（两棵树均不渲染
搜索 UI）** + 散落 scope 归档频道行；ChatMessages 分组 sender 维度 + 行级头像；ChatInput mention
弹层与发送组装；ChannelNotice 渲染；轨迹词表三 kind；归档频道只读态（输入隐藏 + 头部标注）。

---

## 8. commit 拆批（4 批）——全部落地（2026-09-10）

1. ✅ `feat(channel)` 41d2bdc 后端地基：migration 54 + channel.rs 引擎（路由/接力/护栏/触发点）+ 事件三 kind +
   derive skip + sender 元数据双轨 + delegate 注册放开（当批口径，⑦ 批翻转见 C7）+ 删除守卫与项目删除频道归档 + Rust 测试（cargo 1485）。
2. ✅ `feat(channel)` cf4b3db 选举与换帅：自选举流程 + 两档治理 + system prompt 频道段（channel_hint）+
   频道命令四件（ensure/get/setCoordinator/reelect）+ 测试（cargo 1488）。
3. ✅ `feat(chat)` e129d46 前端：侧边栏改版（含搜索框移除与归档频道入口）+ 频道页渲染分支（Header/Messages/
   Input/Notice + 归档只读态）+ mention 通路 + bridge + vitest +25（547）。
4. ✅ `docs`：CHANGELOG [Unreleased] / CLAUDE.md 频道节 + 架构树 + 当前状态 / backend-api-reference
   模块十一 + 本设计稿状态更新。

**落地时实现取舍（与本文的细微出入，均以代码为准）**：护栏实值 = 链 8 / 有序对 2 / @5 / 频率 600s·6 /
统筹失败 streak 2（§5 表同源）；选举投票 max_tokens 128；换帅/罢免后投影一律回落 joined_at 最早成员
（ensure 同款语义）；sender enrichment 走 list_messages 派生回填（`sender_agent_name` 快照 +
`turn_duration_ms`，非表列——migration 零新列）。

## 9. v1 边界（明确不做）

per-agent 位点与任务段投影（v2——原 multi-agent-architecture §6.2 图纸）；并行成员回合（永远串行）；
用户消息排队（C8 打断语义替代）；主动进展概述与旁听回合（C6）；频道内 relay 工具与频道作为投递目标；
跨项目/多频道/散落频道（C1 锁死）；@全员；成员子集讨论；消息编辑/撤回；频道头部「总结近况」按钮
（手动 @统筹者 即可，等真实使用频率再固化）；选举自动重试（全灭报错指路）；护栏常量进配置（L1）；
「生成中发出」标注与区间显示只做频道页（2026-09-10 拍板）。

**已表态后置的未来方向（勿当成被否决）**：**1v1 会话的在途插话**——任务进行中允许用户发消息插入
（CC 的 mid-turn 语义：消息注入在途回合，agent 当前步骤完成后即见，不等整个回合结束）。用户已明确
有意做、往后放。与频道版的架构连续性：共享 C8 的「物化无条件成功」前半段与 C8b 的时间区间记录，
差异只在消费时机（频道 = 回合结束后新链头消费；1v1 = loop 轮间检查注入在途回合）——频道 v1 落地
后此方向是增量演进非推翻。

## 10. 风险与对策

| 风险 | 对策 |
|---|---|
| 删统筹者级联删频道历史（agent_id CASCADE） | §2 守卫 1：删除路径迁移 agent_id，无成员可迁拒删 |
| 项目删除频道转散落破坏边界 | §2 守卫 2：永久删除时频道归档保留（archived_at），Modal 文案披露 |
| 成员回复误触发 @（提名字非指令） | 礼节段教 + 只认成员名精确匹配 + 护栏截断；误触发代价=一回合，可接受 |
| 选举成本（成员多时逐个跑） | 不设成员数上限（健康检查价值 > 成本；项目成员规模天然小）；投票 max_tokens 128 |
| 接力触发与 chat_state 竞态 | 等静默轮询复用 inbox drain 模式；start 失败=忙→重试，结构性无并发 |
| agent 名重名 @ 歧义 | 不触发 + 系统事实说明（诚实不猜）；用户侧结构化 mentions 无此问题 |
| 频道回合烧 token 不可见 | 0.7 批③ 可观测化已就位（BudgetPill/上下文组成自动覆盖成员回合）；侧栏频道行生成中指示 |
| 记忆污染（多成员共享流对 agent 记忆的影响） | v1 频道回合不接 MemoryStage 摘要写入？——**接**（与 1v1 同链路，摘要按会话维度本来就是共享的）；观察池跟踪 |

## 11. 确认点拍板记录（2026-09-10 用户逐条拍板，全部关闭）

1. **搜索框**：**整体移除**（不是移位）——目前没有使用它的场景（§6.1：展开树与 rail 树两处搜索
   一并删，过滤逻辑随之退役）。
2. **频道区块跟随 scope**：确认——项目 scope 显示该项目频道；散落 scope 显示归档频道（归档方案
   落地后入口在此）。
3. **频道随项目删除**：**软删除/归档，不物理删除**——`conversations.archived_at`（migration 54
   顺路加列），处置矩阵与只读形态见 §2「频道归档」节；清理出口 = 归档频道行两步确认删除。
4. **护栏数值**：按 §5 初版值走，**实际使用过程中按真实情况优化**（触发频率/误拦率驱动，观察项在案）。
5. **系统事实不物化消息行**：从 append-only 角度重审后确立为**分层原则**——内容性事实物化（带
   sender 权威）、行为性事实只进事件、对 agent 告知走派生简报（C10b 全文 + 日志完整性论证）。
6. **频道在途发送不拦截**：确认——按 C8 方案「真实还原用户群聊体验」（消息无条件落流 +
   「当前回合结束后处理」轻提示；在途回合不感知中途插入，要立即停走停止按钮）。
