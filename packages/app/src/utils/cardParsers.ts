// cardParsers — 工具卡片取数层的纯解析器（2026-10-30 从 ChatMessages.vue 抽出）。
//
// 抽出动机：委派卡（delegate_to_agent）/ 计划卡（update_plan）的「参数/结果 →
// 卡片数据」解析是纯函数，与组件状态（chat/findToolResult/runningDelegationChildId）
// 无耦合——抽到 utils 供 DelegationCard/PlanCard 数据层与测试复用，也为
// ChatMessages.vue 瘦身。流式期间参数逐字到达，解析失败统一返回 null（非异常）。

import type { PlanItem } from "../types";

/** 解析委派工具参数（agent_id/task）。流式期间参数逐字到达，解析失败返回 null。 */
export function parseDelegateInput(input: string): { agentId: string; task: string } | null {
  try {
    const o = JSON.parse(input) as { agent_id?: unknown; task?: unknown };
    if (o && typeof o.agent_id === "string") {
      return { agentId: o.agent_id, task: typeof o.task === "string" ? o.task : "" };
    }
  } catch { /* 参数未接收完/非 JSON */ }
  return null;
}

/** 解析委派 tool_result（child_conversation_id/agent_name/finish_reason/rounds）。
 *  Err 路径的 content 是错误文案非 JSON → null（状态走 isError）。 */
export function parseDelegateResult(content: string): {
  childConvId: string | null; agentName: string | null;
  finishReason: string | null; rounds: number | null;
} | null {
  try {
    const o = JSON.parse(content) as Record<string, unknown>;
    if (o && (typeof o.child_conversation_id === "string" || typeof o.agent_name === "string")) {
      return {
        childConvId: typeof o.child_conversation_id === "string" ? o.child_conversation_id : null,
        agentName: typeof o.agent_name === "string" ? o.agent_name : null,
        finishReason: typeof o.finish_reason === "string" ? o.finish_reason : null,
        rounds: typeof o.rounds === "number" ? o.rounds : null,
      };
    }
  } catch { /* 非委派 JSON */ }
  return null;
}

/** 解析 spawn_worker 参数（purpose/task——批 P1-5：worker 卡取数）。
 *  流式期间参数逐字到达，解析失败统一 null（同 parseDelegateInput 约定）。 */
export function parseSpawnWorkerInput(input: string): { purpose: string; task: string } | null {
  try {
    const o = JSON.parse(input) as { purpose?: unknown; task?: unknown };
    if (o && typeof o.purpose === "string") {
      return { purpose: o.purpose, task: typeof o.task === "string" ? o.task : "" };
    }
  } catch { /* 参数未接收完/非 JSON */ }
  return null;
}

/** 解析 spawn_worker tool_result（worker_conversation_id/worker_name/finish_reason/rounds）。
 *  字段族与 delegate 不同名（worker_conversation_id / worker_name），故独立解析器。 */
export function parseSpawnWorkerResult(content: string): {
  childConvId: string | null; workerName: string | null;
  finishReason: string | null; rounds: number | null;
} | null {
  try {
    const o = JSON.parse(content) as Record<string, unknown>;
    if (o && (typeof o.worker_conversation_id === "string" || typeof o.worker_name === "string")) {
      return {
        childConvId: typeof o.worker_conversation_id === "string" ? o.worker_conversation_id : null,
        workerName: typeof o.worker_name === "string" ? o.worker_name : null,
        finishReason: typeof o.finish_reason === "string" ? o.finish_reason : null,
        rounds: typeof o.rounds === "number" ? o.rounds : null,
      };
    }
  } catch { /* 非 worker JSON */ }
  return null;
}

/** 解析计划工具参数（steps → PlanItem[]）。steps 缺失/非数组 → null（流式参数
 *  未到齐）；steps: [] 合法（agent 主动清空计划）。字段宽容降级，不为展示卡死。 */
export function parsePlanInput(input: string): PlanItem[] | null {
  try {
    const o = JSON.parse(input) as { steps?: unknown };
    if (o && Array.isArray(o.steps)) {
      return o.steps
        .filter((s): s is Record<string, unknown> => typeof s === "object" && s !== null)
        .map((s) => ({
          text: typeof s.text === "string" ? s.text : "",
          status: typeof s.status === "string" ? s.status : "pending",
          task_conversation_id:
            typeof s.task_conversation_id === "string" ? s.task_conversation_id : null,
        }));
    }
  } catch { /* 参数未接收完/非 JSON */ }
  return null;
}
