// useActiveTurn — 轮次导航条的视位侦测（治本替换 topsCache 静态 offsetTop 扫描）
//
// 根因（2026-08-15 生产诊断）：消息列表启用 content-visibility:auto +
// contain-intrinsic-size（千轮渲染虚拟化，H1）后，未渲染组按**估高**参与
// 布局，静态缓存的 offsetTop 与真实位置系统性漂移——导航条卡死在旧轮、
// 切会话错位、拉到最底才对上，都是同一根因的投影。
//
// 治本 = 只对**实际相交（= 已渲染，坐标真实）**的锚点元素做判定：
// - IntersectionObserver 维护「当前相交集合」（成员真相，IO 只报真实渲染区）
// - scroll（rAF 合帧）在集合内做边界判定：相交的最小轮 t，
//   其元素顶边过「视口顶 +80px」判定线 → active = t，否则 t-1（正在读上一轮尾部）
// - 集合空（视口落在长轮中部，前后锚点都不相交）→ 保持上次值
//
// 判定线语义与旧实现一致（scrollTop+80 的视口顶基准）。

import { computed, onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { TurnAnchor } from "../types";

/** 视口顶判定线（px）：锚点顶边越过此线即视为「正在读这一轮」 */
const THRESHOLD_PX = 80;

/** 纯函数：由「当前相交锚点的 (轮号, 距视口顶距离)」判定活动轮。
 *  - 空列表 → null（视口落在长轮中部等，调用方保持上次值）
 *  - 最小轮的锚点顶边已过判定线 → 该轮（正在读它的开头）
 *  - 否则 → 最小轮 - 1（视口还在读上一轮的尾部内容） */
export function pickActiveTurn(intersecting: { turn: number; top: number }[]): number | null {
  if (intersecting.length === 0) return null;
  let minTurn = Infinity;
  let minTop = Infinity;
  for (const { turn, top } of intersecting) {
    if (turn < minTurn) {
      minTurn = turn;
      minTop = top;
    }
  }
  return minTop <= THRESHOLD_PX ? minTurn : Math.max(1, minTurn - 1);
}

export function useActiveTurn(
  container: Ref<HTMLElement | null>,
  anchors: Ref<TurnAnchor[]>,
) {
  const activeTurn = ref<number | null>(null);

  /** messageId → 轮号（1 起）。锚点重载（翻页/新轮）后轮号可能整体平移，
   *  相交集合的成员→轮号映射在 IO 回调时实时查此表，永不吃旧值。 */
  const turnOfMsg = computed(() => {
    const m = new Map<string, number>();
    anchors.value.forEach((a, i) => m.set(a.message_id, i + 1));
    return m;
  });

  /** 当前相交中的锚点元素集合（IO 维护，scroll 只读） */
  const intersecting = new Map<Element, number>();
  let observer: IntersectionObserver | null = null;
  let rafPending = false;

  function turnOfEl(el: Element): number | null {
    const mid = (el as HTMLElement).dataset.mid;
    if (!mid) return null;
    return turnOfMsg.value.get(mid) ?? null;
  }

  /** 边界判定：只在「真实相交」的元素上读 gBCR——已渲染，坐标可信 */
  function recompute() {
    const root = container.value;
    if (!root || intersecting.size === 0) return;
    const rootTop = root.getBoundingClientRect().top;
    const visible = Array.from(intersecting, ([el, turn]) => ({
      turn,
      top: el.getBoundingClientRect().top - rootTop,
    }));
    const picked = pickActiveTurn(visible);
    if (picked !== null) activeTurn.value = picked; // null = 保持上次值
  }

  function onIoEntries(entries: IntersectionObserverEntry[]) {
    for (const e of entries) {
      const turn = turnOfEl(e.target);
      if (turn === null) continue;
      if (e.isIntersecting) intersecting.set(e.target, turn);
      else intersecting.delete(e.target);
    }
    recompute();
  }

  /** 重建观察：翻页/新轮/锚点重载后调用（DOM 变更后 nextTick） */
  function refresh() {
    const root = container.value;
    if (!root) return;
    intersecting.clear();
    observer?.disconnect();
    if (typeof IntersectionObserver === "undefined") return; // jsdom 等无 IO 环境
    observer = new IntersectionObserver(onIoEntries, { root });
    const map = turnOfMsg.value;
    for (const el of root.querySelectorAll<HTMLElement>("[data-mid]")) {
      if (map.has(el.dataset.mid ?? "")) observer.observe(el);
    }
    // observe 的初始状态由 IO 异步回调送达，无需手动补算
  }

  /** 滚动仅影响边界判定（元素进出视口由 IO 负责）；rAF 合帧 */
  function onScroll() {
    if (rafPending) return;
    rafPending = true;
    requestAnimationFrame(() => {
      rafPending = false;
      recompute();
    });
  }

  // 容器挂载/更换：重挂滚动监听 + 重建观察（root 变了 IO 必须重建）
  watch(container, (el, _, onCleanup) => {
    if (!el) return;
    el.addEventListener("scroll", onScroll, { passive: true });
    refresh();
    onCleanup(() => el.removeEventListener("scroll", onScroll));
  });

  onBeforeUnmount(() => {
    observer?.disconnect();
    observer = null;
  });

  return { activeTurn, turnOfMsg, refresh };
}
