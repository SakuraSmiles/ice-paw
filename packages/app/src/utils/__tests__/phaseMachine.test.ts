import { describe, it, expect } from "vitest";
import {
  initialPhase, nextPhase, phaseLabel, phaseShortLabel,
  type PhaseState, type ToolStats,
} from "../phaseMachine";

const NO_TOOLS: ToolStats = { openCount: 0, waitingCount: 0 };

function drive(prev: PhaseState, steps: Array<readonly [Parameters<typeof nextPhase>[1], Parameters<typeof nextPhase>[2] | undefined]>, tools: ToolStats = NO_TOOLS): PhaseState {
  let s = prev;
  for (const [name, payload] of steps) {
    s = nextPhase(s, name, payload ?? {}, tools);
  }
  return s;
}

describe("phaseMachine", () => {
  it("正常回合主链：preparing → thinking → streaming → done", () => {
    let s = initialPhase();
    expect(s.phase).toBe("preparing");
    s = nextPhase(s, "chat:thinking", {}, NO_TOOLS);
    expect(s.phase).toBe("thinking");
    s = nextPhase(s, "chat:chunk", {}, NO_TOOLS);
    expect(s.phase).toBe("streaming");
    s = nextPhase(s, "chat:done", {}, NO_TOOLS);
    expect(s.phase).toBe("done");
  });

  it("工具轮链：tool_calling（带名与并行计数）→ tool_waiting → round_gap → 下一轮", () => {
    let s = initialPhase();
    s = nextPhase(s, "chat:tool-call-start", { name: "read_file" }, { openCount: 1, waitingCount: 0, lastName: "read_file" });
    expect(s.phase).toBe("tool_calling");
    expect(s.ctx.toolName).toBe("read_file");
    s = nextPhase(s, "chat:tool-call-end", {}, { openCount: 0, waitingCount: 1, lastName: "read_file" });
    expect(s.phase).toBe("tool_waiting");
    s = nextPhase(s, "chat:tool-result", {}, NO_TOOLS);
    expect(s.phase).toBe("round_gap");
    s = nextPhase(s, "chat:assistant-start", {}, NO_TOOLS);
    expect(s.phase).toBe("round_gap");
    s = nextPhase(s, "chat:chunk", {}, NO_TOOLS);
    expect(s.phase).toBe("streaming");
  });

  it("审批等待相位：数据事件不覆盖（阻塞期迟到回声），responded 恢复执行", () => {
    let s = drive(initialPhase(), [
      ["chat:tool-call-start", { name: "run_command" }],
    ], { openCount: 1, waitingCount: 0, lastName: "run_command" });
    s = nextPhase(s, "chat:tool-auth-request", {}, { openCount: 0, waitingCount: 1, lastName: "run_command" });
    expect(s.phase).toBe("awaiting_approval");
    // 阻塞期的迟到数据事件不覆盖
    s = nextPhase(s, "chat:chunk", {}, NO_TOOLS);
    expect(s.phase).toBe("awaiting_approval");
    s = nextPhase(s, "chat:tool-auth-responded", {}, NO_TOOLS);
    expect(s.phase).toBe("tool_waiting");
  });

  it("选择卡等待（ask_user）：respond/cancel 退出，done 也能收口", () => {
    let s = drive(initialPhase(), [["chat:ask-user-request", {}]]);
    expect(s.phase).toBe("awaiting_choice");
    s = nextPhase(s, "chat:tool-result", {}, NO_TOOLS);
    expect(s.phase).toBe("awaiting_choice");
    s = nextPhase(s, "chat:ask-user-request-cancel", {}, NO_TOOLS);
    expect(s.phase).toBe("tool_waiting");
    // done 收口路径
    let s2 = drive(initialPhase(), [["chat:ask-user-request", {}]]);
    s2 = nextPhase(s2, "chat:done", {}, NO_TOOLS);
    expect(s2.phase).toBe("done");
  });

  it("退避相位：retrying 进入（带 attempt/max），chunk 覆盖（重试成功）", () => {
    let s = initialPhase();
    s = nextPhase(s, "chat:retrying", { attempt: 2, max_attempts: 5 }, NO_TOOLS);
    expect(s.phase).toBe("retrying");
    expect(s.ctx.retryAttempt).toBe(2);
    expect(s.ctx.retryMax).toBe(5);
    s = nextPhase(s, "chat:chunk", {}, NO_TOOLS);
    expect(s.phase).toBe("streaming");
  });

  it("委派相位：delegation-started 进入，tool-result（父回合收到）退出", () => {
    let s = drive(initialPhase(), [["chat:delegation-started", {}]]);
    expect(s.phase).toBe("delegating");
    s = nextPhase(s, "chat:tool-result", {}, NO_TOOLS);
    expect(s.phase).toBe("round_gap");
  });

  it("摘要相位：summary-started → injected 回 preparing", () => {
    let s = drive(initialPhase(), [["chat:summary-started", {}]]);
    expect(s.phase).toBe("summarizing");
    s = nextPhase(s, "chat:summary-injected", {}, NO_TOOLS);
    expect(s.phase).toBe("preparing");
  });

  it("幂等：同相位同 ctx 重复事件不重置 since", () => {
    let s = initialPhase();
    s = nextPhase(s, "chat:thinking", {}, NO_TOOLS);
    const t1 = s.since;
    s = nextPhase(s, "chat:thinking", {}, NO_TOOLS);
    expect(s.since).toBe(t1);
  });

  it("多工具并行：toolCount 聚合（3 个 open）", () => {
    const s = nextPhase(initialPhase(), "chat:tool-call-delta", { name: "search_files" }, { openCount: 3, waitingCount: 0, lastName: "search_files" });
    expect(s.phase).toBe("tool_calling");
    expect(s.ctx.toolCount).toBe(3);
    expect(phaseLabel(s)).toContain("3 个工具调用");
  });
});

describe("phaseLabel / phaseShortLabel 文案", () => {
  it("进程感后缀：轮次 + 时长拼接", () => {
    const s: PhaseState = { phase: "thinking", since: Date.now(), ctx: {} };
    expect(phaseLabel(s, { round: 3, elapsedLabel: "42s" })).toBe("思考中 · 第 3 轮 · 42s");
  });

  it("等待用户文案不带时长（用户不该被催）", () => {
    const s: PhaseState = { phase: "awaiting_approval", since: Date.now(), ctx: { toolName: "run_command" } };
    const label = phaseLabel(s, { round: 2, elapsedLabel: "10s" });
    expect(label).toBe("等待你的审批（run_command）");
    expect(label).not.toContain("10s");
  });

  it("退避文案含次数", () => {
    const s: PhaseState = { phase: "retrying", since: Date.now(), ctx: { retryAttempt: 2, retryMax: 5 } };
    expect(phaseLabel(s)).toContain("第 2/5 次重试");
    expect(phaseShortLabel(s)).toBe("重试中");
  });

  it("侧栏短词全表（3-6 字）", () => {
    const cases: Array<[import("../phaseMachine").Phase, string]> = [
      ["awaiting_approval", "等审批"], ["awaiting_choice", "等选择"],
      ["retrying", "重试中"], ["delegating", "委派中"], ["tool_waiting", "工具中"],
    ];
    for (const [p, label] of cases) {
      expect(phaseShortLabel({ phase: p, since: 0, ctx: {} })).toBe(label);
    }
  });
});
