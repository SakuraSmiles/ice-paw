/**
 * turn 终止原因（turn_ended.termination）的中文文案 —— 单一真相源。
 *
 * 消费方：轨迹表轮次分割头 / 轨迹检查器（原两处各写一份词表 + Inspector
 * 裸透英文，backfill 会话验收时暴露）。新 termination 值落词表这里，
 * 词表外裸透原始值（技术兜底，不猜）。
 */

/** termination → 中文文案（词表外裸透原值） */
export function termLabel(t: string): string {
  return TERMINATION_LABELS[t] ?? t;
}

/** 非常规终止（渲染 warn 态）。backfill 是诚实标注的历史补录，非异常。 */
export function isWarnTermination(t: string): boolean {
  return t !== "stop" && t !== "backfill";
}

const TERMINATION_LABELS: Record<string, string> = {
  stop: "正常结束",
  length: "长度截断",
  max_tokens: "长度截断",
  tool_use: "工具轮数上限",
  abort: "手动停止",
  budget_exceeded: "预算超限",
  stuck: "无进展终止",
  error: "出错",
  interrupted: "应用中断",
  backfill: "历史补录",
};
