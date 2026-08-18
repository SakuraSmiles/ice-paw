// sessionRestore.ts — 启动恢复「上次会话与所在页面」：持久化 + 恢复决策纯函数。
//
// 持久化走 localStorage（key=icepaw-last-session，对齐 icepaw-theme / icepaw-panel-*
// 命名先例）：App.vue watch（路由 / 活跃会话 / 侧栏 scope 任一变化）即写——每次
// 导航落一次盘，崩溃/断电不丢，不依赖窗口关闭事件。
//
// 决策（planRestore，纯函数可测）：
// - 持久化会话仍存在、非后台子会话（kind 过滤）、所属项目未归档 → 恢复它，
//   scope 跟随其所属项目；
// - 持久化会话失效（被删 / 项目归档 / delegation 子会话）且并非明确欢迎态 →
//   回退「最近一条有效会话」（原打开软件行为）；
// - 上次明确停在欢迎态（convId=null）→ 保持欢迎态，不硬跳最近会话；
// - scope 指向已归档项目 → 降级散落；route 原样返回，非法路径由路由表通配
//   兜底回首页。

import { parseDbTime } from "./time";

const KEY = "icepaw-last-session";

export interface LastSessionState {
  /** 最后路由 fullPath（如 /projects/p1/settings；首页为 "/"） */
  route: string;
  /** 最后活跃会话 id；null = 上次停在欢迎态 */
  convId: string | null;
  /** 最后侧栏 scope 项目 id；null = 散落 */
  projectId: string | null;
}

export function saveLastSession(s: LastSessionState): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    // 容量满 / 隐私模式：放弃记忆，不影响运行
  }
}

export function loadLastSession(): LastSessionState | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    const v: unknown = JSON.parse(raw);
    if (typeof v !== "object" || v === null || typeof (v as LastSessionState).route !== "string") {
      return null;
    }
    return {
      route: (v as LastSessionState).route,
      convId: typeof (v as LastSessionState).convId === "string" ? (v as LastSessionState).convId : null,
      projectId:
        typeof (v as LastSessionState).projectId === "string" ? (v as LastSessionState).projectId : null,
    };
  } catch {
    return null; // 坏 JSON 当无记忆
  }
}

export interface RestorePlan {
  /** 恢复的会话 id；null = 欢迎态 */
  convId: string | null;
  /** 恢复后的侧栏 scope 项目 id；null = 散落 */
  projectId: string | null;
  /** 需要主动跳转的路由；null = 留在首页（首页无需跳） */
  route: string | null;
}

/** 会话列表行的最小结构（Conversation 的结构子集，便于测试构造） */
export interface RestoreConvLike {
  id: string;
  project_id?: string | null;
  kind?: string;
  updated_at: string;
}

/**
 * 恢复决策。convs 传侧栏可见会话（调用方已过滤 delegation），这里再按
 * 「所属项目未归档」过滤一遍（activeProjectIds = 未归档项目 id 集）。
 */
export function planRestore(
  saved: LastSessionState | null,
  convs: RestoreConvLike[],
  activeProjectIds: ReadonlySet<string>,
): RestorePlan {
  const valid = (c: RestoreConvLike) => !c.project_id || activeProjectIds.has(c.project_id);

  // 1) 持久化会话仍有效 → 恢复，scope 跟随其项目
  if (saved?.convId) {
    const hit = convs.find((c) => c.id === saved.convId && valid(c));
    if (hit) {
      return {
        convId: hit.id,
        projectId: hit.project_id ?? null,
        route: restoreRoute(saved.route),
      };
    }
    // 失效 → 回退最近一条有效会话（与「打开软件」原行为一致）
    const latest = latestValid(convs, valid);
    if (latest) {
      return { convId: latest.id, projectId: latest.project_id ?? null, route: restoreRoute(saved.route) };
    }
    // 连回退都没有（零有效会话）→ 欢迎态 + scope 降级
    return { convId: null, projectId: scopeOrNull(saved.projectId, activeProjectIds), route: restoreRoute(saved.route) };
  }

  // 2) 上次明确欢迎态（有记忆但无会话）→ 尊重，不硬跳最近
  if (saved) {
    return { convId: null, projectId: scopeOrNull(saved.projectId, activeProjectIds), route: restoreRoute(saved.route) };
  }

  // 3) 无记忆（首启 / localStorage 清空）→ 原行为：最近一条有效会话
  const latest = latestValid(convs, valid);
  if (latest) {
    return { convId: latest.id, projectId: latest.project_id ?? null, route: null };
  }
  return { convId: null, projectId: null, route: null };
}

/** 最近一条有效会话（updated_at 最大；并列取后者）。
 *  时间解析复用 parseDbTime（DB 存 UTC 无时区标识，裸 Date.parse 会差时区） */
function latestValid(
  convs: RestoreConvLike[],
  valid: (c: RestoreConvLike) => boolean,
): RestoreConvLike | null {
  let best: RestoreConvLike | null = null;
  let bestTime = -Infinity;
  for (const c of convs) {
    if (!valid(c)) continue;
    const t = parseDbTime(c.updated_at).getTime();
    if (Number.isNaN(t)) continue;
    if (t >= bestTime) {
      best = c;
      bestTime = t;
    }
  }
  return best;
}

/** scope 指向已归档/已删项目时降级散落 */
function scopeOrNull(pid: string | null, activeProjectIds: ReadonlySet<string>): string | null {
  return pid !== null && activeProjectIds.has(pid) ? pid : null;
}

/** 首页无需跳转；其余原样返回（非法路径由路由表通配兜底回首页） */
function restoreRoute(route: string): string | null {
  return route === "/" ? null : route;
}
