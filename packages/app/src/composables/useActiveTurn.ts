// useActiveTurn — 轮次导航条的视位侦测（底线语义，2026-08-17 二轮重设计）
//
// 一根判定线：消息区底边上移 LINE_FROM_BOTTOM_PX（输入框正上方的横线）。
// 线落在哪个轮的区域里（区域 = [锚点i顶, 锚点i+1顶)），活动轮就是哪轮。
// 与事件无关：跳转/翻页/图片加载/流式追加一律经 IO 相交集重算，无逐事件补丁。
//
// 为何从顶线换底线（用户拍板）：顶线规则「最小相交轮过线→该轮，否则 -1」
// 两处系统性少报一——贴底跟随短轮显示 N-1（最高频投诉面孔）；居中阅读时
// 显示不在屏上的前一轮。底线语义贴底恒对（线恒在最新轮区域内），阅读位
// 显示占据下方阅读区的真实轮。固有限制：多轮同屏读屏顶轮时显示的是线下
// 轮，方向恒定（单线规则的数学下限）。
//
// 跳转钉子：点 tick N 的落点把锚 N 停在视口顶（阅读位），线却可能落在
// N+k 的区域——纯线判定会让跳转显得没到位。跳转 = 显式位置声明，钉住 N；
// 用户亲手滚动（滚轮/触摸/滚动键）即解钉，交还线判定。
//
// 坐标可信性根因不变（2026-08-15）：content-visibility:auto 估高布局下未
// 渲染组 offsetTop 系统性漂移——只对实际相交（= 已渲染，坐标真实）的锚点
// 读 gBCR 判定；集合空（线在长轮中部，前后锚点都不相交）保持上次值；
// 无上次值（切会话/锚点重载后）非贴底按滚动比例粗估 bootstrap。贴底
// （默认进入态）是确定性的——优先于一切集合/上次值直接末轮（见 recompute），
// detach 元素剔除防 DOM 换血竞态算出旧会话轮号。

import { computed, onBeforeUnmount, ref, watch, type Ref } from "vue";
import type { TurnAnchor } from "../types";

/** 跳转落点基准线（px，距视口顶）：锚点顶边停在 THRESHOLD_PX - 边际 的阅读位。
 *  与视位判定线（底线）已解耦：落点服务眼睛，号码由钉子保证。导出供
 *  ChatMessages.jumpToTurn 复用。 */
export const THRESHOLD_PX = 80;

/** 视位判定线（px，距消息区底边）：即输入框正上方一点。
 *  导出供测试与调用方对齐语义。 */
export const LINE_FROM_BOTTOM_PX = 24;

/** 贴底判定容差（px）：距内容底这一距离内视为贴底 */
const AT_BOTTOM_EPS = 4;

/** 纯函数：由「当前相交锚点的 (轮号, 距视口顶距离) + 判定线高度」判定活动轮。
 *  - 空列表 → null（视口落在长轮中部等，调用方保持上次值）
 *  - 线以上有锚点 → 线以上最大轮号（线落在其区域内）
 *  - 线以上无、线下有 → 线下最小轮 - 1（线在首锚上方，钳 ≥1） */
export function pickActiveTurn(intersecting: { turn: number; top: number }[], lineY: number): number | null {
  if (intersecting.length === 0) return null;
  let aboveMax = 0; // 线以上最大轮号（0 = 无）
  let belowMin = Infinity; // 线以下最小轮号
  for (const { turn, top } of intersecting) {
    if (top <= lineY) {
      if (turn > aboveMax) aboveMax = turn;
    } else if (turn < belowMin) {
      belowMin = turn;
    }
  }
  if (aboveMax > 0) return aboveMax;
  return Math.max(1, belowMin - 1);
}

/** 解钉的滚动键（滚轮/触摸之外，键盘滚动也是亲手滚动） */
const SCROLL_KEYS = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "]);

export function useActiveTurn(
  container: Ref<HTMLElement | null>,
  anchors: Ref<TurnAnchor[]>,
) {
  const activeTurn = ref<number | null>(null);

  /** 跳转钉住的轮号（null = 线判定接管） */
  const pinned = ref<number | null>(null);

  /** messageId → 轮号（1 起）。锚点重载（新轮）后轮号可能变化，
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
    if (pinned.value !== null) return; // 钉住期线判定静默（activeTurn 已由 pin 直写）
    const root = container.value;
    if (!root) return;
    const total = anchors.value.length;
    if (total === 0) return;
    // 贴底确定性（优先于一切集合/上次值）：贴底 = 内容底对齐视口底，线
    // （底上 24px）必然落在末轮区域内——直接末轮。治 restore 瞬移底部的
    // 竞态窗：消息渲染期 scrollTop=0 阶段顶部锚点先进集合、瞬移后 IO 迟到
    // 一拍，旧集合现读 gBCR 全在视口上方 → 线判定算出小轮号，随后集空
    // 「保持」把中毒值锁死。贴底不看集合，竞态无从产生。
    if (root.scrollTop + root.clientHeight >= root.scrollHeight - AT_BOTTOM_EPS) {
      activeTurn.value = total;
      return;
    }
    // DOM 换血竞态（切会话 → refresh 重建之间的窗口）：旧会话元素已移出
    // 容器，gBCR 全 0 会被当作「线以上」算出旧轮号——剔除（root.contains
    // 而非 isConnected：判「是否还属于本容器」，与挂没挂 document 无关）
    for (const el of intersecting.keys()) {
      if (!root.contains(el)) intersecting.delete(el);
    }
    if (intersecting.size === 0) {
      bootstrapIfUnknown(root);
      return;
    }
    const rootTop = root.getBoundingClientRect().top;
    const lineY = root.clientHeight - LINE_FROM_BOTTOM_PX;
    const visible = Array.from(intersecting, ([el, turn]) => ({
      turn,
      top: el.getBoundingClientRect().top - rootTop,
    }));
    const picked = pickActiveTurn(visible, lineY);
    if (picked !== null) activeTurn.value = picked; // null = 保持上次值
  }

  /** 空集兜底：无上次值（切会话/锚点重载后）时按滚动比例粗估初值。
   *  估高布局下足够 bootstrap（贴底已被确定性分支精确接管），任何锚点一
   *  进视口即被 IO 精确接管。有上次值不动——长轮中段滚动保持语义。 */
  function bootstrapIfUnknown(root: HTMLElement) {
    if (activeTurn.value !== null) return;
    if (root.scrollHeight <= 0) return; // 布局不可得（测试环境）
    const total = anchors.value.length;
    if (total === 0) return;
    const frac = (root.scrollTop + root.clientHeight) / root.scrollHeight;
    activeTurn.value = Math.min(total, Math.max(1, Math.ceil(frac * total)));
  }

  /** 跳转钉子：显式位置声明，立即生效（不等滚动/IO 事件） */
  function pin(turn: number) {
    pinned.value = turn;
    activeTurn.value = turn;
  }

  /** 解钉并立即回归线判定（不等下一次滚动） */
  function clearPin() {
    if (pinned.value === null) return;
    pinned.value = null;
    recompute();
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

  /** 亲手滚动意图（滚轮/触摸/滚动键）→ 解钉交还线判定 */
  function onUserScrollIntent() {
    if (pinned.value !== null) clearPin();
  }
  function onKeyDown(e: KeyboardEvent) {
    if (SCROLL_KEYS.has(e.key)) onUserScrollIntent();
  }

  // 锚点列表重载（切会话/新轮开始）→ 钉子与上次值双双失效：轮号语义已换界，
  // 旧号不可信（切会话保留旧号 = 显示错号的来源之一）。失忆后由 IO/滚动
  // 重算，空集则 bootstrap 兜底——「保持」只服务同界内的长轮中段滚动
  watch(anchors, () => {
    pinned.value = null;
    activeTurn.value = null;
  });

  // 容器挂载/更换：重挂滚动监听 + 重建观察（root 变了 IO 必须重建）
  watch(container, (el, _, onCleanup) => {
    if (!el) return;
    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("wheel", onUserScrollIntent, { passive: true });
    el.addEventListener("touchmove", onUserScrollIntent, { passive: true });
    el.addEventListener("keydown", onKeyDown);
    refresh();
    onCleanup(() => {
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("wheel", onUserScrollIntent);
      el.removeEventListener("touchmove", onUserScrollIntent);
      el.removeEventListener("keydown", onKeyDown);
    });
  });

  onBeforeUnmount(() => {
    observer?.disconnect();
    observer = null;
  });

  return { activeTurn, turnOfMsg, refresh, pin, clearPin };
}
