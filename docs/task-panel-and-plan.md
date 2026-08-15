# 任务面板与计划（MA-1 UX 打磨 + update_plan）实施蓝图

状态：已评审定案（2026-08-15），实施中。
前置讨论：任务入口设计讨论（四层概念模型）+ 计划 vs 任务概念辨析，结论已锁定如下。

## 0. 概念定位（已锁定，不再重议）

- **会话** = 用户参与的事件流（存储原子，append-only）；**任务** = agent 自主发起的工作单元（**执行单元**，本身就是会话，kind='delegation'）；**计划** = **意图文档**（会话内容，以事件形态存在）——正交于任务，靠 `task_conversation_id` 引用边关联。
- 四否证（计划 ≠ 任务）：无计划的任务存在；无任务的计划存在；一条目 0..n 任务（重派）；勾选是 agent 判断、任务状态是事件派生。
- 入口三层不重复：**消息流卡片**（就地锚点，读到哪看到哪）→ **任务胶囊**（会话级索引，本会话派生的任务 + 当前计划）→ **项目台账**（MA-2 跨会话全局）。
- 已拍板的 UI 决策：任务详情默认落**轨迹** tab；胶囊只显**本会话**任务；会话级任务列表**不分页**（状态优先 + 时间倒序，超 8 条截断）；分页维度律 = 轮次是会话内因果单位 / 时间是跨会话唯一序轴 / 状态是任务列表第一排序键。

## 1. 范围与不做

**做**（5 个 commit）：

| # | 内容 | 层 |
|---|---|---|
| C1 | 任务详情 v1：子会话头返回父会话 + 状态徽章升级 | 前端 |
| C2 | `chat:delegation-started` 推送：委派**开始**即知 child id（运行中卡片可跳） | 后端小件+前端 |
| C3 | 任务胶囊 + popover（任务列表段） | 前端 |
| C4 | 计划后端：`plan_updated` 事件（第 14 kind）+ `update_plan` 工具 + derive 容忍 + `get_session_plan` 命令 | 后端 |
| C5 | 计划前端：PlanCard（对话流）+ popover 计划段 + 轨迹 PLAN 行 | 前端 |

**不做**（v1 边界，全部有明确归属）：

- 任务 done/failed 精确终态 → MA-2 台账（`turn_ended` 派生状态机）；v1 只有 进行中（streamingConvIds）/ 已结束 两态，**不伪造**
- 并行 fan-out、父回合结束后子任务仍在跑的「输入框上方任务条」→ MA-1.5
- 跨会话计划聚合 / 项目级计划全貌 → MA-2 台账
- 右侧常驻 dock → 远期（监控台形态，并行任务多了再议）
- 用户手写/编辑计划 → 未来（词表已兼容 actor=user，无 UI）
- 子会话（专家）计划不上浮到父会话——只活在子会话日志（边界已定）

## 2. C1 任务详情 v1

**问题**：委派子会话无回路（用户已反馈）；「委派会话」徽章是静态死信息。

- `Conversation` 前端类型补 `parent_conversation_id?: string | null`（后端 ConversationRow 已有此列，序列化自动带出，零后端改动）。
- `ChatHeader.vue`：kind='delegation' 时——
  - 左侧加「← 返回 {父会话标题}」按钮（`selectConversation(parent_id)`，落「对话」tab，与 openTrajectoryNext 语义对称）；
  - 原静态「委派会话」徽章升级为「委派任务」+ 状态点（进行中脉冲 = `streamingConvIds.has(convId)`；已结束中性点）。
- `DelegationCard.vue` 文案「查看轨迹」→「打开任务」（跳转行为不变：`openConversationAtTrajectory`，默认落轨迹 tab——已拍板）。

**不变式**：parent 恒存在（深度=1 结构护栏保证 delegation 子会话必有 chat 父）；父会话必在侧栏列表（kind='chat'）。

## 3. C2 delegation-started 推送

**问题**：`child_conversation_id` 只在完成时的 tool_result 回传 → 运行中卡片/任何入口都无法跳进子会话看现场。

- 后端 `delegate.rs`：子会话创建成功后 `app.emit("chat:delegation-started", { conversation_id: 父, child_conversation_id, agent_name, title })`（在 run_agent_turn spawn 前，inline）。
- 前端 chat store：listen 该事件 → `loadConversations()`（子会话行即刻入库，列表刷新后胶囊/卡片可见）。
- `DelegationCard` 运行中态：childConvId 取「本会话当前运行中的 delegation 子会话」——**v1 串行执行保证同父同时至多一个运行中委派**，无歧义；`ChatMessages` 取数层从 store 算出传入，卡片零改取数逻辑（仅接受运行中也有 childConvId）。

## 4. C3 任务胶囊 + popover

**位置**：ChatPage 标签条右侧（`margin-left:auto`），`[◉ 任务 N]` 胶囊。

- **显隐**：本会话 delegation 子会话 > 0 **或** 当前计划存在（C5 后并入）。
- **胶囊**：数 = 任务数；有进行中 → 呼吸点。
- **popover 两段**（C5 前 Only 任务段）：
  - 计划段（C5）：当前计划快照（勾选态 + 条目挂任务的跳转箭头）
  - 任务段：「任务（本会话）」状态点 + 标题（`委派: …`）+ 相对时间；点击 → 任务详情（= openConversationAtTrajectory）
- **排序**：进行中置顶，其余 updated_at 倒序；>8 条截断 + 「查看全部 → 项目」（跳项目列表，已有极简任务列表）。
- **数据**：全前端派生（`chat.conversations.filter(kind==='delegation' && parent_conversation_id===activeConvId)` + `streamingConvIds`），零新后端。
- 新组件 `components/chat/TaskPanel.vue`（胶囊+popover 一体），ChatPage 挂载。

## 5. C4/C5 计划（update_plan 全链）

### 5.1 事件词表：第 14 kind `plan_updated`

```jsonc
// session_events 行：kind='plan_updated'，actor='agent:<id>'，turn_id=本 turn，
// message_id=null（工具执行的 assistant 关联由同 turn 的 tool_execution 事件承载）
{
  "v": 1,
  "items": [
    { "text": "调研现有渲染方案", "status": "pending | in_progress | done",
      "task_conversation_id": null }   // 可选：条目挂的委派子会话（跳转用）
  ]
}
```

- **全量快照语义**：每次调用整体覆写，回放 last-wins（与 assistant_message supersede 同款心智，无增量合并复杂度）；空数组 = 清空计划。
- actor 恒 agent（v1 只有 agent 调工具；词表兼容未来 user）。

### 5.2 `update_plan` 内置工具

- 注册：`register_builtin`（**全局注册表**——委派子会话的专家也能维护自己的计划，事件天然落子会话日志）；`AuthorizationLevel::Always`（无会话外副作用，不弹窗）。
- 参数（TodoWrite 同款形状，数组 + 平对象，兼容弱 schema 模型）：

```jsonc
{ "steps": [ { "text": string(必填,≤200字), "status": "pending|in_progress|done"(必填),
               "task_conversation_id": string(可选) } ],   // maxItems 30
  "required": ["steps"] }
```

- description 明示**全量覆写**语义 + 使用时机（开始多步任务时建立、每步完成即更新勾选、不达标可重派并保持条目 in_progress）。
- 校验失败回**教学型错误**（指出哪条违规 + 正确形状），模型可自修复重试。
- 执行：校验 → 构造 EventCtx → `log_plan_updated`（inline await，warn-only，硬规则同全体事件）→ 返回确认 JSON（含条目数）。

### 5.3 事件链路适配（三处，缺一则污染读路径）

1. **`derive.rs` 回放器**：skip 臂（现 148-150 行的「非消息行事实」清单）加 `"plan_updated"`——否则每个用计划的会话记 DeriveIssue → reconcile 出 DERIVE_ISSUE diff → read_route 永久回退 Legacy。
2. **`reconcile.rs`**：核查 kind 清单容忍（对账平面是行级，plan 不产行；确认分类器无需改 + 单测覆盖）。
3. **`ToolContext` 加 `turn_id: Option<String>`**：`execute_tool_round` 富化注入（`ev.turn_id`，与 app_handle/proposal_registry 同点）——工具内构造 EventCtx 必须有 turn_id，否则事件落 NULL → 轨迹归「纪元前桶」（错分组）。

### 5.4 `get_session_plan` 命令

- `get_session_plan(conversation_id) -> Option<{ items, updated_at }>`：取该会话**最后一条** plan_updated（快照语义即当前态）。
- live：event_bus → `session:event-appended` 前端已监听，kind 过滤 `plan_updated` 时刷新。

### 5.5 前端

- **PlanCard**（`components/chat/PlanCard.vue`）：`ChatMessages` 对 `update_plan` 工具调用双路径渲染（历史 parseToolUseBlocks / 流式 toolCallList，同 DelegationCard 取数模式）；条目勾选态 ○/●/✓，头「计划 N/M」，挂任务的条目带跳转箭头（→ 任务详情）。
- **popover 计划段**：`get_session_plan` 快照 + 事件驱动刷新（见 5.4）。
- **轨迹 PLAN 行**：`RowKind += "plan"`（label PLAN，默认**可见**——非 aux：计划变更是有信息量的会话事件）；summary = `计划 2/5 · 首条目文本…`；inspector 读 payload 原文。
- **types**：`SessionEvent` 联合 += plan_updated 分支 + `PlanUpdatedPayload` / `PlanItem`。

## 6. 验证清单

- [ ] `cargo check` / `clippy --tests -- -D warnings` 0 警告 / `cargo test --lib` 全绿（新增：event_log plan round-trip、derive 容忍、update_plan 校验、reconcile 不受影响）
- [ ] `pnpm typecheck && pnpm lint && pnpm test && pnpm build` 全绿
- [ ] 手测：①委派运行中卡片可跳子会话、子会话头「返回父会话」回到对话 tab；②任务胶囊计数/脉冲/popover 排序与跳转；③长任务里 agent 建计划 → 卡片演进勾选、胶囊计划段同步、轨迹 PLAN 行出现；④Phase 2A 无回归（用计划后会话仍走 Derive 路由——`get_read_route_status`）
- [ ] commit 拆分：C1..C5 各一 commit，无 brand/ 混入，无 Co-authored-by
