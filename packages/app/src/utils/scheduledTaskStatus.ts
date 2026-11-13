// 定时任务执行记录的展示契约；项目委派台账的 taskStatus.ts 使用另一套终态。
type StatusMeta = {
  label: string;
  tone: "success" | "warning" | "danger" | "neutral";
};

const STATUS_META: Record<string, StatusMeta> = {
  running: { label: "运行中", tone: "warning" },
  done: { label: "已完成", tone: "success" },
  error: { label: "未完成", tone: "danger" },
  stopped: { label: "已停止", tone: "neutral" },
  missed: { label: "已错过", tone: "neutral" },
};
const UNKNOWN_STATUS: StatusMeta = { label: "未完成", tone: "danger" };

export function scheduledTaskStatus(status: string): StatusMeta {
  return Object.prototype.hasOwnProperty.call(STATUS_META, status) ? STATUS_META[status] : UNKNOWN_STATUS;
}

interface RunOutcome {
  status: string;
  error?: string | null;
  summary?: string | null;
}

/** 原因与输出分开：部分输出不能替代失败原因，也不能证明任务正常完成。 */
export function scheduledTaskReason(run: RunOutcome): string | null {
  if (run.status === "running" || run.status === "done") return null;
  if (run.error?.trim()) return run.error.trim();
  if (run.status === "stopped") return "任务已由你停止，可查看已生成的内容。";
  if (run.status === "missed") return "本次计划已错过，未执行。";
  if (run.status === "error") return "任务未正常完成，请查看执行记录。";
  return "任务终止状态未知，请查看执行记录。";
}

/** 只有 done 发完成通知；其他终态优先通知原因，不将部分输出当作正常结果。 */
export function scheduledTaskNotification(
  run: RunOutcome & { task_name: string },
): { title: string; body: string } | null {
  if (run.status === "running") return null;
  const firstLine = (run.summary ?? "").trim().split("\n")[0].trim();
  return {
    title: `定时任务${scheduledTaskStatus(run.status).label}：${run.task_name}`,
    body: run.status === "done"
      ? firstLine || "查看执行记录了解结果"
      : scheduledTaskReason(run)!,
  };
}
