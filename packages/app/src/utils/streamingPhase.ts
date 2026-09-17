// streamingPhase — 生成中气泡 footer 的动态阶段（0.9 阶段提示）。
//
// 把 chat store 的流式状态映射为单段阶段文案，替换静态「正在生成…」。
// 纯函数（零 store 依赖），输入最小结构类型——测试可脱离 Pinia 直测优先级。
//
// 阶段优先级（自上而下，含 fallback 共 6 档）：
//   1 调用工具中     —— 有 ended===false 的工具调用（正在让模型产出 tool_use）
//   2 等待工具结果   —— 有 ended===true 且 result 未回填（工具在跑，等回传）
//   3 回答中         —— streamingText 非空
//   4 思考中         —— streamingThinking 非空（思考结束不清理该字段，故必须
//                       排在「回答中」之后，用 text 非空裁决——否则回答阶段会被
//                       残留思考内容误判成思考中）
//   5 压缩历史消息中 —— summarizing（后端 chat:summary-started 驱动，Pipeline 摘要期）
//   6 准备中         —— 以上皆无（TTFT 兜底：请求已发，首 token 未至）

/** 阶段文案（与优先级顺序一致；尾项是兜底） */
export const PHASE_LABELS = [
  "调用工具中",
  "等待工具结果",
  "回答中",
  "思考中",
  "压缩历史消息中",
  "准备中",
] as const;

export type StreamingPhase = (typeof PHASE_LABELS)[number];

/** phaseOf 的最小输入结构。streamingToolCalls 用迭代器承载（Map 传 .values()），
 *  值只读 ended/result 两字段——避免与 store 内部 ToolCallState 耦合。 */
export interface StreamingPhaseState {
  streamingText: string;
  streamingThinking: string;
  streamingToolCalls: Iterable<{ ended: boolean; result?: unknown }>;
  summarizing: boolean;
}

export function phaseOf(state: StreamingPhaseState): StreamingPhase {
  const calls = [...state.streamingToolCalls];
  if (calls.some((c) => !c.ended)) return "调用工具中";
  if (calls.some((c) => c.ended && !c.result)) return "等待工具结果";
  if (state.streamingText) return "回答中";
  if (state.streamingThinking) return "思考中";
  if (state.summarizing) return "压缩历史消息中";
  return "准备中";
}
