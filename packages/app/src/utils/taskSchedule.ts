// taskSchedule.ts — 定时任务调度人话摘要（设置页列表 / ChatHeader 任务会话头共用）
export const WEEKDAY_LABELS = ["一", "二", "三", "四", "五", "六", "日"];

function parseData(json: string): Record<string, unknown> {
  try { return JSON.parse(json) as Record<string, unknown>; } catch { return {}; }
}

/** 调度档位 JSON → 人话摘要（每天 09:00 / 每周一/三/五 08:00 / 每 30 分钟 / cron / 一次性） */
export function scheduleLabel(kind: string, dataJson: string): string {
  const d = parseData(dataJson);
  switch (kind) {
    case "once": return `一次性 ${typeof d.at === "string" ? d.at : "?"}`;
    case "daily": return `每天 ${typeof d.time === "string" ? d.time : "?"}`;
    case "weekly": {
      const days = Array.isArray(d.weekdays)
        ? (d.weekdays as number[]).map((w) => WEEKDAY_LABELS[w] ?? "?").join("/")
        : "?";
      return `每周${days} ${typeof d.time === "string" ? d.time : "?"}`;
    }
    case "interval": return `每 ${typeof d.minutes === "number" ? d.minutes : "?"} 分钟`;
    case "cron": return `cron：${typeof d.expr === "string" ? d.expr : "?"}`;
    default: return kind;
  }
}
