// useProjectTrajectory — 项目轴轨迹数据源（MA-2）：useTrajectory 的跨会话镜像。
//
// 结构与 useTrajectory 一一对应（尾部优先分页 / 增量拼接 / 并发防御同款），
// 差异只有三点：
// 1. 游标 = session_events 全局 id（AUTOINCREMENT，跨会话可比；seq 是 per-conv
//    的，跨会话不能当游标——见 migration 44 设计时预留的「项目级轨迹序」）
// 2. 无 legacy / turnOffset 概念：项目轴没有「事件纪元前的旧会话」（事件按
//    会话各自判定，无事件会话在此视图不出现）；轮号恒从窗口首桶起（跨会话
//    无全局轮序，轮号弱化为段序号，调用方注释写明）
// 3. live 过滤带「项目会话集」：事件通知按 conversation_id 过滤，本 composable
//    不知道项目会话集（chat store 才知道），由调用方传 isProjectConv 谓词
import { ref, onMounted, onBeforeUnmount, toValue } from "vue";
import type { MaybeRefOrGetter } from "vue";
import { listen } from "@tauri-apps/api/event";
import { bridge } from "../api/bridge";
import type { ProjectEvent, SessionEvent } from "../types";

/** 与 useTrajectory 同款页宽（项目轴事件量 ≥ 单会话，同量级防御） */
export const PROJECT_TRAJECTORY_PAGE_SIZE = 1000;

/**
 * scopeTurnKeys — buildRows 的跨会话前置适配（纯函数）。
 *
 * buildRows 的 turn 分桶是顺序扫描 + turn_id 变化即切组：跨会话合并流里
 * 不同会话常有相同 turn_id（如 "turn-3"）→ 错误合桶（统计互相污染、supersede
 * 误归并）。调用前把 turn_id 加 session 前缀，桶键天然隔离；null 保持 null
 * （各会话的纪元前孤儿事件本就无轮归属，共用孤儿桶可接受）。
 */
export function scopeTurnKeys<T extends SessionEvent>(events: T[]): T[] {
  return events.map((e) => {
    if (e.turn_id == null) return e; // null 保持（孤儿桶语义不变）
    // 引用稳定化：同一源事件恒得同一 scoped 对象——buildRows 的 searchText
    // WeakMap 按对象引用缓存，增量拼接时旧行引用复用、缓存不失效
    const hit = scopeCache.get(e);
    if (hit) return hit as T;
    const scoped = { ...e, turn_id: `${e.session_id}::${e.turn_id}` } as T;
    scopeCache.set(e as SessionEvent, scoped);
    return scoped;
  });
}
const scopeCache = new WeakMap<SessionEvent, SessionEvent>();

export function useProjectTrajectory(
  projectId: MaybeRefOrGetter<string>,
  /** 会话是否属于本项目（live 通知过滤用；chat store 的 project_id 缓存） */
  isProjectConv: (convId: string) => boolean,
) {
  const events = ref<ProjectEvent[]>([]);
  const loading = ref(false);
  const loadingEarlier = ref(false);
  const error = ref<string | null>(null);
  const hasMore = ref(false);

  let currentId: string | null = null;
  let minId: number | null = null;

  async function load() {
    const pid = toValue(projectId);
    if (!pid) return;
    currentId = pid;
    loading.value = true;
    error.value = null;
    try {
      const page = await bridge.projects.listEvents(pid, { limit: PROJECT_TRAJECTORY_PAGE_SIZE });
      if (currentId !== pid) return; // 切换项目竞态守卫
      events.value = page;
      hasMore.value = page.length === PROJECT_TRAJECTORY_PAGE_SIZE;
      minId = page.length ? page[0].id : null;
    } catch (e) {
      if (currentId !== pid) return;
      error.value = e instanceof Error ? e.message : String(e);
      events.value = [];
      hasMore.value = false;
    } finally {
      if (currentId === pid) loading.value = false;
    }
  }

  /**
   * live 追加：拉全局 id > 已载最大 id 的项目增量并原地拼接。
   * 返回新事件数（0 = 已追平）。与 useTrajectory.refreshLatest 同款并发防御：
   * 拼接时以**当前**尾部 id 再过滤一次——并发进入的两拨同源增量，后落者清零。
   */
  async function refreshLatest(): Promise<number> {
    const pid = currentId;
    if (!pid || loading.value) return 0;
    const maxId = events.value.length ? events.value[events.value.length - 1].id : 0;
    try {
      const inc = await bridge.projects.listEvents(pid, { limit: PROJECT_TRAJECTORY_PAGE_SIZE, afterId: maxId });
      if (currentId !== pid) return 0; // 切换项目竞态守卫
      if (!inc.length) return 0;
      const tail = events.value.length ? events.value[events.value.length - 1].id : 0;
      const fresh = inc.filter((e) => e.id > tail);
      if (fresh.length) events.value = [...events.value, ...fresh];
      return fresh.length;
    } catch {
      return 0; // 增量失败静默：下次通知/补拉再试（与单会话轨迹同款宽容）
    }
  }

  /** 「加载更早」：以当前已载最小全局 id 为游标向前翻一页 */
  async function loadEarlier() {
    const pid = currentId;
    if (!pid || minId == null || loadingEarlier.value || !hasMore.value) return;
    loadingEarlier.value = true;
    try {
      const page = await bridge.projects.listEvents(pid, { limit: PROJECT_TRAJECTORY_PAGE_SIZE, beforeId: minId });
      if (currentId === pid && page.length) {
        minId = page[0].id;
        events.value = [...page, ...events.value];
      }
      hasMore.value = page.length === PROJECT_TRAJECTORY_PAGE_SIZE;
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e);
    } finally {
      loadingEarlier.value = false;
    }
  }

  // ---- live 更新（D6：事件驱动，不做常驻轮询；onActivated 补拉由页面层调） ----
  // 会话级过滤 = 已载会话集 ∪ 项目会话集（谓词）。新委派子会话两边都不在 →
  // chat:delegation-started 无条件补一轮（payload 无项目字段，宁多拉不漏拉——
  // after_id 查询本身项目域过滤，多拉只是幂等空返）。
  const loadedConvIds = () => new Set(events.value.map((e) => e.session_id));
  async function onEventAppended(payload: { conversation_id: string; kind: string }) {
    if (!loadedConvIds().has(payload.conversation_id) && !isProjectConv(payload.conversation_id)) return;
    await refreshLatest();
  }

  const unlisteners: Array<() => void> = [];
  onMounted(async () => {
    // 初始 load 由视图层驱动（首载后要贴底，与 useTrajectory/TrajectoryView 同分工）
    unlisteners.push(
      await listen<{ conversation_id: string; kind: string }>("session:event-appended", (e) => {
        void onEventAppended(e.payload);
      }),
    );
    unlisteners.push(await listen("chat:delegation-started", () => { void refreshLatest(); }));
  });
  onBeforeUnmount(() => {
    unlisteners.forEach((u) => u());
    unlisteners.length = 0;
  });

  return { events, loading, loadingEarlier, error, hasMore, load, loadEarlier, refreshLatest };
}
