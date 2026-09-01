// useProjectTrajectory.test.ts — 项目轴轨迹数据源锁定：全局 id 游标分页边界 /
// 增量拼接 + 并发防御（同源增量不双拼）/ live 过滤（已载会话集 ∪ 项目会话集，
// delegation-started 宁多拉）/ live 去抖 300ms 合并 / keep-alive 停用暂停·激活
// 恢复（路由级缓存监听器泄漏修复）/ scopeTurnKeys 跨会话桶键隔离。
import { describe, it, expect, beforeEach, vi } from "vitest";
import { defineComponent, h, KeepAlive, nextTick, ref } from "vue";
import { mount, flushPromises } from "@vue/test-utils";
import { listen } from "@tauri-apps/api/event";
import { PROJECT_TRAJECTORY_PAGE_SIZE, scopeTurnKeys, useProjectTrajectory } from "../useProjectTrajectory";
import { bridge } from "../../api/bridge";
import type { ProjectEvent } from "../../types";

vi.mock("../../api/bridge", () => ({
  bridge: { projects: { listEvents: vi.fn() } },
}));

const mockListEvents = vi.mocked(bridge.projects.listEvents);
const mockListen = vi.mocked(listen);

function captureHandlers() {
  const handlers = new Map<string, (event: { payload: unknown }) => void>();
  mockListen.mockImplementation(async (event, handler) => {
    handlers.set(event, handler as (event: { payload: unknown }) => void);
    return () => { handlers.delete(event); };
  });
  return handlers;
}

let gid = 0;
/** 项目事件构造器：全局 id 自增（跨会话序），seq 独立给（per-conv 可重复） */
function pev(session: string, seq: number, opts: { turnId?: string | null } = {}): ProjectEvent {
  gid += 1;
  return {
    id: gid,
    session_id: session,
    seq,
    kind: "user_message",
    actor: "user",
    turn_id: opts.turnId === undefined ? "t1" : opts.turnId,
    message_id: null,
    payload: { content: `ev-${gid}`, blocks: [] },
    created_at: `2026-08-18T10:00:${String(gid % 60).padStart(2, "0")}Z`,
    session_title: `会话-${session}`,
    session_kind: session === "child" ? "delegation" : "chat",
  };
}

/** 宿主组件工厂（composable 的 onMounted/onBeforeUnmount/keep-alive 钩子需组件
 *  上下文）。全文件唯一 defineComponent——vue/one-component-per-file。 */
function makeHost(
  pid: () => string,
  isProjectConv: (id: string) => boolean,
  onApi: (api: ReturnType<typeof useProjectTrajectory>) => void,
) {
  return defineComponent({
    setup() {
      onApi(useProjectTrajectory(pid, isProjectConv));
      return () => h("div");
    },
  });
}

/** 直接挂载宿主并同步取回 api */
function mountHost(pid = () => "p1", isProjectConv: (id: string) => boolean = () => false) {
  let api!: ReturnType<typeof useProjectTrajectory>;
  mount(makeHost(pid, isProjectConv, (a) => { api = a; }));
  return api;
}

describe("useProjectTrajectory 项目轴数据源", () => {
  beforeEach(() => {
    gid = 0;
    // 默认空返（增量拉取的常态）；具体用例用 mockResolvedValueOnce 覆盖
    mockListEvents.mockReset().mockResolvedValue([]);
  });

  it("满页 → hasMore；「加载更早」以最小全局 id 为游标前置拼接", async () => {
    const api = mountHost();
    const tail = Array.from({ length: PROJECT_TRAJECTORY_PAGE_SIZE }, () => pev("s1", 1));
    const earlier = [pev("s1", 0), pev("s2", 0)];
    mockListEvents
      .mockResolvedValueOnce(tail)
      .mockResolvedValueOnce(earlier);

    await api.load();
    expect(api.hasMore.value).toBe(true);
    expect(api.events.value).toHaveLength(PROJECT_TRAJECTORY_PAGE_SIZE);

    await api.loadEarlier();
    // 游标 = 首载窗口的最小全局 id（tail[0].id = 1）
    expect(mockListEvents).toHaveBeenLastCalledWith("p1", { limit: PROJECT_TRAJECTORY_PAGE_SIZE, beforeId: tail[0].id });
    expect(api.events.value.slice(0, 2)).toEqual(earlier);
    expect(api.hasMore.value).toBe(false); // 不足一页
  });

  it("不满页 → 无更早；增量 afterId = 已载最大全局 id", async () => {
    const api = mountHost();
    const e1 = pev("s1", 1);
    const e2 = pev("s2", 5); // 跨会话交错：全局 id 才是权威序
    mockListEvents.mockResolvedValueOnce([e1, e2]);
    await api.load();
    expect(api.hasMore.value).toBe(false);

    const e3 = pev("child", 9);
    mockListEvents.mockResolvedValueOnce([e3]);
    await api.refreshLatest();
    expect(mockListEvents).toHaveBeenLastCalledWith("p1", { limit: PROJECT_TRAJECTORY_PAGE_SIZE, afterId: e2.id });
    expect(api.events.value.map((e) => e.id)).toEqual([e1.id, e2.id, e3.id]);
  });

  it("并发 refreshLatest 不重复拼接（push 通知与补拉竞态回归，与 useTrajectory 同款）", async () => {
    const api = mountHost();
    mockListEvents.mockResolvedValueOnce([pev("s1", 1), pev("s1", 2)]);
    await api.load();

    const e3 = pev("s1", 3);
    mockListEvents.mockImplementation(async () => [e3]);
    const [a, b] = await Promise.all([api.refreshLatest(), api.refreshLatest()]);
    expect(a + b).toBe(1);
    expect(api.events.value.filter((e) => e.id === e3.id)).toHaveLength(1);
  });

  it("live 过滤：已载会话集 ∪ 项目会话集命中才拉增量，其他项目零动作", async () => {
    vi.useFakeTimers();
    try {
      const handlers = captureHandlers();
      const api = mountHost(() => "p1", (id) => id === "proj-conv");
      await flushPromises();

      const inLoaded = pev("s1", 1);
      mockListEvents.mockResolvedValueOnce([inLoaded]);
      await api.load();
      mockListEvents.mockClear();

      // 噪声：他项目会话（不在已载集，谓词也不认）——去抖窗口到期后也零动作
      handlers.get("session:event-appended")!({ payload: { kind: "assistant_message", conversation_id: "other-project" } });
      await vi.advanceTimersByTimeAsync(300);
      expect(mockListEvents).not.toHaveBeenCalled();

      // 命中：已载会话集
      handlers.get("session:event-appended")!({ payload: { kind: "tool_execution", conversation_id: "s1" } });
      await vi.advanceTimersByTimeAsync(300);
      expect(mockListEvents).toHaveBeenCalledTimes(1);

      // 命中：项目会话集（事件未载但谓词认识——新委派子会话首事件）
      handlers.get("session:event-appended")!({ payload: { kind: "turn_context", conversation_id: "proj-conv" } });
      await vi.advanceTimersByTimeAsync(300);
      expect(mockListEvents).toHaveBeenCalledTimes(2);

      // 命中：delegation-started 无项目字段，无条件补拉（宁多拉不漏拉）
      mockListEvents.mockClear();
      handlers.get("chat:delegation-started")!({ payload: { conversation_id: "别处" } });
      await vi.advanceTimersByTimeAsync(300);
      expect(mockListEvents).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });

  it("live 去抖：密集通知按 300ms 首呼锚定合并（5 连发 → 两拨拉取，非 5 拨）", async () => {
    vi.useFakeTimers();
    try {
      const handlers = captureHandlers();
      const api = mountHost();
      await flushPromises();
      mockListEvents.mockResolvedValueOnce([pev("s1", 1)]);
      await api.load();
      mockListEvents.mockClear();

      for (let i = 0; i < 5; i++) {
        handlers.get("session:event-appended")!({ payload: { kind: "tool_execution", conversation_id: "s1" } });
        await vi.advanceTimersByTimeAsync(100); // 模拟生成期事件连发的节奏
      }
      // t=500：t=300 首拨已拉、t=600 次拨未到期 → 恰 1 次
      expect(mockListEvents).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(300);
      expect(mockListEvents).toHaveBeenCalledTimes(2);
    } finally {
      vi.useRealTimers();
    }
  });

  it("keep-alive 停用期间暂停 live 拉取，激活后恢复（路由级缓存监听器泄漏修复）", async () => {
    vi.useFakeTimers();
    try {
      const handlers = captureHandlers();
      let api!: ReturnType<typeof useProjectTrajectory>;
      const Host = makeHost(() => "p1", () => false, (a) => { api = a; });
      const on = ref(true);
      // 外层用函数式组件（文件内保持唯一 defineComponent）：KeepAlive 对
      // Host↔null 切换触发 onDeactivated/onActivated
      mount(() => h(KeepAlive, null, { default: () => (on.value ? h(Host) : null) }));
      await flushPromises();

      mockListEvents.mockResolvedValueOnce([pev("s1", 1)]);
      await api.load();
      mockListEvents.mockClear();

      // 离开项目详情页（keep-alive 缓存不卸载）：此后任何页面上落库的事件不再驱动本视图
      on.value = false;
      await nextTick();
      handlers.get("session:event-appended")!({ payload: { kind: "tool_execution", conversation_id: "s1" } });
      await vi.advanceTimersByTimeAsync(1000);
      expect(mockListEvents).not.toHaveBeenCalled();

      // 回到项目详情页：监听恢复（错过的增量由页面层 onActivated 补拉，本层不重复拉）
      on.value = true;
      await nextTick();
      handlers.get("session:event-appended")!({ payload: { kind: "tool_execution", conversation_id: "s1" } });
      await vi.advanceTimersByTimeAsync(300);
      expect(mockListEvents).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("scopeTurnKeys 跨会话桶键适配", () => {
  it("同 turn_id 跨会话不合桶：前缀隔离；null 保持；源事件不可变", () => {
    const a = pev("s1", 1, { turnId: "t9" });
    const b = pev("s2", 1, { turnId: "t9" });
    const orphan = pev("s1", 2, { turnId: null });
    const scoped = scopeTurnKeys([a, b, orphan]);

    expect(scoped[0].turn_id).toBe("s1::t9");
    expect(scoped[1].turn_id).toBe("s2::t9");
    expect(scoped[0].turn_id).not.toBe(scoped[1].turn_id);
    expect(scoped[2].turn_id).toBeNull(); // 孤儿桶语义保持
    expect(a.turn_id).toBe("t9"); // 源不可变（append-only 视图不改事实）
    expect(scoped[2]).toBe(orphan); // null 事件原引用直过（ WeakMap 缓存友好）
  });

  it("同源事件重复 scope 引用稳定（buildRows searchText WeakMap 缓存前提）", () => {
    const a = pev("s1", 1, { turnId: "t1" });
    const s1 = scopeTurnKeys([a]);
    const s2 = scopeTurnKeys([a]);
    expect(s1[0]).toBe(s2[0]);
  });
});
