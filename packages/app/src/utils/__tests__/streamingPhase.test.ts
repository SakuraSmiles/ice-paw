// streamingPhase — 六档阶段优先级 + 思考/文本重叠裁决
import { describe, expect, it } from "vitest";
import { phaseOf, PHASE_LABELS, type StreamingPhaseState } from "../streamingPhase";

/** 构造最小输入（流式工具调用以数组承载，模拟 Map.values() 迭代器） */
function state(overrides: Partial<StreamingPhaseState> = {}): StreamingPhaseState {
  return {
    streamingText: "",
    streamingThinking: "",
    streamingToolCalls: [],
    summarizing: false,
    ...overrides,
  };
}

const tool = (ended: boolean, hasResult = false) => ({
  ended,
  result: hasResult ? { content: "ok", isError: false, durationMs: 1 } : undefined,
});

describe("phaseOf", () => {
  it("六档词表齐备且顺序与优先级一致", () => {
    expect(PHASE_LABELS).toEqual([
      "调用工具中",
      "等待工具结果",
      "回答中",
      "思考中",
      "压缩历史消息中",
      "准备中",
    ]);
  });

  it("全空 → 准备中（TTFT 兜底）", () => {
    expect(phaseOf(state())).toBe("准备中");
  });

  it("有 ended===false 的工具调用 → 调用工具中", () => {
    expect(phaseOf(state({ streamingToolCalls: [tool(false)] }))).toBe("调用工具中");
  });

  it("有 ended===true 且 result 未回填 → 等待工具结果", () => {
    expect(phaseOf(state({ streamingToolCalls: [tool(true)] }))).toBe("等待工具结果");
  });

  it("工具结果已回填 → 不再等待（回落后续档）", () => {
    expect(
      phaseOf(state({ streamingToolCalls: [tool(true, true)] })),
    ).toBe("准备中");
  });

  it("回答中：streamingText 非空", () => {
    expect(phaseOf(state({ streamingText: "你好" }))).toBe("回答中");
  });

  it("思考中：仅 streamingThinking 非空", () => {
    expect(phaseOf(state({ streamingThinking: "让我想想" }))).toBe("思考中");
  });

  it("压缩历史消息中：summarizing 置位（其余为空）", () => {
    expect(phaseOf(state({ summarizing: true }))).toBe("压缩历史消息中");
  });

  it("优先级：调用工具中 > 等待结果 > 回答中 > 思考中 > 压缩中 > 准备中", () => {
    // 一轮 Anthropic 消息可「思考→文本→tool_use」共存，工具态恒压过文本态
    expect(
      phaseOf(
        state({
          streamingText: "正在写",
          streamingThinking: "先想",
          streamingToolCalls: [tool(false)],
        }),
      ),
    ).toBe("调用工具中");
    expect(
      phaseOf(
        state({
          streamingText: "正在写",
          streamingThinking: "先想",
          streamingToolCalls: [tool(true)],
        }),
      ),
    ).toBe("等待工具结果");
  });

  it("思考/文本重叠裁决：text 非空压过残留 thinking（思考结束不清理）", () => {
    expect(
      phaseOf(state({ streamingText: "回答正文", streamingThinking: "残留思考" })),
    ).toBe("回答中");
  });

  it("summarizing 与其他信号共存时仍按优先级（工具/文本先裁决）", () => {
    // 摘要只发生在流式开始前，正常情况下不会与 text 共存；防御性确认优先级不倒挂
    expect(phaseOf(state({ summarizing: true, streamingText: "正文" }))).toBe("回答中");
  });
});
