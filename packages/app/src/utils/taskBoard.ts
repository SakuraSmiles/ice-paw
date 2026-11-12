/**
 * 任务面板分区排序（批 P2-2，person-first——DSH 005-panel 哲学本地化）。
 *
 * 排序律（设计拍板 2026-11-12）：**等你回应 > 执行中 > 失败受阻 > 已完成**——
 * 最需要用户注意力的永远置顶。与 TaskPanel（会话级任务胶囊）的
 * 「状态优先+时间倒序」同族，但本面板多了 person 等待维度（ask_user 未作答）。
 *
 * 等待维度诚实边界：ask_user 的 pending 是进程内 oneshot（无超时但随进程
 * 存亡）——「等回答」只在应用活着时成立，重启后未答的 ask 其回合已不存在，
 * 不存在历史悬置态；故 waiting 判定直接消费 chat store 的 live Map，无需持久化。
 */

import type { ProjectTask } from "../types";
import { taskStatus, type TaskStatus } from "./taskStatus";

/** 面板分区（呈现实体；顺序即渲染顺序） */
export type BoardSection = "waiting" | "running" | "failed" | "done";

export const SECTION_TITLES: Record<BoardSection, string> = {
  waiting: "需要你",
  running: "进行中",
  failed: "失败 / 受阻",
  done: "已完成",
};

/** 分区行（任务 + 面板派生态） */
export interface BoardRow {
  task: ProjectTask;
  section: BoardSection;
  /** 基础五桶（running/done/failed/interrupted/ended-other）——面板分区的前置 */
  status: TaskStatus;
  /** 等你回答的问题摘要（仅 waiting 区有值；ask_user 的 question 截断） */
  waitingQuestion: string | null;
}

/**
 * 分区判定（纯函数）：
 * - waiting：会话有未作答 ask_user（live Map 命中）——无论底层五桶是什么，
 *   人的注意力优先级最高（跑着的任务问你问题 = 卡在等你）；
 * - running：五桶 running 且非 waiting；
 * - failed：failed + interrupted（中断 = 异常停摆，受阻语义；诚实不伪造终态）；
 * - done：done + ended-other（历史补录是中性完成态）。
 */
export function sectionOf(
  status: TaskStatus,
  waiting: boolean,
): BoardSection {
  if (waiting) return "waiting";
  if (status === "running") return "running";
  if (status === "failed" || status === "interrupted") return "failed";
  return "done";
}

/**
 * 全量分区 + 组内排序。
 * 组内序：waiting/running 按 updated_at 正序（等最久的排最前——先来先注意）；
 * failed/done 按 updated_at 倒序（最近的结果最相关）。
 * waitingQuestions: convId → 问题摘要（调用方从 chat store pendingAskRequests 投影）。
 */
export function buildBoard(
  tasks: readonly ProjectTask[],
  streamingIds: ReadonlySet<string>,
  waitingQuestions: ReadonlyMap<string, string>,
): BoardRow[] {
  const rows: BoardRow[] = tasks.map((task) => {
    const status = taskStatus(task, streamingIds);
    const waitingQuestion = waitingQuestions.get(task.conv_id) ?? null;
    return {
      task,
      status,
      section: sectionOf(status, waitingQuestion != null),
      waitingQuestion,
    };
  });
  const sectionOrder: Record<BoardSection, number> = {
    waiting: 0, running: 1, failed: 2, done: 3,
  };
  return rows.sort((a, b) => {
    const s = sectionOrder[a.section] - sectionOrder[b.section];
    if (s !== 0) return s;
    const asc = a.section === "waiting" || a.section === "running";
    const ta = Date.parse(a.task.updated_at) || 0;
    const tb = Date.parse(b.task.updated_at) || 0;
    return asc ? ta - tb : tb - ta;
  });
}
