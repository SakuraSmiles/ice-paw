/**
 * 台账任务终态推导（纯函数）—— 前端单一真相源。
 *
 * 分桶规则与后端 commands/project_cmd.rs::termination_bucket 同款（注释互指）：
 * - running：流式 overlay 恒最高优先——静态数据表达不了「此刻正在跑」，
 *   streamingConvIds 是唯一实时信号
 * - done：termination ∈ {stop, end_turn}（与 delegate.rs is_normal_completion 同源）
 * - ended-other：backfill（boot 扫尾给零事件旧会话合成的事件，诚实标注的
 *   历史补录，中性灰——与 termLabels isWarnTermination 同款豁免）
 * - failed：词表外全归此（length/max_tokens/tool_use/budget_exceeded/stuck/
 *   abort/error/interrupted/未来新增/脏数据——不猜，技术兜底）
 * - interrupted：无 turn_ended 且非流式（含零事件会话）——终止未落库，
 *   诚实不伪造（boot sweep 也不补 spawn 前失败的零事件会话）
 */

export type TaskStatus = "running" | "done" | "failed" | "ended-other" | "interrupted";

/** 任务终态（台账行 + 流式 overlay） */
export function taskStatus(
  task: { conv_id: string; termination: string | null },
  streamingIds: ReadonlySet<string>,
): TaskStatus {
  if (streamingIds.has(task.conv_id)) return "running";
  if (task.termination === null) return "interrupted";
  switch (task.termination) {
    case "stop":
    case "end_turn":
      return "done";
    case "backfill":
      return "ended-other";
    default:
      return "failed";
  }
}

/** 状态 → 短文案（台账状态点 title / 空态说明） */
export const TASK_STATUS_LABELS: Record<TaskStatus, string> = {
  running: "进行中",
  done: "已完成",
  failed: "未成功",
  "ended-other": "历史补录",
  interrupted: "中断",
};
