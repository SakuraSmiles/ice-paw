// taskStatus.test.ts — 台账终态推导规则锁定（与后端 termination_bucket 同款）：
// running 流式 overlay 恒最高优先 / 五桶判定 / 词表外不猜归 failed。
import { describe, it, expect } from "vitest";
import { taskStatus, TASK_STATUS_LABELS } from "../taskStatus";

function task(conv_id: string, termination: string | null) {
  return { conv_id, termination };
}

describe("taskStatus 终态推导", () => {
  it("running 流式 overlay 恒最高优先——即使 termination 已落库", () => {
    // 边界：turn_ended 刚落库但前端流式态还没撤（时序错位）——以流式为准
    expect(taskStatus(task("c1", "stop"), new Set(["c1"]))).toBe("running");
    expect(taskStatus(task("c1", null), new Set(["c1"]))).toBe("running");
  });

  it("done = stop | end_turn", () => {
    const none = new Set<string>();
    expect(taskStatus(task("c", "stop"), none)).toBe("done");
    expect(taskStatus(task("c", "end_turn"), none)).toBe("done");
  });

  it("failed = 全部异常终止 + 词表外兜底", () => {
    const none = new Set<string>();
    for (const t of [
      "length",
      "max_tokens",
      "tool_use",
      "budget_exceeded",
      "stuck",
      "abort",
      "error",
      "interrupted",
      "未来新增的值",
    ]) {
      expect(taskStatus(task("c", t), none)).toBe("failed");
    }
  });

  it("ended-other = backfill（历史补录中性豁免）", () => {
    expect(taskStatus(task("c", "backfill"), new Set())).toBe("ended-other");
  });

  it("interrupted = 无 turn_ended 且非流式（含零事件会话，诚实不伪造）", () => {
    expect(taskStatus(task("c", null), new Set())).toBe("interrupted");
  });

  it("五桶文案齐备", () => {
    for (const s of ["running", "done", "failed", "ended-other", "interrupted"] as const) {
      expect(TASK_STATUS_LABELS[s].length).toBeGreaterThan(0);
    }
  });
});
