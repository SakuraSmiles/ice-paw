// earlierPagination — 「加载更早」分页三件套工厂（U3-5 ② 分页复制收敛）。
//
// useTrajectory 与 useProjectTrajectory 的 loadEarlier 曾逐行复制——仅游标字段
// （单会话 seq vs 项目轴全局 id）与拉取函数之别，其余完全同构：loadingEarlier /
// hasMore 双闸 + 首屏游标回填 + 更早页前置拼接 + 满页启发式。收敛到本工厂，
// 防「改一处漏一处」漂移（两处都曾在竞态守卫/游标更新上各自修过 bug）。
//
// 工厂只持有「更早分页」相关的响应式状态（loadingEarlier / hasMore / error /
// minCursor）；首屏 load 与 live 增量仍留在各自 composable（拉取入口与额外状态
// 差异足够大，不强行合并）。
import { ref, type Ref } from "vue";

export interface EarlierPaginationOptions<T> {
  /** 当前会话/项目锚（null = 未载入）。loadEarlier 起始捕获一次 id，
   *  跨 await 后用于竞态守卫——A→B 切换的旧页不得前插进新视图。 */
  currentId: () => string | null;
  /** 每页事件数（满页 = 可能还有更早，启发式） */
  pageSize: number;
  /** 拉取 beforeCursor 之前的一页（正序返回，页首即更早边界） */
  fetch: (currentId: string, beforeCursor: number) => Promise<T[]>;
  /** 取某条事件的游标值（单会话 seq / 项目轴全局 id） */
  cursorOf: (item: T) => number;
  /** 拉回一页后的副作用（如轨迹页重查全局轮偏移），仅在 hasMore 更新后调用 */
  onPage?: () => void;
}

export interface EarlierPagination<T> {
  loadingEarlier: Ref<boolean>;
  hasMore: Ref<boolean>;
  error: Ref<string | null>;
  /** 更早游标（null = 已到顶）；首屏 load 后由 markFirstPage 回填 */
  minCursor: Ref<number | null>;
  /** 首屏载入后回填游标 + 满页启发式（load 里调用） */
  markFirstPage: (page: T[]) => void;
  loadEarlier: (events: Ref<T[]>) => Promise<void>;
}

export function createEarlierPagination<T>(
  opts: EarlierPaginationOptions<T>,
): EarlierPagination<T> {
  const loadingEarlier = ref(false);
  const hasMore = ref(false);
  const error = ref<string | null>(null);
  const minCursor = ref<number | null>(null);

  function markFirstPage(page: T[]) {
    minCursor.value = page.length ? opts.cursorOf(page[0]) : null;
    hasMore.value = page.length === opts.pageSize;
  }

  async function loadEarlier(events: Ref<T[]>): Promise<void> {
    const id = opts.currentId();
    if (id == null || minCursor.value == null || loadingEarlier.value || !hasMore.value) return;
    loadingEarlier.value = true;
    try {
      const page = await opts.fetch(id, minCursor.value);
      // 竞态守卫：await 期间用户可能已切走——过期页丢弃。project 版恒有、
      // 单会话版原缺失（只判 currentId 非空），收敛时统一补齐。
      if (opts.currentId() === id && page.length) {
        minCursor.value = opts.cursorOf(page[0]);
        events.value = [...page, ...events.value];
      }
      hasMore.value = page.length === opts.pageSize;
      if (hasMore.value) opts.onPage?.();
    } catch (e) {
      error.value = e instanceof Error ? e.message : String(e);
    } finally {
      loadingEarlier.value = false;
    }
  }

  return { loadingEarlier, hasMore, error, minCursor, markFirstPage, loadEarlier };
}
