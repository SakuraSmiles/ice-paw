# 多 Agent 协作与项目维度轨迹 — 整体设计

> 状态：**设计稿（待评审）** · 2026-08-15
> 范围：MA-1 委派会话化 → MA-2 任务台账+项目轨迹 → MA-3 持久通道+背景消息互通
> 前置阅读：`docs/architecture.md`、CLAUDE.md「会话事件日志」「配置提案系统」两节

---

## 0. 一句话

把现存的隐形 `delegate_to_agent`（单轮无记录 LLM 调用）升格为**真会话**：人/agent 之间的一切交互都是 session、全量落事件日志、人随时可审计；任务台账与项目轨迹从事件日志**派生渲染**（真相在产物）；agent 间经持久通道互通（图协作，不设中心协调员强制路径）。

## 1. 愿景不变式（设计的宪法，2026-08-14 已锁定）

1. **统一 Session**：人↔agent、agent↔agent 的交互都是 conversations 表里的一行，无「私聊」特殊类。用户对全部交互可审计。
2. **agent 间是图**：委派/通道都是图的边，允许不经协调员直连。
3. **委派边界一律 session，无 event 逃生口**：任何 agent 间协作必须在 session 内发生，禁止隐形 LLM 调用不留痕。
4. **真相在产物**：任务台账、项目轨迹都是 session_events 的**派生视图**（可随时重建），不另立真相源。
5. **日志无损**：session_events append-only 永不删改；压缩只作用于喂给 LLM 的实时窗口 projection（TokenWindowStage / 滚动摘要），不作用于日志。
6. **产物归属仲裁复用 proposal / ToolAuth**：多 agent 写同一 workspace 的冲突走审批卡片，不新增仲裁机制。
7. **展示/逻辑分离**：会话类型只影响渲染形态（聊天/任务卡/通道），不影响底层会话模型。

## 2. 现状盘点

### 已就位（本设计的复用面）

| 地基 | 位置 | 对本设计的意义 |
|---|---|---|
| append-only 事件日志 + derive-on-read 主读路径 | `session_events`（migration 44）+ `read_route.rs`（6d3c948） | 新会话类型零成本获得完整历史/回放/对账 |
| 13 kind typed emitters + supersede | `harness/event_log.rs` | 委派生命周期事实全部有落点 |
| 轨迹 UI（表格+检查器+瀑布图+live 推送） | `TrajectoryView` 等 + `session:event-appended` 总线（aa96e16） | **任意会话类型免费获得轨迹页**；总线注释明写「为多 agent 后台事件铺路」 |
| loop_engine 全套熔断 + B1 自动续期 | `loop_engine.rs` + `budget.rs`（bdc6339） | 委派子会话独立预算、长任务不饿死 |
| per-conversation 隔离 | ChatState cancel token / BatchWriter / PathAuthSession per conv | 并发多 loop 结构上已支持 |
| 工具授权 payload 带 conversation_id | `ToolAuthRequestPayload.conversation_id` | 后台会话的授权请求可路由 |
| agent 自持 provider/model/api_key | `AgentRow` + `agent_cmd.get_with_credentials` | 多模型委派（专家用自己的模型）数据现成 |
| 审批通道 | `proposal_registry` + `ToolAuthRegistry`（oneshot 模式同构） | 不变式 6 直接复用 |
| conversations 已有 project_id | migration Phase 2 | 项目维度挂接点现成 |

### 缺口

- **`delegate.rs` 是隐形 MVP**（173 行）：主 agent 的 provider/key + 单轮无工具 + 60s 超时 + 结果只进 tool_result，无 session 无事件——违反不变式 3，是 MA-1 要替换的对象。
- conversations 无类型/发起者/父会话字段；会话列表无类型过滤。
- 无跨会话游标（seq 是 per-conversation 的）；项目页无轨迹/任务视图。
- 无 channel 参与者模型、无 per-agent 消费位点。

## 3. 核心模型：统一 Session 三类型

```
conversations
├── kind = 'chat'        用户 ↔ agent（现有全部会话；旧数据默认值）
├── kind = 'delegation'  agent → agent 委派子会话（MA-1）
└── kind = 'channel'     持久通道会话，多参与者（MA-3）
```

所有类型共用：messages、session_events、轨迹页、预算熔断、hooks、读路径路由（read_route 不感知 kind，派生路径自动覆盖新类型）。

## 4. MA-1 委派会话化

### 4.1 Schema（migration 45）

```sql
ALTER TABLE conversations ADD COLUMN kind TEXT NOT NULL DEFAULT 'chat';
ALTER TABLE conversations ADD COLUMN initiator_type TEXT;          -- 'user' | 'agent'；NULL ≡ 'user'（旧数据）
ALTER TABLE conversations ADD COLUMN initiator_agent_id TEXT;      -- delegation：发起委派的 agent
ALTER TABLE conversations ADD COLUMN parent_conversation_id TEXT
  REFERENCES conversations(id) ON DELETE SET NULL;                 -- 委派图边；父删边不删子（审计保留）
CREATE INDEX idx_conversations_kind_project ON conversations(kind, project_id);
```

原则：**只加可空列/带默认值列**，旧读路径零迁移；不加 FK 到 agents（agent 可删，会话须活得比 agent 久，同 session_events 无 FK message_id 的先例）。

> **实施修正（2026-08-15，migration 45 落地时）**：不新建 `project_members` 表——项目成员已有 `project_agents` 表（migration 13）+ 完整 CRUD 与管理 UI。可调度集合（4.3.1）直接复用 `project_agents`，零新表零迁移。

### 4.2 session_runner 抽取（本次唯一的结构重构）

`send_message` 命令体现有 ~400 行编排（取 agent+creds → provider → Pipeline 上下文 → 占位 → spawn_stream_loop）。抽出可复用内核：

```rust
// harness/session_runner.rs
pub struct AgentTurnInput {
    pub conversation_id: String,
    pub user_content: String,            // 委派任务文本 / 用户消息
    pub parent_cancel: Option<CancellationToken>,  // 委派：父 loop 的 cancel（级联取消）
    pub auth_registry: ToolAuthRegistry, // app.state 共享实例
    pub tool_registry: McpRegistry,      // 目标 agent 视角的组装结果
    /* app/pool 经 AppHandle state 取（agent_cmd/mcp_manager/global_registry） */
}
pub struct TurnOutcome {
    pub finish_reason: String,           // stop / budget_exceeded / ...（复用词表）
    pub final_text: String,              // 回传给委派方的正文
    pub rounds: u32,
    pub usage: Option<TokenUsage>,
}

pub async fn run_agent_turn(app: &AppHandle, input: AgentTurnInput)
    -> AppResult<TurnOutcome>;   // inline await 完成（不 spawn 后失联）
```

要点：
- **完成信号**：`spawn_stream_loop` 增加可选 `oneshot::Sender<TurnOutcome>`，在 cleanup 守卫（RAII drop）前发送——任何退出路径（正常/取消/panic-drop）都会送达，委派方不会悬挂。
- `send_message` 命令改为调 `run_agent_turn`（行为等价，回归靠 719 lib tests + 真机）；`read_route`/摘要等下游不动。
- 取消级联：父 cancel → `parent_cancel.child_token()` 作为子 loop 的 token，「停止生成」一键停整棵委派树。
- api_key/provider：走 `agent_cmd.get_with_credentials(&target_agent_id)`——**专家用自己的模型**（多模型兑现点），与主 agent 无关。

### 4.3 delegate v2 生命周期

```
主 agent 调 delegate_to_agent(agent_id, task)
  1. 校验：target 存在、≠ 自己、∈ 可调度集合（4.3.1 回退规则）、有可用 key
     （失败诚实回 Err 给主 LLM）
  2. 建子会话：kind='delegation', agent_id=target, project_id=父会话的,
     initiator_type='agent', initiator_agent_id=主 agent, parent_conversation_id=父,
     title="委派: {task 前 30 字}"
  3. 子会话落 user 消息（task）+ 事件日志（log_user_message——13 kind 现成）
  4. run_agent_turn(child)：专家跑完整 loop（可调工具、独立预算+B1 续期、
     自己的 hooks：ConversationStart/BeforeLlm/AfterTool/ConversationEnd 全生效）
  5. tool_result 回传主 agent：
     { child_conversation_id, agent_name, finish_reason, response }
     ——主 LLM 拿到正文 + 子会话 id（可在后续轮引用/续派）
  6. 超时：去掉 60s 硬超时，换壁钟护栏 15min（子 loop 自身的 budget/stuck 才是主要终止器）
```

授权（2026-08-15 评审更新）：**项目组内免弹窗**——`AuthorizationLevel::Silent`，加入当前项目组的 agent 均可被自由调度（v1 不设特殊限制）。失控保护改为结构性护栏：
- **委派深度上限 = 1**（评审定稿：只允许对话 agent 向其他 agent 委派一次，接收方不能再委派）：工具注册**按会话类型判定**——kind='chat' 才注册 delegate 工具，delegation 子会话一律不注册。无需传递深度计数器，「A 委派 B、B 委派回 A」的乒乓球委派在结构上不可能；子会话独立预算 + B1 续期封顶开销。
- 可调度集合校验（target ∈ 解析结果，见 4.3.1）替代授权弹窗成为边界。
- 子会话内部工具授权不变（各自独立 PathAuthSession，敏感操作仍过用户手，弹窗路由见 4.5）。

#### 4.3.1 可调度集合（项目成员与散落会话的回退规则）

```
解析 dispatchable_agents(conversation)：
  1. conv.project_id 存在 且 project_agents 非空 → 该项目成员集合
  2. 否则（无项目 / 默认项目 / 成员为空）→ 全部 agent   ← 散落会话零摩擦回退
```

- **设计意图**：想管细的项目去项目页配团队；不想管的（含全部存量散落会话）一行配置不动、天然全量可调度——可选配置而非强制前置，向后兼容零迁移。
- 主 agent 的「能调度谁」感知：system prompt 注入可调度清单（agent 名 + 专长摘要，`context/system_prompt.rs` 现有委派提示扩展）。
- 自我委派仍禁止（现有校验保留）。

### 4.4 并发与 fan-out（明确 v1 边界）

- `execute_tool_round` 顺序执行工具（`for` 循环）→ 同轮多个委派**串行**。v1 接受此边界（专家调用天然分钟级，串行可预期）。
- 并行 fan-out（join_all + per-call 超时）列为 **MA-1.5**，待 MA-1 真机验证后按需排期。事件/预算/授权结构均已就绪，纯执行器层改动。

### 4.5 后台会话的工具授权路由

- 后端零改动（payload 已带 conversation_id）。
- 前端：授权请求的 conv ≠ 当前打开会话时，**不再绑死聊天页弹窗**——全局横幅队列「后台委派请求授权：write_file @ <会话名>」，点击跳该子会话。超时/拒绝路径与现有一致（oneshot 超时回 Err）。
- v1 从简：一次一个横幅（串行工具执行天然不会并发弹）。
- **展示方式不做过细设计**（2026-08-15 评审）：用户对现有授权确认框观感本就不满意，后续会专门重做授权 UI（含前台弹窗与后台横幅的统一形态）；MA-1 临时沿用现有机制，只保功能正确（可批/可拒/超时回 Err）。

### 4.6 UI 呈现

- **侧栏会话列表只显示 kind='chat'**（后台会话不污染用户主列表）。
- 父会话消息流：`delegate_to_agent` 的 tool_result 渲染为**委派卡片**（进行态实时跳轮数/工具数/token，完成态含结果摘要 + 「查看轨迹」跳子会话轨迹页）。⚠️ 卡片具体形态**不锁设计——实现期用户看着效果边看边调**（2026-08-15 评审拍板），初版只保最小要素：目标 agent / 状态 / 跳转。
- 子会话轨迹页：现有 TrajectoryView 直接打开（conversation_id 路由现成），live 推送免费生效。
- **极简任务列表进 MA-1**（入口保障：侧栏藏了 delegation 会话，必须有可达路径）：项目页加只读列表 = kind='delegation' 会话 + 状态点 + 点开跳轨迹；MA-2 长成完整台账。

### 4.7 验收标准（MA-1）

1. 委派全程在 session_events 有完整记录（子会话事件独立成流，可导出 JSONL）。
2. 专家用自己的 provider/model（日志 turn_context 可证）。
3. 委派免弹窗且按可调度集合校验：配了成员的项目外 agent 被拒（诚实 Err 回主 LLM）；散落会话/空成员全量可调度。
4. 委派深度 = 1：delegation 子会话不注册 delegate 工具（接收方不能二次委派，乒乓球委派结构上不可能）。
5. 专家可调工具；子会话敏感工具授权经横幅路由可批可拒。
6. 父「停止生成」级联停子会话；子预算独立且 B1 续期生效。
7. 侧栏无后台会话；父会话卡片 + 项目任务列表双入口可达子轨迹。
8. 旧会话（无 kind）行为零变化；read_route 对子会话走 derive 且 reconcile 零 diff。

## 5. MA-2 任务台账 + 项目轨迹

### 5.1 任务 = 派生视图（不变式 4）

不建 task 表。**任务 ≡ kind='delegation' 会话**：

- 状态机：`running`（最新 turn 未 ended）→ `done/failed/stopped`（子会话 turn_ended.finish_reason 映射）→ 回传成功另计 `returned`。
- 查询：`conversations WHERE kind='delegation' AND project_id=?` + 子会话最新 `turn_ended` 事件（一次 join/子查询）。
- 派生结果可缓存（内存/表皆可），**缓存必须可从事件日志整体重建**——这是「真相在产物」的可验证含义。

### 5.2 项目轨迹 tab（跨会话聚合渲染）

- 游标：seq 是 per-conversation 的，跨会话用 **session_events.rowid 全局序**（单写池下近似真实时序；同事务多行有稳定相对序）。
- API：`project_trajectory_tail(project_id, before_rowid, limit)` —— conversations 过滤 project_id 后 UNION 事件流，尾部页语义同 `list_tail`。
- 前端复用 TrajectoryView：加「会话/发起者」列（chat/delegation 分色徽章）；瀑布图泳道按 **agent** 分道（User/Model/Tools/Hooks 四道改为 per-agent 分组的 Model/Tools 道——具体布局 MA-2 设计时定稿）。
- 规模：项目轨迹复用千轮债务的全部成果（虚拟行/搜索缓存/像素聚合），游标分页天然兼容。

### 5.3 轨迹系统的复用与扩展清单（明确边界）

**零改动直接复用**：TrajectoryView 全套渲染、虚拟行/搜索缓存/像素聚合（千轮债务成果）、live 推送（总线按 conversation_id 过滤，多会话并发各推各的）、单会话导出 JSONL、read_route 派生读路径（子会话同构覆盖）。

**MA-2 需要扩展的**（项目轨迹 = 跨会话聚合后的新维度）：
1. **瀑布图泳道**：现 4 泳道（User/Model/Tools/Hooks）是单会话单 agent 假设；项目轨迹改 **per-agent 分组泳道**（agent 数多时折叠/着色，布局 MA-2 设计时定稿）。
2. **轮号（M3 全局偏移）**：`count_turns_before` 是 per-conversation 的；项目轨迹的分组单元从「轮」升为「会话×轮」，偏移查询按会话分组各自取。
3. **工具栏过滤维度**：新增按 会话类型（chat/delegation）/ 发起者（人/agent）/ agent 过滤的分段按钮（现有分段按钮框架扩展）。
4. **委派边的呈现**：父会话 tool_execution（delegate 调用）与子会话首事件的关联渲染——子会话行加「↳ 委派自 <父>」徽章 + 点击跳父；委派树的全局图视图（graph render）不在 MA-2 范围，留 MA-3 后按需。

**已知边界（v1 接受，明示不隐藏）**：
- **后台会话无 ephemeral 流式行**：ephemeral 行依赖 chat store 单活跃会话的 streaming 状态；子会话非活跃时其轨迹页仍有 live 事件行（落库即推），但未落盘的流式文本要等 assistant_message 事件落库才出现。后台观感 = 事件粒度直播，非字符粒度。MA-3 推模式（agent 自动唤醒）再评估是否值得做多会话 streaming 状态。
- **项目级导出**：单会话 JSONL 导出现成；跨会话项目轨迹导出（合并 rowid 序）列 MA-2 可选项。

### 5.4 验收标准（MA-2）

1. 项目页「任务」列表状态与子会话实际终态一致（含 failed/stopped）。
2. 项目轨迹可分页翻到项目创建时；搜索/耗时模式可用。
3. 删除派生缓存后全量重建，结果逐字节一致。

## 6. MA-3 持久通道 + 背景消息互通

### 6.1 模型

```sql
-- migration（MA-3 时点）：participants 是多对多，列式不够
CREATE TABLE channel_participants (
  channel_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  agent_id   TEXT NOT NULL,             -- 无 FK（同 4.1 原则）
  last_consumed_seq INTEGER NOT NULL DEFAULT 0,  -- ★ per-agent 消费位点（continuity 的锚）
  joined_at  TEXT NOT NULL,
  PRIMARY KEY (channel_id, agent_id)
);
```

- channel 会话的「消息」= initiator_type 标记的 user_message（'agent' 发起即 agent 发言；事件词表用现有 `user_message`，payload 加 `initiator` 字段，v 迭代而非新 kind——若评审认为语义过载，再立 `message_posted` kind，**开放问题 ①**）。
- 人的发言 = 同一 channel 里的普通 user 消息——人随时进入通道参与/旁观（不变式 1）。

### 6.2 任务段 continuity（持久通道 ⊃ 任务段）

agent 被（委派/定时/事件）唤醒时，喂给它的 channel 上下文**不是全量历史**：

```
channel 窗口 = 摘要(0 .. last_consumed_seq)   ← 该 agent 上次消费位点的滚动摘要
             + 增量事件(last_consumed_seq .. max_seq)  ← 全文
```

- 摘要复用 Phase 2 滚动增量摘要机制，但**位点 per-agent**（`last_consumed_seq`），不是 per-channel——各专家按自己的节奏消费。
- 消费位点在 agent 完成一轮处理时推进；压缩/摘要只发生在该 projection 上（不变式 5 的具体化）。

### 6.3 消息路由（事件总线驱动）

- 已就位的 `session:event-appended` 总线是路由底座：订阅 → 过滤本通道事件 → 触发目标 agent 处理。
- **v1 拉模式**：agent 不被消息自动唤醒；由用户/主 agent 显式委派或@触发，唤醒时带任务段上下文。防风暴（agent 互发消息无限循环）天然安全。
- **v2 推模式**（自动唤醒）：必须带三道护栏——速率限制（每 agent 每分钟唤醒上限）、通道静默期、递归深度计数（agent→agent 链深上限）。无护栏不上线。

### 6.4 产物归属仲裁（不变式 6）

多 agent 并发写同一 workspace 的产物冲突：`proposal_guard` 域扩展出「产物冲突」提案类型（Conflict → 审批卡片，用户拍板归属/覆盖/另存）。MA-3 与推模式同期引入；MA-1/2 串行执行下无冲突面。

### 6.5 压缩触发点（愿景遗留问题的落地答案）

| 触发点 | 对象 | 动作 |
|---|---|---|
| 委派唤醒时 | 任务段 projection | 按位点切摘要+增量（6.2） |
| 单轮发送前 | 现有 TokenWindowStage | 80% 硬裁（已上线，不变） |
| channel 写入后（异步） | 摘要物化缓存 | 增量 fold（covered_until 追踪） |
| **永不触发** | session_events 日志 | ——（不变式 5） |

## 7. 对现有系统的影响面（全量清单）

| 系统 | 影响 | 结论 |
|---|---|---|
| session_events 词表 | MA-1/2 零新 kind；MA-3 一个 payload 字段或一个新 kind | 兼容 |
| read_route / derive | 新 kind 会话同构走既有路径 | 零改动 |
| reconcile | 对账平面不感知会话类型 | 零改动 |
| hooks | 子会话跑目标 agent 自己的 hooks | 零改动（语义正确） |
| budget/B1 | 每次委派=独立 send，独立预算+续期 | 零改动 |
| 前端 chat store | 单活跃会话模型保留；后台会话只进轨迹/任务视图 | 增量 |
| 事件推送总线 | 已就绪 | 零改动 |
| execute_tool_round | 串行边界（4.4） | MA-1.5 再动 |

## 8. 实施顺序与里程碑

```
MA-1  schema45（会话类型四列 + project_members）+ session_runner 抽取 + delegate v2
      （Silent 授权 + 可调度集合回退 + 深度上限 1：子会话不注册 delegate 工具）
      + 授权横幅（临时沿用现有形态，授权 UI 后续专项重做）+ 委派卡片（实现期迭代）
      + 极简任务列表入口                                              ← 先做，1 个可控大块
      └ 验收后打包 0.3.5
MA-2  任务台账派生 + project_trajectory_tail + 项目页任务/轨迹 tab（场景 B）
MA-3  channel_participants + 任务段 continuity + 拉模式路由 + 产物仲裁（场景 C）
      └ 推模式（自动唤醒）带护栏，最后
```

场景映射：**MA-1 = 场景 A（委派会话化）；MA-2 = 场景 B（任务台账+项目轨迹）；MA-3 = 场景 C（持久通道）**——逐个来，每阶段独立可验收、可打包；MA-1 打开的「委派=session」不变式是后两阶段的地基。

## 9. 风险与开放问题

1. **① channel 消息的词表表达**：`user_message` 加 initiator 字段 vs 新 `message_posted` kind——MA-3 设计时定（倾向后者：语义不耦合人类轮次概念）。
2. **② 并发 SQLite 写**：父子 loop 并发写同库。现有 pool 多连接已支持，但委派高峰下的锁竞争未测——MA-1 真机观察 `database is locked` 频率，必要时 busy_timeout 调优。
3. **③ 专家 agent 的 key 缺失/模型不可用**：委派失败的诚实回传路径（主 LLM 收到明确错误可换人/换法），不静默。
4. **④ 子会话生命周期管理**：委派会话永久保留 vs 项目归档联动——v1 永久保留（日志无损优先），归档联动随 MA-2。
5. **⑤ 前端多会话并发流式**：父子同时 streaming 时事件按 conversation_id 分流已成立，但「委派进行中」的父气泡观感需设计（进行态卡片而非干等）。
