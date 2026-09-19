# Steer（生成中插话）设计小稿 v1

> **状态：已实施（随 0.9.1 发版）；2026-09-19 W1 批修订——搁浅族根治（marker 记账 + 令牌快照 + boot 扫尾，见 §5/§8/§10.3/§12）。** 2026-09-18 用户拍板：Steer 先行，长期记忆作为未来线路。
> 本文档只定「生成中用户发送一条消息、agent 在下一工具边界改道」的机制，**不写代码**。
> 方向 = **无状态 + 路线乙**（拆两道 turn 保留上下文），明确**不引入世界状态/长期记忆**。

---

## 1. 目标与非目标

**目标**：1v1 会话中，agent 正在生成时，用户仍能发送消息；该消息在**下一个工具边界**生效，agent 据此改道，已完成的工具轮不丢。

**非目标**（本期不做）：
- ❌ 世界状态 / 长期记忆（`EnvironmentStage` / WorldState 差分注入 = 未来线路，见 §9）
- ❌ 回合中途预算复查（路线乙天然绕开，见 §7）
- ❌ 频道（频道已有 C8 在途插话，语义不同——Queue 非 Steer，不动）
- ❌ 轮间注入的「一道 turn」形态（路线甲，已否）

---

## 2. 核心机制（一句话）

**Steer = 「自动打断在途回合」+「turn_ended 后自动续跑」，两半都是现成能力**：
- 「打断」半 = 复用 `stop_generation` 已有的 `CancellationToken.cancel()`（`infra/cancel.rs` 的 AtomicBool 标志位，loop 在 yield 点轮询）；
- 「续跑」半 = 复用频道 C8 的「用户消息无条件物化 + DB 积压为真相 + turn_ended 后消费积压开新回合」。

**不引入任何新事件 kind、不改 loop 核心、不动 append-only 日志模型。**

---

## 3. 边界定义（关键）

cancel 标志位在 loop 两个 yield 点被轮询（[loop_engine.rs](packages/app/src-tauri/src/harness/loop_engine.rs)），steer 消息到达时**只置位 cancel，不杀流**，由 yield 点自然接管。两个 yield 点的行为：

| yield 点 | 位置 | 到点时的状态 | cancel 命中后 |
|---|---|---|---|
| **① 循环顶** | `'tool_round` 顶部（loop_engine.rs:280） | 上一轮 assistant 已 finalize + tool_result 已落盘；当前占位是「新鲜空行」 | `log_message_discarded(cancel_top_placeholder)` + `finalize_cancel`——**最干净边界**：无半成品、无孤儿 |
| **② 落盘前** | 阶段 C0（loop_engine.rs:621） | 本轮流式已完成（thinking+text+tool_use 已收进 `round_blocks`），**尚未落盘、工具未执行** | `finalize_guard_logged`（剔除 ToolUse 后落盘或删占位）+ `finalize_cancel`——**工具不执行**，模型本轮产出保留 text/thinking |

**结论**：steer 到达时刻决定落哪个点——
- 到达时正在**流式**（思考/文本/tool_use 输出中）→ 命中 ②，等模型说完本轮，**tool_use 被剔除、工具不跑**，立即收尾；
- 到达时正在**工具执行/结果落盘** → 命中 ①，上一轮（含 tool_result）完整保留，下一轮不启动。

两个点都由既有 `finalize_cancel`（[cleanup.rs:252](packages/app/src-tauri/src/harness/cleanup.rs)）对称收尾：`turn_ended(abort)` → `unregister` → `chat:done(abort)`，孤儿 tool_use 防线（`finalize_guard_logged`）已覆盖。

**语义对齐 Claude Code**：从不打断模型正在写的那一段（不杀流），只在「模型本轮说完 / 工具将跑未跑」处截断——这就是「下一个工具边界」。

---

## 4. 事件形态（零新 kind）

| 事件 | 形态 |
|---|---|
| 用户 steer 消息 B | **普通 `user_message`**，走现有物化路径（`repo::message::create` + `update_content_blocks` + 附件 + `log_user_message`），无新 kind、无特殊标记 |
| 在途回合 A 收尾 | `finalize_cancel` → `turn_ended(termination="abort")`——与手动停止**完全相同** |
| 新回合 B | 普通 `run_agent_turn(pre_materialized=true)`，从 `seq 1` 正常写事件 |

**不变式全部保持**：`turn_id == user_msg_id`（B 自己的 turn_id = B 的 message_id）、`turn_ended` 先于 `cleanup` 落库、append-only 无中途注入。

---

## 5. 端到端时序

```
用户在 agent 生成中发送 B
  → 前端放行（不拦截），invoke sendMessage(conv, B)
  → 后端 send_message(1v1)：chat_state.token_of(conv) == Some(in_flight)（快照令牌，W1 ②）
       → 【Steer 分支】物化 B（复用 channel handle_user_send 物化块）
       → in_flight.cancel()   // 点名取消快照令牌——stop 无回合身份，大附件物化
                              // 超 A 自然收尾时会误伤已起跑的续跑回合；A 已收尾
                              // 则 cancel 打在死令牌上（幂等无害）
       → 返回 Ok（消息已落库；死令牌分支手动 spawn 兜底积压消费）
  → 前端靠 user_message 事件权威刷新，B 气泡出现（标「已排队，本轮边界后生效」）
  → 在途 loop 在下一个 yield 点（①或②）命中 cancel
       → finalize_cancel：turn_ended(abort) → unregister → chat:done(abort)
  → 1v1 turn_ended 广播触发 watcher
       → 静默窗（复用 inbox/channel 的 2s 轮询 · 30s 上限）等落定
       → 查积压（marker 记账，W1 ①）：user_message 事件在场且无
         turn_context/turn_ended 标记的 B（`list_unconsumed_user_anchors`）
       → run_agent_turn(pre_materialized=true) 开新回合 B
  → 回合 B 走完整 Pipeline（read_route → derive 读历史，天然含回合 A 已 finalize 的全部轮次 + B）
```

---

## 6. 前端

1. `sendMessage` 的 `if (sending.value && !isChannel)` 守卫（[chat.ts:594](packages/app/src/stores/chat.ts)）：从「写 send_failed 横幅早退」改为**放行**——发送 + **不乐观 push**（复用频道模式，靠 `user_message` 事件驱动权威刷新，避免 `sending` 状态复杂度与重复渲染）。
2. steer 气泡加轻量标记「已排队 · 本轮边界后生效」；回合 A abort、回合 B 起跑后标记消失。
3. `chat:done(abort)` 已由现有流处理（非正常完成态），需确认 abort 后前端不会把 B 气泡误当「未落库」清掉——B 已物化落库，与乐观占位无关。

---

## 7. 预算 / 窗口（路线乙的关键红利）

**回合 B 是全新回合，走完整 Pipeline**，`LoopBudget` 独立、`TokenWindowStage` + `MemoryStage` 在回合 B 起点重新跑：

- 回合 A 已 finalize 的轮次（含大 tool_result）作为**历史**进回合 B 的 `read_route → derive`；
- 若 A 的部分输出 + B 超窗口，由回合 B 的 `TokenWindowStage` 硬裁 + `MemoryStage` 折叠正常处理。

**因此「回合中途预算复查」这个观察项被路线乙天然绕开**——我们从不往一个运行中的回合注入 token，永远开新回合重新估算。这是路线乙优于路线甲的决定性理由之一（路线甲才需要中途预算复查）。

---

## 8. resume / 崩溃

- 回合 A：steer 后必然走到 `finalize_cancel` → `turn_ended(abort)` **落库后才 unregister**，A 是**已 closed** 的干净回合。
- 回合 B：若半途崩溃，是普通「无 turn_ended 尾巴」，走现有 `backfill`/`reconcile` 的 incomplete_turn 处理，**无新增边**（B 是普通回合，不是特殊形态）。
- **B 物化后、消费回合起跑前崩溃/重启（W1 ③）**：turn_ended watcher 不再触发、前端「排队中」角标随重启清零，积压若无扫尾即永久搁浅。boot 扫尾 `spawn_boot_sweep`（lib.rs setup，`conversations_with_unconsumed_anchors` 圈 1v1 会话）逐会话 `consume_steer_backlog` 自愈——marker 记账下幂等，正常 boot 零命中零成本。

---

## 9. 与长期记忆的分界（未来线路，不混入）

- Steer 回答的是「生成中插一句话怎么进」——**无状态 replay**，已完成轮次已落库，新回合重读即得上下文连续性。
- 长期记忆 / 世界状态（codex `WorldState` 差分注入 → 我们台账已留的 `EnvironmentStage`，快照可存 `session_events`）回答的是「agent 怎么记住环境/人格/进度而不每轮重发」。**独立线路，Steer 不依赖它。**

---

## 10. 边界情况

1. **text-only 回复中发送**：模型正在写最终答案（无 tool_use），steer 置位 cancel 后，loop 无下一个工具轮、自然走 `finalize_success` 收尾 → steer 消息成为下一回合的正常起头（等价 Queue，符合 Claude Code「text-only 等到结束」）。
2. **一次长回合内连发多条（已定论：数据层各自成回合，不合并）**：`turn_id == user_msg_id` 是 append-only 日志最深不变式，derive / reconcile / 轨迹轮号 / MemoryStage 摘要锚点 / MA-3 pending 判定全部建在它上面——数据层合并 = 动地基，否。连发 B1/B2/B3 各自成回合、串行完整执行。**连发互不打断**靠「静默窗口」：steer 触发 cancel 后 watcher 不立即开新回合，先等短静默窗（复用频道 `CHAIN_HEAD_QUIET` 3s）让连发到齐——B2/B3 在 B1 start 前已进积压，不再是「新发送」，不会去 cancel B1。**「看着像 1 回合」留给呈现层**（纯渲染归组，频道轨迹「逻辑轮键归父」已有先例），后做、不碰数据。
3. **发送后立即又发（A 已 unregister、B 未 start 的窗口）**：watcher 与正常发送路径都走 `chat_state.start` 原子认领，撞上返回「在途」→ watcher 退避重试（复用 channel 的等静默轮询），不会双跑。**W1 ① 已按此落地**：`consume_steer_backlog` 撞忙不放弃——退避回循环头等静默、按剩余积压续接；积压边界 = marker 记账非「最近 turn_ended 之后」推断，任何触发源（watcher / 死令牌兜底 / boot 扫尾）读到同一账面，撞忙放弃不再丢边界。
4. **委派子会话 / 频道**：不涉及——Steer 只对 `kind='chat'` 的 1v1。

---

## 11. 待拍板 / 已定

1. ~~多次 steer 的合并~~ **已定论（2026-09-18）**：数据层各自成回合（§10.2 论证，`turn_id == user_msg_id` 不可谈判）；「看着像 1 回合」归呈现层，后做。
2. ~~前端标记形态~~ **已定（2026-09-18）**：气泡角标——「排队中」状态钉在 steer 消息气泡上（右上角小胶囊或轮廓虚线 + micro 小字），回合 B 起跑后消失。
3. ~~过渡提示~~ **已定方向（2026-09-18）**：需要，但**后续专门设计**，不做纯文字（那样很丑）——本期先静默衔接。

---

## 12. 实施文件清单（已实施随 0.9.1；W1 批增补）

| 文件 | 改动 |
|---|---|
| `packages/app/src-tauri/src/commands/chat_cmd.rs` | 1v1 分支：`token_of` 快照在途令牌 → 物化 → 点名 cancel 快照（W1 ② 单快照设计），空闲走原路径 |
| `packages/app/src-tauri/src/harness/channel.rs` | `materialize_user_message` 抽成 1v1 可复用（物化块 + `user_message` 事件即 marker 入账） |
| `packages/app/src-tauri/src/harness/steer.rs` | 1v1 turn_ended watcher：查积压 → 静默窗 → `run_agent_turn(pre_materialized=true)`；W1 批 = busy-retry 退避循环（`consume_steer_backlog`）+ `spawn_boot_sweep` + 简报测试 |
| `packages/app/src-tauri/src/harness/chat_state.rs` | W1 ②：`token_of`（快照在途回合令牌，回合身份版 stop） |
| `packages/app/src-tauri/src/db/repo/message.rs` | W1 ①：`list_unconsumed_user_anchors` / `conversations_with_unconsumed_anchors`（marker 记账三腿谓词）+ 回归测试 |
| `packages/app/src-tauri/src/lib.rs` | W1 ③：`spawn_boot_sweep` boot 挂点 |
| `packages/app/src/stores/chat.ts` | `sendMessage` 守卫放行 + 不乐观 push + steer 标记 |
| `packages/app/src/composables/useChatEvents.ts` | `user_message` 事件分支泛化到 1v1（权威刷新 steer 气泡） |

**零改动**：`session_runner.rs`（`pre_materialized` 现成）、`cleanup.rs`（finalize 对称性现成）、`loop_engine.rs`（cancel yield 点现成）、`event_log.rs`（零新 kind——积压记账复用既有 `user_message`/`turn_context`/`turn_ended` 三 kind）。

---

## 13. 结论

Steer 不需要新的状态模型，也不需要动 loop 核心。它是「自动 stop_at_boundary + 自动续跑」两个现成能力的接线，最深的改动只是把频道那套物化/积压机制从 `kind=channel` 泛化到 1v1。设计上唯一要拍的是 §11 的三个点。
