// sessionRestore.test.ts — 启动恢复决策锁定：持久化会话有效→原位恢复 /
// 失效→回退最近 / 明确欢迎态→不硬跳 / scope 归档降级散落 / 无记忆→原行为 /
// 持久化坏 JSON 当无记忆。
import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  saveLastSession,
  loadLastSession,
  planRestore,
  type LastSessionState,
  type RestoreConvLike,
} from "../sessionRestore";

function conv(id: string, opts: Partial<RestoreConvLike> = {}): RestoreConvLike {
  return { id, project_id: null, kind: undefined, updated_at: "2026-08-18 00:00:00", ...opts };
}

/** DB 真实格式时间戳（"YYYY-MM-DD HH:MM:SS"，UTC 无时区标识——走 parseDbTime 补 Z 路径） */
const T = (s: number) => new Date(Date.UTC(2026, 7, 18, 0, 0, s)).toISOString().replace("T", " ").slice(0, 19);

describe("saveLastSession / loadLastSession", () => {
  beforeEach(() => localStorage.clear());

  it("往返持久化；null 字段保真", () => {
    saveLastSession({ route: "/projects/p1/settings", convId: null, projectId: "p1" });
    expect(loadLastSession()).toEqual({ route: "/projects/p1/settings", convId: null, projectId: "p1" });
  });

  it("坏 JSON / 缺 route / 无记录 → null（当无记忆）", () => {
    expect(loadLastSession()).toBeNull();
    localStorage.setItem("icepaw-last-session", "{oops");
    expect(loadLastSession()).toBeNull();
    localStorage.setItem("icepaw-last-session", JSON.stringify({ convId: "c1" }));
    expect(loadLastSession()).toBeNull();
  });
});

describe("planRestore 恢复决策", () => {
  const P = new Set(["p1", "p2"]);

  it("持久化会话有效 → 原位恢复，scope 跟随其项目", () => {
    const saved: LastSessionState = { route: "/", convId: "c2", projectId: "p1" };
    const convs = [conv("c1", { project_id: "p1", updated_at: T(30) }), conv("c2", { project_id: "p1", updated_at: T(10) })];
    expect(planRestore(saved, convs, P)).toEqual({ convId: "c2", projectId: "p1", route: null });
  });

  it("非首页路由原样返回；首页返回 null（无需跳转）", () => {
    const saved: LastSessionState = { route: "/projects/p1/timeline", convId: "c1", projectId: "p1" };
    const plan = planRestore(saved, [conv("c1")], P);
    expect(plan.route).toBe("/projects/p1/timeline");
  });

  it("持久化会话失效（被删）→ 回退最近一条有效会话", () => {
    const saved: LastSessionState = { route: "/", convId: "gone", projectId: "p1" };
    const convs = [
      conv("c1", { project_id: "p1", updated_at: T(10) }),
      conv("c2", { project_id: "p2", updated_at: T(30) }),
      conv("c3", { project_id: null, updated_at: T(20) }),
    ];
    expect(planRestore(saved, convs, P)).toEqual({ convId: "c2", projectId: "p2", route: null });
  });

  it("持久化会话指向归档项目 → 视为失效走回退链", () => {
    const saved: LastSessionState = { route: "/", convId: "c1", projectId: "p1" };
    const convs = [conv("c1", { project_id: "pArchived", updated_at: T(30) }), conv("c2", { updated_at: T(10) })];
    // pArchived 不在活跃集 → c1 无效 → 回退 c2（散落）
    expect(planRestore(saved, convs, new Set(["p1"]))).toEqual({ convId: "c2", projectId: null, route: null });
  });

  it("上次明确欢迎态（convId=null）→ 保持欢迎态，不硬跳最近", () => {
    const saved: LastSessionState = { route: "/", convId: null, projectId: "p1" };
    const convs = [conv("c1", { project_id: "p1", updated_at: T(30) })];
    expect(planRestore(saved, convs, P)).toEqual({ convId: null, projectId: "p1", route: null });
  });

  it("会话全失效且无回退 → 欢迎态 + scope 归档降级散落", () => {
    const saved: LastSessionState = { route: "/projects/pArchived/settings", convId: "gone", projectId: "pArchived" };
    const convs: RestoreConvLike[] = [];
    expect(planRestore(saved, convs, P)).toEqual({ convId: null, projectId: null, route: "/projects/pArchived/settings" });
  });

  it("无记忆（首启）→ 原行为：最近一条有效会话，不跳路由", () => {
    const convs = [
      conv("c1", { project_id: "p1", updated_at: T(10) }),
      conv("c2", { project_id: "p2", updated_at: T(30) }),
    ];
    expect(planRestore(null, convs, P)).toEqual({ convId: "c2", projectId: "p2", route: null });
  });

  it("updated_at 并列取后者（列表序稳定时确定性恢复）", () => {
    const convs = [conv("c1", { updated_at: T(10) }), conv("c2", { updated_at: T(10) })];
    expect(planRestore(null, convs, P)?.convId).toBe("c2");
  });
});

// localStorage 不存在时 save 不抛（隐私模式兜底）
it("saveLastSession 在 storage 抛错时静默", () => {
  const spy = vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
    throw new Error("quota");
  });
  expect(() => saveLastSession({ route: "/", convId: null, projectId: null })).not.toThrow();
  spy.mockRestore();
});
