// phaseMachine — 生成中相位状态机（2026-10-26 生成中状态渲染三步改造批 2）
//
// 第一性修正：后端已在每个相位切换点 emit 对应数据事件（chat:thinking /
// chat:chunk / chat:tool-call-* / chat:retrying / chat:tool-auth-request /
// chat:ask-user-request / chat:delegation-started）——本模块把现有事件流
// 归约成相位，不新建后端事件。`nextPhase` 是纯 reducer；`phaseLabel` /
// `phaseShortLabel` 生成用户语言文案（长版进气泡相位行/cursor-bar，短版进
// 侧栏 badge）。
//
// 与 phaseOf 的分工：phaseOf 从前端残留状态「推断」（兜底——事件缺失/旧
// 回放时）；phaseMachine 是事件驱动的权威相位（真相在后端事件序）。
// 相位优先级：等待用户（审批/选择）> 退避 > 委派 > 工具执行中 > 工具等待 >
// 流式输出 > 思考 > 压缩 > 轮间隙 > 准备。

/** 相位枚举（等待态全含：审批/选择卡/委派/退避——用户拍板覆盖全部页面上
 *  需要体现的事件面）。 */
export type Phase =
  | "preparing"          // 回合已起、首 token 未至（TTFT）——旧「准备中」
  | "summarizing"        // 压缩历史（Pipeline MemoryStage）
  | "thinking"           // 模型思考流
  | "streaming"          // 正文输出流
  | "tool_calling"       // 模型正在产出 tool_use（参数流式）
  | "tool_waiting"       // 工具执行中（参数到齐等 result）
  | "awaiting_approval"  // 审批卡在等用户
  | "awaiting_choice"    // ask_user 选择卡在等用户
  | "delegating"         // 委派子会话执行中
  | "retrying"           // 限流/网络退避等待
  | "round_gap"          // 工具轮完成、下一轮未起（assistant-start 瞬间）
  | "done";              // 回合终态

/** 相位附加上下文（reducer 状态的载荷部分）。 */
export interface PhaseContext {
  /** tool_calling/tool_waiting 的最近工具名（用户语言展示） */
  toolName?: string;
  /** 一轮并行工具数（tool_calling 聚合展示「N 个工具」） */
  toolCount?: number;
  /** retrying：第 n/m 次 + 等待秒数 */
  retryAttempt?: number;
  retryMax?: number;
  retryWaitSecs?: number;
}

export interface PhaseState {
  phase: Phase;
  since: number;          // 相位进入时间戳（ms）
  ctx: PhaseContext;
}

export function initialPhase(): PhaseState {
  return { phase: "preparing", since: Date.now(), ctx: {} };
}

/** 事件名联合（与后端 chat:* 词表对齐的子集——只取影响相位的）。 */
export type PhaseEventName =
  | "chat:start"
  | "chat:processing"
  | "chat:summary-started"
  | "chat:summary-injected"
  | "chat:thinking"
  | "chat:chunk"
  | "chat:tool-call-start"
  | "chat:tool-call-delta"
  | "chat:tool-call-end"
  | "chat:tool-result"
  | "chat:tool-auth-request"
  | "chat:tool-auth-responded"
  | "chat:tool-auth-request-cancel"
  | "chat:ask-user-request"
  | "chat:ask-user-responded"
  | "chat:ask-user-request-cancel"
  | "chat:delegation-started"
  | "chat:retrying"
  | "chat:assistant-start"
  | "chat:done"
  | "chat:error";

/** 事件载荷的最小结构（相位 reducer 只读这些字段）。 */
export interface PhaseEventPayload {
  conversation_id?: string;
  stage?: string;                 // chat:processing
  name?: string;                  // tool-call-start
  tool_use_id?: string;           // auth-request（关联最近工具名由调用方补）
  attempt?: number;               // retrying
  max_attempts?: number;          // retrying
  reason?: string;                // retrying
}

/** 活跃工具计数（调用方从桶的 streamingToolCalls 聚合传入——reducer 保持纯）。 */
export interface ToolStats {
  openCount: number;    // ended=false（参数流式中）
  waitingCount: number; // ended=true 且无 result（执行中）
  lastName?: string;    // 最近 start 的工具名
}

/**
 * 相位 reducer：事件 → 新相位状态。纯函数（时间戳由内部 Date.now 取，唯一
 * 非确定性来源，与 useThinkingTimer 模式一致）。
 *
 * 语义要点：
 * - 等待用户相位（审批/选择）进入后不被数据事件覆盖（用户在场是最高事实，
 *   后端此刻阻塞在等响应）；被 responded/cancel 事件显式退出。
 * - retrying 被 chunk/thinking/assistant-start 覆盖（退避结束重试成功）。
 * - tool-result → round_gap（下一事件通常立刻到来；round_gap 是诚实的瞬间态）。
 * - 事件与当前相位相同 → 只刷新 since 不动 ctx（幂等）。
 */
export function nextPhase(
  prev: PhaseState,
  event: PhaseEventName,
  payload: PhaseEventPayload,
  tools: ToolStats,
): PhaseState {
  const now = Date.now();
  const set = (phase: Phase, ctx: PhaseContext = {}): PhaseState =>
    prev.phase === phase && sameCtx(prev.ctx, ctx)
      ? prev
      : { phase, since: now, ctx };

  // 等待用户相位：只被显式响应/取消退出（数据事件是阻塞期的迟到回声）
  if (prev.phase === "awaiting_approval") {
    if (event === "chat:tool-auth-responded" || event === "chat:tool-auth-request-cancel") {
      return set("tool_waiting", { toolName: prev.ctx.toolName, toolCount: 1 });
    }
    if (event === "chat:done" || event === "chat:error") return set("done");
    return prev;
  }
  if (prev.phase === "awaiting_choice") {
    if (event === "chat:ask-user-responded" || event === "chat:ask-user-request-cancel") {
      return set("tool_waiting", { toolName: prev.ctx.toolName, toolCount: 1 });
    }
    if (event === "chat:done" || event === "chat:error") return set("done");
    return prev;
  }

  switch (event) {
    case "chat:start":
      return set("preparing");
    case "chat:processing":
      // stage=pipeline 为主；ocr 在回合中段也可能到（保持当前相位不回退）
      return prev.phase === "preparing" ? prev : prev.phase === "done" ? set("preparing") : prev;
    case "chat:summary-started":
      return set("summarizing");
    case "chat:summary-injected":
      return set("preparing");
    case "chat:thinking":
      return set("thinking");
    case "chat:chunk":
      return set("streaming");
    case "chat:tool-call-start":
    case "chat:tool-call-delta":
      return set("tool_calling", {
        toolName: payload.name ?? tools.lastName,
        toolCount: Math.max(1, tools.openCount),
      });
    case "chat:tool-call-end":
      return set("tool_waiting", {
        toolName: payload.name ?? tools.lastName,
        toolCount: Math.max(1, tools.waitingCount),
      });
    case "chat:tool-result":
      return set("round_gap");
    case "chat:tool-auth-request":
      return set("awaiting_approval", { toolName: tools.lastName, toolCount: 1 });
    case "chat:ask-user-request":
      return set("awaiting_choice", { toolName: tools.lastName, toolCount: 1 });
    case "chat:delegation-started":
      return set("delegating");
    case "chat:retrying":
      return set("retrying", {
        retryAttempt: payload.attempt,
        retryMax: payload.max_attempts,
        retryWaitSecs: 0, // 秒数由退避展示层按 payload.reason 缺席自行倒计时；此处占位
      });
    case "chat:assistant-start":
      return set("round_gap");
    case "chat:done":
    case "chat:error":
      return set("done");
    default:
      return prev;
  }
}

function sameCtx(a: PhaseContext, b: PhaseContext): boolean {
  return (
    a.toolName === b.toolName &&
    a.toolCount === b.toolCount &&
    a.retryAttempt === b.retryAttempt &&
    a.retryMax === b.retryMax
  );
}

// =========================================================================
// 文案
// =========================================================================

/** 长文案（气泡相位行 / cursor-bar）。progress 可选：已持续时长 + 轮次。 */
export function phaseLabel(
  s: PhaseState,
  progress?: { elapsedLabel?: string; round?: number },
): string {
  const suffix = progress
    ? ` · ${[
        progress.round && progress.round > 0 ? `第 ${progress.round} 轮` : "",
        progress.elapsedLabel ?? "",
      ].filter(Boolean).join(" · ")}`
    : "";
  switch (s.phase) {
    case "preparing":
      return `准备中${suffix}`;
    case "summarizing":
      return `压缩历史消息中${suffix}`;
    case "thinking":
      return `思考中${suffix}`;
    case "streaming":
      return `正在回答${suffix}`;
    case "tool_calling":
      return s.ctx.toolCount && s.ctx.toolCount > 1
        ? `正在组织 ${s.ctx.toolCount} 个工具调用${suffix}`
        : `正在调用 ${s.ctx.toolName ?? "工具"}${suffix}`;
    case "tool_waiting":
      return `正在执行 ${s.ctx.toolName ?? "工具"}…${suffix}`;
    case "awaiting_approval":
      return `等待你的审批${s.ctx.toolName ? `（${s.ctx.toolName}）` : ""}`;
    case "awaiting_choice":
      return "等待你的选择";
    case "delegating":
      return `委派执行中${suffix}`;
    case "retrying": {
      const n = s.ctx.retryAttempt;
      const m = s.ctx.retryMax;
      const attempt = n && m ? `第 ${n}/${m} 次重试` : "重试中";
      return `被限流，稍后自动重试（${attempt}）${suffix}`;
    }
    case "round_gap":
      return `轮次切换中${suffix}`;
    case "done":
      return "已完成";
  }
}

/** 短文案（侧栏 badge——3-6 字，替代统一「生成中」）。 */
export function phaseShortLabel(s: PhaseState): string {
  switch (s.phase) {
    case "preparing":
      return "准备中";
    case "summarizing":
      return "压缩中";
    case "thinking":
      return "思考中";
    case "streaming":
      return "回答中";
    case "tool_calling":
    case "tool_waiting":
      return "工具中";
    case "awaiting_approval":
      return "等审批";
    case "awaiting_choice":
      return "等选择";
    case "delegating":
      return "委派中";
    case "retrying":
      return "重试中";
    case "round_gap":
      return "生成中";
    case "done":
      return "生成中"; // done 后桶即删，此值不应被渲染
  }
}
