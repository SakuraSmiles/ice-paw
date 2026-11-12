// taskBoard 纯函数测试（批 P2-2）：分区判定 + 组内排序方向（旧 TaskLedger v1
// 排序锁的语义迁移处——running 置顶在此以 section 语义延续）。
import { describe, it, expect } from "vitest";
import { buildBoard, sectionOf } from "../taskBoard";
import type { ProjectTask } from "../../types";

function t(conv: string, over: Partial<ProjectTask> = {}): ProjectTask {
  return {
    conv_id: conv, title: conv, executor_agent_id: "a", initiator_agent_id: null,
    parent_conversation_id: null, started_at: "2026-01-01 00:00:00",
    updated_at: "2026-01-01 00:00:00", ended_at: null, termination: null, rounds: null,
    ...over,
  };
}

describe("sectionOf（分区判定）", () => {
  it("waiting 压过一切（包括 running）——人的注意力最高优先", () => {
    expect(sectionOf("running", true)).toBe("waiting");
    expect(sectionOf("done", true)).toBe("waiting");
  });
  it("非 waiting 按五桶映射：running → running；failed/interrupted → failed；done/ended-other → done", () => {
    expect(sectionOf("running", false)).toBe("running");
    expect(sectionOf("failed", false)).toBe("failed");
    expect(sectionOf("interrupted", false)).toBe("failed");
    expect(sectionOf("done", false)).toBe("done");
    expect(sectionOf("ended-other", false)).toBe("done");
  });
});

describe("buildBoard（全量排序）", () => {
  const streaming = new Set(["r1", "w1"]);

  it("区序 waiting > running > failed > done；waiting/running 组内等最久在前，failed/done 最近在前", () => {
    const rows = buildBoard(
      [
        t("old-done", { termination: "stop", updated_at: "2026-01-01 09:00:00" }),
        t("new-done", { termination: "stop", updated_at: "2026-01-01 12:00:00" }),
        t("r1", { updated_at: "2026-01-01 08:00:00" }),
        t("r2", { updated_at: "2026-01-01 08:30:00" }), // 无 streaming → interrupted → failed
        t("w1", { updated_at: "2026-01-01 08:10:00" }),
      ],
      streaming,
      new Map([["w1", "用哪个版本？"]]),
    );
    expect(rows.map((r) => r.task.conv_id)).toEqual([
      "w1", // waiting（唯一）
      "r1", // running（唯一）
      "r2", // failed（唯一，interrupted 归入）
      "new-done", "old-done", // done 组内倒序：最近的在前
    ]);
  });

  it("waiting 行携带问题摘要；非 waiting 行为 null", () => {
    const rows = buildBoard(
      [t("w1"), t("d1", { termination: "stop" })],
      new Set(["w1"]),
      new Map([["w1", "选 A 还是 B？"]]),
    );
    expect(rows[0].waitingQuestion).toBe("选 A 还是 B？");
    expect(rows[1].waitingQuestion).toBeNull();
  });
});
