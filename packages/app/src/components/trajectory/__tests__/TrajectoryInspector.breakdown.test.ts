// TrajectoryInspector「上下文组成」区（③ 可观测化）：段列表 / 偏差行 / 逐轮命中行 /
// 旧回合（无 breakdown）不渲染。行模型走 buildRows 真实链路（折头 → Inspector）。
import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import TrajectoryInspector from "../TrajectoryInspector.vue";
import { buildRows, DEFAULT_HIDDEN, type TurnHeaderRow } from "../../../composables/useTrajectory";
import type { ContextBreakdownPayload, SessionEvent } from "../../../types";

let seq = 0;
function ev(kind: SessionEvent["kind"], payload: unknown): SessionEvent {
  seq += 1;
  return {
    id: seq,
    session_id: "s1",
    seq,
    kind,
    actor: "agent:a1",
    turn_id: "t1",
    message_id: null,
    payload: payload as never,
    created_at: `2026-09-10T10:00:${String(seq % 60).padStart(2, "0")}Z`,
  };
}

function headerWith(breakdown: ContextBreakdownPayload | null): TurnHeaderRow {
  const events = [
    ev("turn_context", { provider: "anthropic", effective_model: "glm-5.3", tools_enabled: true, tool_names: ["read_file"] }),
    ev("user_message", { content: "q", blocks: [] }),
    ev("turn_ended", { termination: "stop", rounds: 2, usage: { prompt_tokens: 4000, completion_tokens: 50, cached_tokens: 0 } }),
  ];
  if (breakdown) events.push(ev("context_breakdown", breakdown));
  const rows = buildRows(events, { collapsedTurns: new Set(), hiddenKinds: new Set(DEFAULT_HIDDEN), query: "", turnOffset: 0 });
  const h = rows.find((r): r is TurnHeaderRow => r.type === "turn-header");
  if (!h) throw new Error("header row missing");
  return h;
}

function mountInspector(h: TurnHeaderRow) {
  return mount(TrajectoryInspector, { props: { row: h } });
}

describe("TrajectoryInspector 上下文组成区", () => {
  it("段列表：中文标签 + 估算值 + 计数（词表映射未知 label 原样透出）", () => {
    const w = mountInspector(
      headerWith({
        v: 1,
        segments: [
          { label: "system_persona", est: 300 },
          { label: "tool_defs", est: 2400, count: 24 },
          { label: "history", est: 900, count: 4 },
        ],
        est_total: 3600,
        actual_prompt_tokens: 4000,
        fingerprint: { tools: "aaa", system_stable: "bbb", os_stable: "ccc" },
        rounds: [{ prompt: 4000, cached: 2000, tools_hash: "aaa", injected: false, model_switched: false }],
      }),
    );
    expect(w.find(".isec-title").text()).toBe("本轮配置快照");
    expect(w.text()).toContain("上下文组成");
    const labels = w.findAll(".ibd-label").map((n) => n.text());
    expect(labels).toEqual(["人设", "工具定义", "历史消息"]);
    expect(w.text()).toContain("2.4K tok · 24");
    // 未知 label（后端新增段前端未跟）：原样透出不吞
    const w2 = mountInspector(
      headerWith({
        v: 1,
        segments: [{ label: "future_seg", est: 10 }],
        est_total: 10,
        fingerprint: { tools: "a", system_stable: "b", os_stable: "c" },
        rounds: [],
      }),
    );
    expect(w2.findAll(".ibd-label").map((n) => n.text())).toEqual(["future_seg"]);
  });

  it("合计行：估算 + 实际首轮输入 + 偏差措辞（偏低/偏高/缺失）", () => {
    const mk = (est: number, actual?: number) =>
      headerWith({
        v: 1,
        segments: [{ label: "history", est }],
        est_total: est,
        actual_prompt_tokens: actual,
        fingerprint: { tools: "a", system_stable: "b", os_stable: "c" },
        rounds: [{ prompt: actual ?? 0, cached: 0, tools_hash: "a", injected: false, model_switched: false }],
      });
    const low = mountInspector(mk(3600, 4000));
    expect(low.text()).toContain("估算 3.6K tok");
    expect(low.text()).toContain("实际首轮输入 4K tok");
    expect(low.text()).toContain("估算偏低约 10%");
    const high = mountInspector(mk(5000, 4000));
    expect(high.text()).toContain("估算偏高约 25%");
    const noActual = mountInspector(mk(3600, undefined));
    expect(noActual.text()).toContain("估算 3.6K tok");
    expect(noActual.text()).not.toContain("实际首轮输入");
  });

  it("逐轮命中行：输入/命中/百分比 + 全未命中徽 + 工具列表变化徽 + 换档徽", () => {
    const w = mountInspector(
      headerWith({
        v: 1,
        segments: [{ label: "history", est: 1000 }],
        est_total: 1000,
        actual_prompt_tokens: 1000,
        fingerprint: { tools: "aaa", system_stable: "b", os_stable: "c" },
        rounds: [
          { prompt: 4000, cached: 2000, tools_hash: "aaa", injected: false, model_switched: false },
          { prompt: 6000, cached: 0, tools_hash: "bbb", injected: true, model_switched: true },
        ],
      }),
    );
    const rounds = w.findAll(".ibd-round");
    expect(rounds).toHaveLength(2);
    expect(rounds[0].text()).toContain("输入 4K · 命中 2K（50%）");
    expect(rounds[0].find(".ibd-badge").exists()).toBe(false);
    expect(rounds[1].text()).toContain("输入 6K · 命中 0（0%）");
    const badges = rounds[1].findAll(".ibd-badge").map((n) => n.text());
    expect(badges).toContain("全未命中");
    expect(badges).toContain("工具列表变化");
    expect(badges).toContain("换档");
  });

  it("无 breakdown（无 usage 回合/旧回合）：整区不渲染", () => {
    const w = mountInspector(headerWith(null));
    expect(w.text()).not.toContain("上下文组成");
    expect(w.find(".ibd-seg").exists()).toBe(false);
  });

  it("披露脚注常驻（估算口径 + 推断非报告）", () => {
    const w = mountInspector(
      headerWith({
        v: 1,
        segments: [{ label: "history", est: 100 }],
        est_total: 100,
        fingerprint: { tools: "a", system_stable: "b", os_stable: "c" },
        rounds: [],
      }),
    );
    const foot = w.find(".ibd-foot");
    expect(foot.exists()).toBe(true);
    expect(foot.text()).toContain("本地估算");
    expect(foot.text()).toContain("非 provider 报告");
  });
});
