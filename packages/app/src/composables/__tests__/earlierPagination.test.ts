// earlierPagination.test.ts — 「加载更早」分页三件套工厂单测（U3-5 ② 收敛）。
//
// 原先该行为只能经 useTrajectory / useProjectTrajectory 间接验证；抽出工厂后
// 直接断言，尤其锁住收敛时补上的严格竞态守卫（单会话版原缺失——A→B 切换后
// 旧页会前插进新会话）。
import { describe, it, expect, vi } from "vitest";
import { ref } from "vue";
import { createEarlierPagination } from "../earlierPagination";

type Item = { id: number };

function makePagination(opts?: Partial<Parameters<typeof createEarlierPagination<Item>>[0]>) {
  return createEarlierPagination<Item>({
    currentId: () => "c1",
    pageSize: 2,
    fetch: vi.fn(),
    cursorOf: (e) => e.id,
    ...opts,
  });
}

describe("createEarlierPagination 分页三件套", () => {
  it("markFirstPage：满页 → hasMore + 游标=页首；不满页 → 无更早；空页 → 游标空", () => {
    const p = makePagination();
    p.markFirstPage([{ id: 3 }, { id: 4 }]);
    expect(p.hasMore.value).toBe(true);
    expect(p.minCursor.value).toBe(3);

    p.markFirstPage([{ id: 9 }]);
    expect(p.hasMore.value).toBe(false);
    expect(p.minCursor.value).toBe(9);

    p.markFirstPage([]);
    expect(p.hasMore.value).toBe(false);
    expect(p.minCursor.value).toBeNull();
  });

  it("loadEarlier：更早页前置拼接 + 游标前移 + 满页启发式 + onPage 副作用", async () => {
    const fetch = vi.fn();
    const onPage = vi.fn();
    const p = makePagination({ fetch, onPage });
    const events = ref<Item[]>([{ id: 5 }, { id: 6 }]);
    p.markFirstPage([{ id: 5 }, { id: 6 }]); // minCursor=5, hasMore=true
    fetch.mockResolvedValueOnce([{ id: 3 }, { id: 4 }]);

    await p.loadEarlier(events);

    expect(fetch).toHaveBeenCalledWith("c1", 5);
    expect(events.value.map((e) => e.id)).toEqual([3, 4, 5, 6]);
    expect(p.minCursor.value).toBe(3);
    expect(p.hasMore.value).toBe(true); // 满页 → 可能还有更早
    expect(onPage).toHaveBeenCalledTimes(1); // 仅 hasMore 时触发
  });

  it("loadEarlier：跨 await 切换会话，过期页丢弃（收敛补上的严格守卫）", async () => {
    let resolve!: (v: Item[]) => void;
    let current = "c1";
    const p = makePagination({
      currentId: () => current,
      fetch: () => new Promise<Item[]>((res) => { resolve = res; }),
    });
    const events = ref<Item[]>([{ id: 5 }, { id: 6 }]);
    p.markFirstPage([{ id: 5 }, { id: 6 }]);

    const loading = p.loadEarlier(events);
    current = "c2"; // await 期间用户切走
    resolve([{ id: 3 }, { id: 4 }]);
    await loading;

    expect(events.value.map((e) => e.id)).toEqual([5, 6]); // 过期页不前插
    expect(p.minCursor.value).toBe(5); // 游标不动
  });

  it("loadEarlier：无游标/无更早/加载中三闸早退，不触发 fetch", async () => {
    const fetch = vi.fn();
    const p = makePagination({ fetch });
    const events = ref<Item[]>([]);
    await p.loadEarlier(events); // 无游标（未 markFirstPage）
    expect(fetch).not.toHaveBeenCalled();
  });
});
